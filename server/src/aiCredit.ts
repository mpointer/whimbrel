import type { AccountRecord, SqlExecutor, SqlRow } from "./accounts";

/** Whimbrel AI credit: a prepaid balance in micro-dollars on the account
 * row, holds placed before a request and settled after it, and an
 * append-only ledger. The ledger keeps model and token counts only, never
 * prompt or response text.
 *
 * Every money move is one `db.batch`, a single transaction: all of its
 * statements apply or none do. A batch only rolls back on an SQL error,
 * not on "matched no rows", so each later statement carries its own guard
 * on what the earlier one did.
 *
 * Ledger `micros` is the signed change: a purchase is positive, usage and
 * refunds are negative. A usage row records the real cost, even in the
 * rare case where less was taken because both buckets ran out.
 *
 * Plus accounts also have a monthly allowance (`ai_allowance_micros`),
 * spendable only while `ai_allowance_expires_ms` is in the future. Usage
 * draws the allowance first, then prepaid. A hold remembers how much of it
 * came from the allowance and which allowance window (its expiry) it drew
 * from, so the settle can charge each bucket its own share. */

/** A hold older than this is settled by the daily cron as if the request
 * that placed it had died: the hold amount is kept as the charge. */
export const STALE_HOLD_MS = 15 * 60 * 1000;

/** Throws unless `x` is a whole, non-negative number of micros (positive
 * when `positive`). Money inputs come from our own code and Stripe, so a bad
 * one is a bug, never something to clamp. */
function checkMicros(name: string, x: number, positive = false): void {
  if (!Number.isSafeInteger(x) || x < 0 || (positive && x === 0)) {
    throw new RangeError(`${name} must be a ${positive ? "positive" : "non-negative"} whole number of micros, got ${x}`);
  }
}

/** Debits round up to a hundredth of a cent (100 micros). This is the one
 * place that rule lives: hold amounts and costs both go through it. */
function roundUpDebit(x: number): number {
  return Math.ceil(x / 100) * 100;
}

export interface AiUsage {
  model: string;
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
}

/** The allowance used when `server_setting('plus_allowance_micros')` is
 * missing, not a whole number, or outside 1..MAX_ALLOWANCE_MICROS. */
export const DEFAULT_ALLOWANCE_MICROS = 6_000_000;
export const MAX_ALLOWANCE_MICROS = 50_000_000;

/** SQL for the account's spendable allowance at `?` (now): 0 once expired
 * or when there is none (NULL expiry). */
const LIVE_ALLOWANCE = "CASE WHEN ai_allowance_expires_ms > ? THEN ai_allowance_micros ELSE 0 END";

/** The prepaid balance only (the allowance is not included). */
export async function aiBalance(db: SqlExecutor, accountId: string): Promise<number> {
  const rows = await db.execute("SELECT ai_credit_micros FROM account WHERE id = ?", [accountId]);
  return rows.length ? Number(rows[0].ai_credit_micros) : 0;
}

/** The allowance the account can spend at `now`: 0 when expired or none.
 * Used only by tests (production reads `spendable` and the account row); kept
 * here so it shares LIVE_ALLOWANCE with the money path it checks. */
export async function spendableAllowance(db: SqlExecutor, accountId: string, now: number): Promise<number> {
  const rows = await db.execute(`SELECT ${LIVE_ALLOWANCE} AS allowance FROM account WHERE id = ?`, [now, accountId]);
  return rows.length ? Number(rows[0].allowance) : 0;
}

/** Allowance plus prepaid: what a request can spend at `now`. */
export async function spendable(db: SqlExecutor, accountId: string, now: number): Promise<number> {
  const rows = await db.execute(`SELECT ai_credit_micros + ${LIVE_ALLOWANCE} AS spendable FROM account WHERE id = ?`, [now, accountId]);
  return rows.length ? Number(rows[0].spendable) : 0;
}

/** The allowance fields of the balance view: the allowance is 0 once
 * expired; the expiry is the stored one, null only when there is none.
 * Clients must not display a past expiry as "renews on". After
 * zeroAllowance the expiry can be later than now while the allowance is 0;
 * clients must ignore the date whenever the allowance is 0. */
export function allowanceView(
  account: Pick<AccountRecord, "aiAllowanceMicros" | "aiAllowanceExpiresMs">,
  now: number,
): { allowanceMicros: number; allowanceExpiresMs: number | null } {
  const expires = account.aiAllowanceExpiresMs ?? null;
  return { allowanceMicros: expires !== null && expires > now ? account.aiAllowanceMicros : 0, allowanceExpiresMs: expires };
}

/** The end of month `n` of a subscription period that starts at
 * `periodStartUnixMs`: the start plus `n` calendar months in UTC, on the same
 * day of the month (clamped to the last day of a shorter month) at the same
 * time of day, and never later than `periodEndUnixMs`. A yearly Plus period
 * grants month by month with it (month 1 from the invoice, months 2 to 12
 * from the cron). */
export function monthWindowEnd(periodStartUnixMs: number, n: number, periodEndUnixMs: number): number {
  if (!Number.isSafeInteger(periodStartUnixMs) || !Number.isSafeInteger(n) || n < 0 || !Number.isSafeInteger(periodEndUnixMs)) {
    throw new RangeError(`monthWindowEnd needs whole numbers, got ${periodStartUnixMs}, ${n}, ${periodEndUnixMs}`);
  }
  return Math.min(addMonthsUtc(periodStartUnixMs, n), periodEndUnixMs);
}

/** `ms` moved by `months` calendar months (negative goes back) in UTC: the
 * same day of the month, clamped to the last day of a shorter month, at the
 * same time of day. monthWindowEnd's rule, without the end clamp. */
export function addMonthsUtc(ms: number, months: number): number {
  if (!Number.isSafeInteger(ms) || !Number.isSafeInteger(months)) {
    throw new RangeError(`addMonthsUtc needs whole numbers, got ${ms}, ${months}`);
  }
  const d = new Date(ms);
  const year = d.getUTCFullYear();
  const month = d.getUTCMonth() + months;
  const lastDay = new Date(Date.UTC(year, month + 1, 0)).getUTCDate();
  return Date.UTC(year, month, Math.min(d.getUTCDate(), lastDay), d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds(), d.getUTCMilliseconds());
}

/** The configured monthly allowance: `server_setting('plus_allowance_micros')`
 * when it is a whole number in 1..MAX_ALLOWANCE_MICROS (digits only, no
 * sign, exponent or fraction), else DEFAULT_ALLOWANCE_MICROS. A row that is
 * present but unusable (including a REAL such as 7500000.0, which the TEXT
 * column stores as "7500000.0") logs one warning line; a missing row is
 * the normal default and logs nothing. */
export async function readAllowanceSetting(db: SqlExecutor): Promise<number> {
  const rows = await db.execute("SELECT value FROM server_setting WHERE key = 'plus_allowance_micros'", []);
  return wholeSetting("plus_allowance_micros", rows, MAX_ALLOWANCE_MICROS, DEFAULT_ALLOWANCE_MICROS);
}

/** A server_setting row's value when it is a whole number in 1..max (digits
 * only), else `fallback`; a present but unusable row logs one warning line,
 * a missing row logs nothing. */
function wholeSetting(key: string, rows: SqlRow[], max: number, fallback: number): number {
  if (!rows.length) return fallback;
  const text = String(rows[0].value).trim();
  const value = /^[0-9]{1,9}$/.test(text) ? Number(text) : NaN;
  if (value >= 1 && value <= max) return value;
  console.warn(`server_setting ${key} ${JSON.stringify(String(rows[0].value))} is not a whole number in 1..${max}; using ${fallback}`);
  return fallback;
}

/** The yearly cron's cap on grants per run when
 * `server_setting('plus_yearly_grants_per_run')` is missing or unusable. */
export const DEFAULT_YEARLY_GRANTS_PER_RUN = 10;
export const MAX_YEARLY_GRANTS_PER_RUN = 200;

/** The yearly cron's cap on grants per run:
 * `server_setting('plus_yearly_grants_per_run')` when it is a whole number in
 * 1..MAX_YEARLY_GRANTS_PER_RUN, else DEFAULT_YEARLY_GRANTS_PER_RUN (warning
 * once, as readAllowanceSetting does, for a present but unusable row).
 *
 * Subrequests: through @tursodatabase/serverless each `execute` is two
 * fetches (describe, then cursor) and each `batch` one. A run with grants to
 * make costs the SELECT (2), this setting (2), the allowance setting (2) and
 * one batch per grant, so 10 grants is about 16 subrequests and a run with
 * nothing due only 2; a grant whose stream breaks costs one more request,
 * so the worst case at 10 is 26. The Workers Free plan allows 50
 * subrequests per invocation: on Free keep plus_yearly_grants_per_run at
 * about 20 or less (the default is 10). On the Paid plan (10,000 by default)
 * it can be raised to 200. The hourly trigger makes 10 a run about 240
 * grants a day. */
export async function readYearlyGrantsPerRun(db: SqlExecutor): Promise<number> {
  const rows = await db.execute("SELECT value FROM server_setting WHERE key = 'plus_yearly_grants_per_run'", []);
  return wholeSetting("plus_yearly_grants_per_run", rows, MAX_YEARLY_GRANTS_PER_RUN, DEFAULT_YEARLY_GRANTS_PER_RUN);
}

/** The amount of the account's newest purchase, 0 when it has none. */
export async function lastPurchaseMicros(db: SqlExecutor, accountId: string): Promise<number> {
  const rows = await db.execute(
    "SELECT micros FROM ai_ledger WHERE account_id = ? AND kind = 'purchase' ORDER BY created_at_unix_ms DESC, rowid DESC LIMIT 1",
    [accountId],
  );
  return rows.length ? Number(rows[0].micros) : 0;
}

/** The purchase credited under this Stripe reference (the PaymentIntent
 * id, ruling R6), or null when there is none. */
export async function purchaseByStripeRef(db: SqlExecutor, stripeRef: string): Promise<{ accountId: string; micros: number } | null> {
  const rows = await db.execute("SELECT account_id, micros FROM ai_ledger WHERE stripe_ref = ? AND kind = 'purchase'", [stripeRef]);
  return rows.length ? { accountId: String(rows[0].account_id), micros: Number(rows[0].micros) } : null;
}

/** Takes `amount` (rounded up to 100 micros) up front, from the live
 * allowance first and the rest from prepaid. Returns the hold id, or null
 * when the prepaid part is more than the prepaid balance or the account
 * already has 4 holds open (at most 4 requests in flight per account).
 *
 * Statement order matters: the hold row is written first, its split and
 * allowance window selected from the account row as it stands and guarded
 * on the funds and the cap; the account is then debited from that hold row.
 * The split is never read after the debit. */
export async function placeHold(db: SqlExecutor, accountId: string, amount: number, now: number): Promise<string | null> {
  checkMicros("hold", amount, true);
  const micros = roundUpDebit(amount);
  const id = crypto.randomUUID();
  const [inserted] = await db.batch([
    {
      sql: `INSERT INTO ai_hold (id, account_id, micros, created_at_unix_ms, from_allowance_micros, allowance_expires_ms) SELECT ?, id, ?, ?, MIN(?, ${LIVE_ALLOWANCE}), CASE WHEN ai_allowance_expires_ms > ? THEN ai_allowance_expires_ms ELSE NULL END FROM account WHERE id = ? AND ai_open_holds < 4 AND ai_credit_micros >= ? - MIN(?, ${LIVE_ALLOWANCE}) RETURNING id`,
      params: [id, micros, now, micros, now, now, accountId, micros, micros, now],
    },
    // Our fresh hold id exists only if the guarded insert above applied.
    {
      sql: "UPDATE account SET ai_allowance_micros = ai_allowance_micros - (SELECT from_allowance_micros FROM ai_hold WHERE id = ?), ai_credit_micros = ai_credit_micros - (SELECT micros - from_allowance_micros FROM ai_hold WHERE id = ?), ai_open_holds = ai_open_holds + 1 WHERE id = ? AND EXISTS (SELECT 1 FROM ai_hold WHERE id = ?)",
      params: [id, id, accountId, id],
    },
  ]);
  return inserted.length > 0 ? id : null;
}

/** Closes a hold in one transaction: takes the charge from each bucket,
 * gives back what the hold reserved but did not spend, frees the in-flight
 * slot, writes one `usage` ledger row and deletes the hold. Settling a hold
 * that is already gone matches nothing, so it cannot charge twice.
 *
 * With a hold of Ha (allowance) + Hp (prepaid), the real cost C (rounded up
 * once, on the total, to 100 micros), the live allowance A (counted only
 * while it is still the window the hold drew from) and prepaid P:
 * a = min(C, Ha + A) from the allowance, p = min(C - a, Hp + P) from
 * prepaid, so neither bucket goes below 0. Ha - a goes back to the
 * allowance only if the account's allowance window is still the hold's
 * (a new month's grant is never topped up by last month's hold); Hp - p
 * goes back to prepaid. The ledger row records the real cost,
 * micros = -C (even when a + p is less because both buckets ran out), and
 * from_allowance_micros = a.
 *
 * `usage` null means the request died without a bill: the whole hold is
 * the charge, each bucket paying exactly its reserved share (a = Ha,
 * p = Hp), whatever has since come back to the allowance.
 *
 * Statement order mirrors placeHold: the ledger row is written first, from
 * the hold and the account row as they stand (C and a); the account is
 * then updated from that ledger row, p being computed inside that one
 * UPDATE from the prepaid balance before it changes; then the hold is
 * deleted. */
export async function settleHold(db: SqlExecutor, holdId: string, usage: AiUsage | null, now: number): Promise<void> {
  if (usage) checkMicros("costMicros", usage.costMicros);
  const cost = usage ? roundUpDebit(usage.costMicros) : null;
  const ledgerId = crypto.randomUUID();
  await db.batch([
    {
      sql: "INSERT INTO ai_ledger (id, account_id, kind, micros, model, input_tokens, output_tokens, stripe_ref, created_at_unix_ms, from_allowance_micros) SELECT ?, account_id, 'usage', 0 - c, ?, ?, ?, NULL, ?, a FROM (SELECT h.account_id AS account_id, COALESCE(x.cost, h.micros) AS c, CASE WHEN x.cost IS NULL THEN h.from_allowance_micros ELSE MIN(x.cost, h.from_allowance_micros + CASE WHEN acc.ai_allowance_expires_ms = h.allowance_expires_ms AND acc.ai_allowance_expires_ms > ? THEN acc.ai_allowance_micros ELSE 0 END) END AS a FROM ai_hold h JOIN account acc ON acc.id = h.account_id CROSS JOIN (SELECT ? AS cost) x WHERE h.id = ?)",
      params: [ledgerId, usage?.model ?? null, usage?.inputTokens ?? null, usage?.outputTokens ?? null, now, now, cost, holdId],
    },
    // Our fresh ledger id exists only if the hold was still open.
    {
      sql: "UPDATE account SET ai_allowance_micros = ai_allowance_micros + CASE WHEN ai_allowance_expires_ms = (SELECT allowance_expires_ms FROM ai_hold WHERE id = ?) THEN (SELECT from_allowance_micros FROM ai_hold WHERE id = ?) - (SELECT from_allowance_micros FROM ai_ledger WHERE id = ?) ELSE 0 END, ai_credit_micros = ai_credit_micros + (SELECT micros - from_allowance_micros FROM ai_hold WHERE id = ?) - MIN((SELECT 0 - micros - from_allowance_micros FROM ai_ledger WHERE id = ?), (SELECT micros - from_allowance_micros FROM ai_hold WHERE id = ?) + ai_credit_micros), ai_open_holds = MAX(ai_open_holds - 1, 0) WHERE id = (SELECT account_id FROM ai_ledger WHERE id = ?)",
      // In order: hold (its window), hold (Ha), ledger (a), hold (Hp),
      // ledger (C - a), hold (Hp, for the prepaid cap), ledger (WHERE).
      params: [holdId, holdId, ledgerId, holdId, ledgerId, holdId, ledgerId],
    },
    { sql: "DELETE FROM ai_hold WHERE id = ?", params: [holdId] },
  ]);
}

/** Settles, with no usage, every hold older than STALE_HOLD_MS. Returns how
 * many were found. Run from the daily cron. */
export async function settleStaleHolds(db: SqlExecutor, now: number): Promise<number> {
  const stale = await db.execute("SELECT id FROM ai_hold WHERE created_at_unix_ms < ?", [now - STALE_HOLD_MS]);
  for (const row of stale) await settleHold(db, String(row.id), null, now);
  return stale.length;
}

/** Adds a paid pack to the balance, in one transaction with its ledger row.
 * Returns false, and adds nothing, when this Stripe reference was already
 * credited, or when the account is gone or being deleted (the ledger row is
 * written only for a live account, so a deletion that wins the race leaves
 * nothing behind; the caller tells the two apart). */
export async function creditPurchase(
  db: SqlExecutor,
  accountId: string,
  micros: number,
  stripeRef: string,
  now: number,
): Promise<boolean> {
  checkMicros("purchase", micros);
  const ledgerId = crypto.randomUUID();
  const [inserted] = await db.batch([
    {
      sql: "INSERT INTO ai_ledger (id, account_id, kind, micros, model, input_tokens, output_tokens, stripe_ref, created_at_unix_ms) SELECT ?, id, 'purchase', ?, NULL, NULL, NULL, ?, ? FROM account WHERE id = ? AND deleting_since_unix_ms IS NULL ON CONFLICT(stripe_ref) DO NOTHING RETURNING id",
      params: [ledgerId, micros, stripeRef, now, accountId],
    },
    // Our fresh ledger id exists only if the insert above was not a duplicate.
    {
      sql: "UPDATE account SET ai_credit_micros = ai_credit_micros + ? WHERE id = ? AND EXISTS (SELECT 1 FROM ai_ledger WHERE id = ?)",
      params: [micros, accountId, ledgerId],
    },
  ]);
  return inserted.length > 0;
}

/** Takes a Stripe refund back out of the balance: min(refundMicros,
 * balance), never below 0, in one transaction with its `refund` ledger row.
 * Idempotent per Stripe refund id (`stripe_ref = 'refund:' + refundRef`).
 * Returns the micros removed, 0 on a repeat or an unknown account.
 *
 * Ruling R4: credit sitting in open holds is not in `ai_credit_micros`, so
 * a refund that lands mid-request can remove less than it should. That is
 * accepted: holds last seconds, refunds are manual, and the shortfall is
 * bounded by the holds open at that instant. The ledger row records it in
 * `shortfall_micros` (refund minus removed) so it is visible. */
export async function refundPurchase(
  db: SqlExecutor,
  accountId: string,
  refundMicros: number,
  refundRef: string,
  now: number,
): Promise<number> {
  checkMicros("refund", refundMicros);
  const ledgerId = crypto.randomUUID();
  const [inserted] = await db.batch([
    {
      sql: "INSERT INTO ai_ledger (id, account_id, kind, micros, model, input_tokens, output_tokens, stripe_ref, created_at_unix_ms, shortfall_micros) SELECT ?, id, 'refund', -MIN(?, ai_credit_micros), NULL, NULL, NULL, ?, ?, ? - MIN(?, ai_credit_micros) FROM account WHERE id = ? ON CONFLICT(stripe_ref) DO NOTHING RETURNING micros",
      params: [ledgerId, refundMicros, `refund:${refundRef}`, now, refundMicros, refundMicros, accountId],
    },
    {
      sql: "UPDATE account SET ai_credit_micros = ai_credit_micros + (SELECT micros FROM ai_ledger WHERE id = ?) WHERE id = ? AND EXISTS (SELECT 1 FROM ai_ledger WHERE id = ?)",
      params: [ledgerId, accountId, ledgerId],
    },
  ]);
  return inserted.length > 0 ? -Number(inserted[0].micros) : 0;
}

/** Grants a Plus allowance, in one transaction. The grant row is written
 * only for a live account (not missing, not being deleted) and only once
 * per `grantKey` (the Stripe invoice id, or "<invoice id>:m<n>"); only if
 * that insert applied, and only for a LATER period end than the current
 * one, is the allowance REPLACED (no rollover) and its expiry moved to
 * `periodEndUnixMs`. A late or out-of-order grant for an earlier period
 * end, or a duplicate for the same period end, keeps its row for the audit
 * trail but changes nothing.
 *
 * One exception to "later only": when the current window is a zero of
 * ANOTHER subscription (the account's allowance is 0 and a zero marker of a
 * different, known `subscription_id` ends exactly at the stored expiry, or
 * 1 ms before it after a repeat zero in the same ms), a grant carrying its
 * own `subscriptionId` lands whatever its period end, so a subscription
 * bought after a refund of an earlier one always gets its first month. It
 * switches itself off once used: that grant's whole-second end is never a
 * marker's end (see zeroAllowance), so the next grant of the new
 * subscription is under the plain guard again. A NULL subscription id on
 * either side (rows written before migration 0007's column) never takes
 * the exception, and neither does a grant of the zeroed subscription
 * itself.
 *
 * Returns false, writing nothing, for a repeat key (including a refund
 * tombstone, see zeroAllowance), a missing or deleting account,
 * `micros <= 0`, or a period that has already ended
 * (`periodEndUnixMs <= now`). */
export async function grantAllowance(
  db: SqlExecutor,
  args: GrantArgs,
  now: number,
): Promise<boolean> {
  return (await writeGrant(db, args, now)).written;
}

/** A grant: `lineStartUnixMs`/`lineEndUnixMs`, when given (both or
 * neither), are the Stripe invoice line's period, stored on the grant row
 * so the yearly cron can follow the paid line. `subscriptionId` is the
 * Stripe subscription the grant is for, stored on the row (NULL when not
 * given). */
export interface GrantArgs {
  accountId: string;
  grantKey: string;
  micros: number;
  periodEndUnixMs: number;
  lineStartUnixMs?: number;
  lineEndUnixMs?: number;
  subscriptionId?: string;
}

/** grantAllowance's transaction. `written`: the grant row was inserted;
 * `refilled`: the allowance was also replaced (the forward guard let it). */
export async function writeGrant(
  db: SqlExecutor,
  args: GrantArgs,
  now: number,
): Promise<{ written: boolean; refilled: boolean }> {
  if (!Number.isSafeInteger(args.micros) || !Number.isSafeInteger(args.periodEndUnixMs)) {
    throw new RangeError(`allowance grant needs whole numbers, got micros ${args.micros}, periodEndUnixMs ${args.periodEndUnixMs}`);
  }
  const lineStart = args.lineStartUnixMs ?? null;
  const lineEnd = args.lineEndUnixMs ?? null;
  if ((lineStart === null) !== (lineEnd === null) || (lineStart !== null && (!Number.isSafeInteger(lineStart) || !Number.isSafeInteger(lineEnd) || lineStart >= lineEnd!))) {
    throw new RangeError(`allowance grant line bounds must be two whole numbers, start before end, or neither; got ${lineStart}, ${lineEnd}`);
  }
  if (args.subscriptionId !== undefined && (typeof args.subscriptionId !== "string" || args.subscriptionId === "")) {
    throw new RangeError("allowance grant subscription id must be a non-empty string when given");
  }
  const sub = args.subscriptionId ?? null;
  if (args.micros <= 0 || args.periodEndUnixMs <= now) return { written: false, refilled: false };
  const [inserted, updated] = await db.batch([
    {
      sql: "INSERT INTO ai_allowance_grant (grant_key, account_id, micros, period_end_unix_ms, created_at_unix_ms, line_start_unix_ms, line_end_unix_ms, subscription_id) SELECT ?, id, ?, ?, ?, ?, ?, ? FROM account WHERE id = ? AND deleting_since_unix_ms IS NULL ON CONFLICT(grant_key) DO NOTHING RETURNING grant_key",
      params: [args.grantKey, args.micros, args.periodEndUnixMs, now, lineStart, lineEnd, sub, args.accountId],
    },
    // changes() is the row count of the insert just above, in this same
    // transaction: 0 for a repeat key or a missing or deleting account. The
    // EXISTS is the "current window is another subscription's zero" exception.
    {
      sql: "UPDATE account SET ai_allowance_micros = ?, ai_allowance_expires_ms = ? WHERE id = ? AND changes() = 1 AND (ai_allowance_expires_ms IS NULL OR ai_allowance_expires_ms < ? OR (ai_allowance_micros = 0 AND EXISTS (SELECT 1 FROM ai_allowance_grant z WHERE z.account_id = account.id AND substr(z.grant_key, 1, 7) = 'zeroed:' AND z.subscription_id IS NOT NULL AND z.subscription_id <> ? AND z.period_end_unix_ms IN (account.ai_allowance_expires_ms, account.ai_allowance_expires_ms - 1)))) RETURNING id",
      params: [args.micros, args.periodEndUnixMs, args.accountId, args.periodEndUnixMs, sub],
    },
  ]);
  return { written: inserted.length > 0, refilled: updated.length > 0 };
}

/** How a zero ends the window. `subscriptionId` is the Plus subscription
 * being zeroed (written on the marker; null only in tests of rows from
 * before migration 0007's column). `mode`:
 * - "cancel": the subscription has ended (immediate cancel, unpaid,
 *   expired, paused). The expiry becomes now + 1 ms. Only that subscription
 *   is blocked, and by the webhook, not the expiry: an ended subscription's
 *   late invoice.paid never reaches a grant (its plan is free, or the
 *   account is on another subscription). A new subscription bought straight
 *   after is never refused because the old window ran later.
 * - "refund": a full refund of a still-active subscription's current
 *   invoice. The expiry becomes MAX(old expiry, now) + 1 ms, so the forward
 *   guard refuses the refunded window itself and any late grant of that
 *   subscription ending inside it.
 * `tombstoneKey`: a refunded invoice with no grant row yet. A row under that
 * key (micros 0, period end 0) is written in the same transaction, so the
 * invoice's own later invoice.paid is a repeat key and grants nothing. */
export interface ZeroArgs {
  subscriptionId: string | null;
  mode: "cancel" | "refund";
  tombstoneKey?: string;
}

/** Removes the unspent allowance of a refunded or ended Plus subscription,
 * in one transaction: the allowance becomes 0 and the expiry moves as
 * `zero.mode` says (an old NULL expiry counts as 0).
 *
 * - An open hold drew from the old window, whose stored expiry is the old
 *   expiry (or, for a hold placed after an earlier zero, an earlier zero's
 *   value with no allowance share). "refund": old + 1 ms equals neither.
 *   "cancel": now + 1 ms could equal a hold's window only if that window
 *   ended 1 ms after the cancel, when a refill would be spendable for at most
 *   1 ms. No grant sets the zero's expiry back, so no settle refills the
 *   allowance in practice.
 * - "refund": the expiry-forward guard in grantAllowance (`expiry < period
 *   end`) then refuses every grant whose period end is at or before old end
 *   + 1 ms: the refunded period itself, any late grant for a period ending
 *   inside it, and anything already ended. Such a grant's row is still
 *   recorded for the audit trail, as for any late grant, but changes
 *   nothing. A genuinely later period (a Stripe period end is a whole
 *   second, so more than old end + 1 ms) grants its full amount. A grant of
 *   ANOTHER subscription lands whatever its end (grantAllowance's
 *   exception).
 * - "cancel": now + 1 ms, see ZeroArgs.
 *
 * No over-credit residual is left. The expiry it leaves can be later than
 * now while the allowance is 0 (see allowanceView).
 *
 * In the same transaction it records a marker row in ai_allowance_grant
 * (grant key `zeroed:<accountId>:<now>`, micros 0, period end = the new
 * expiry, subscription_id = the zeroed subscription), so the yearly cron
 * can tell a zeroed subscription from one whose window merely expired. A
 * repeat zero in the same millisecond keeps the first marker (ON CONFLICT DO
 * NOTHING). */
export async function zeroAllowance(db: SqlExecutor, accountId: string, now: number, zero: ZeroArgs): Promise<void> {
  if (zero.mode !== "cancel" && zero.mode !== "refund") throw new RangeError(`zeroAllowance mode must be cancel or refund, got ${String(zero.mode)}`);
  const stmts = [
    zero.mode === "cancel"
      ? { sql: "UPDATE account SET ai_allowance_micros = 0, ai_allowance_expires_ms = ? + 1 WHERE id = ?", params: [now, accountId] }
      : { sql: "UPDATE account SET ai_allowance_micros = 0, ai_allowance_expires_ms = MAX(COALESCE(ai_allowance_expires_ms, 0), ?) + 1 WHERE id = ?", params: [now, accountId] },
    {
      sql: "INSERT INTO ai_allowance_grant (grant_key, account_id, micros, period_end_unix_ms, created_at_unix_ms, subscription_id) SELECT ?, id, 0, ai_allowance_expires_ms, ?, ? FROM account WHERE id = ? ON CONFLICT(grant_key) DO NOTHING",
      params: [`zeroed:${accountId}:${now}`, now, zero.subscriptionId, accountId],
    },
  ];
  if (zero.tombstoneKey !== undefined) {
    stmts.push({
      sql: "INSERT INTO ai_allowance_grant (grant_key, account_id, micros, period_end_unix_ms, created_at_unix_ms, subscription_id) SELECT ?, id, 0, 0, ?, ? FROM account WHERE id = ? ON CONFLICT(grant_key) DO NOTHING",
      params: [zero.tombstoneKey, now, zero.subscriptionId, accountId],
    });
  }
  await db.batch(stmts);
}

/** One year back is at most 366 days. Grant rows ending before
 * `plan_until - YEAR_BACK_MS` cannot belong to the current year (a yearly
 * line ends at or after plan_until, and its base grant ends after its
 * start), so the SELECT skips them; the exact checks are done in JS. */
export const YEAR_BACK_MS = 366 * 24 * 60 * 60 * 1000;

/** The month of a yearly period [startMs, endMs) that contains `now`: month
 * n (1..12) runs from start + (n - 1) months to start + n months
 * (monthWindowEnd's day clamp), and month 12 always ends at `endMs` (so a
 * year paid through 29 Feb has no uncovered last day). null when `now` is
 * at or after `endMs`. A `now` before the start counts as month 1. */
export function yearlyMonthAt(startMs: number, endMs: number, now: number): { n: number; endMs: number } | null {
  if (now >= endMs) return null;
  let n = 1;
  for (let m = 2; m <= 12; m++) if (now >= addMonthsUtc(startMs, m - 1)) n = m;
  return { n, endMs: n === 12 ? endMs : monthWindowEnd(startMs, n, endMs) };
}

/** True when a stored invoice line [start, end) is a yearly line (it spans
 * more than 11 months), so it can define the year's month windows. A
 * monthly line (a monthly-to-yearly switch's last monthly grant) cannot. */
function isYearlyLine(startMs: number, endMs: number): boolean {
  return endMs > addMonthsUtc(startMs, 11);
}

/** Grants months 2 to 12 of a yearly Plus subscription's monthly allowance
 * (month 1, or the month it was paid in when paid late, comes from the paid
 * invoice). Run from the hourly yearly-grants trigger. Returns how many
 * grants refilled the allowance (a grant row whose refill the forward guard
 * refused is written but not counted).
 *
 * An account qualifies while `plan_tier = 'plus'`, `plan_interval = 'year'`,
 * `plan_until_unix_ms > now` and it is not being deleted (a cancel at period
 * end keeps its grants until `plan_until`; an immediate cancel makes the
 * plan free). G1, its base grant for the current year, is the grant row with
 * the latest period end (then the latest created, then the greatest key)
 * among those whose key is neither a zero marker (`zeroed:`) nor a month
 * key (`:m`) and that belong to the current year:
 * - a row with a stored yearly invoice line (migration 0007) belongs when
 *   its line has not ended (`line_end > now`); the year is that line, S =
 *   line start, yearEnd = line end, so a mid-year `cancel_at` (which moves
 *   plan_until) does not move the windows;
 * - a row without one (written before 0007, or a monthly line) belongs when
 *   its period end is after S, where the year is derived from plan_until:
 *   yearEnd = plan_until, S = yearEnd - 12 calendar months.
 * Month n is yearlyMonthAt(S, yearEnd). Only the CURRENT month n (2..12) is
 * granted, never a missed earlier one, under the key `<G1>:m<n>`. No G1 (the
 * first invoice never granted) means no grant, and so does a G1 that
 * already reaches the end of month n (an invoice paid late, in month n).
 * The month grant stores G1's stored line (if any) as its line bounds and
 * G1's subscription id (the account's current one when G1 has none).
 *
 * Only rows of the account's current subscription can be G1: a row whose
 * `subscription_id` is set and differs from the account's
 * `stripe_subscription_id` (a cancelled earlier subscription) is skipped, as
 * is a refund tombstone (micros 0). A NULL on either side (rows written
 * before migration 0007's column) does not exclude.
 *
 * A zeroed subscription (refund or immediate cancel) gets nothing: it is
 * zeroed when a marker row of the account has `period_end_unix_ms >= G1`'s
 * and belongs to G1's own subscription (same `subscription_id`, or NULL on
 * either side, the rule before the column: any marker blocks), or when it
 * belongs to G1's known subscription and was written at or after G1's row
 * (a cancel-mode zero ends at now + 1, earlier than G1's window, so a stale
 * write that puts the plan back after the cancel still grants nothing). A zero of
 * another, earlier subscription never blocks a new one. Not "the newest row
 * is a marker" (a refused late grant writes a row after it), not "the
 * expiry equals the marker's" (a repeat zero in the same ms moves the
 * expiry 1 ms on), not created_at order.
 *
 * Only accounts whose allowance window has ended (expiry NULL or <= now)
 * are read at all. The Workers Free plan gives a Cron Trigger 10 ms of CPU,
 * and reading every yearly account with its grant rows every hour would pass
 * that at a few hundred accounts; this way a run reads only the accounts at
 * a month boundary since their window ended. It loses no grant: the expiry
 * is always the end of a granted window (a month grant, a late invoice's
 * month, or a zero, after which the marker rule blocks anyway), so a due
 * account's window has ended. Accepted side effect: when the base grant
 * outlasts the start of month 2 (a monthly-to-yearly switch whose last
 * monthly window, anchored on the 29th to 31st, ends up to 3 days after the
 * yearly month 2 starts), month 2 is granted when that window ends, up to 3
 * days late, and so runs up to 3 days short.
 *
 * Idempotent: an account whose current month key exists is skipped, and
 * grantAllowance's key guard makes a repeat write nothing anyway. One
 * SELECT (eligible accounts joined to their grant rows of about this year),
 * then, once something is due, one read each of the per-run cap
 * (readYearlyGrantsPerRun) and the allowance setting, then one grant per
 * due account, ordered by account id, at most the cap per run. An account
 * whose grant throws is logged by id and skipped; the run goes on.
 *
 * Without migration 0007's columns the SELECT fails: that logs the one line
 * "plus grants need migration 0007" and the run grants nothing. */
export async function grantDueYearlyMonths(db: SqlExecutor, now: number): Promise<number> {
  let rows: SqlRow[];
  try {
    rows = await db.execute(
      "SELECT a.id AS account_id, a.plan_until_unix_ms AS plan_until, a.stripe_subscription_id AS current_sub, g.grant_key AS grant_key, g.micros AS micros, g.period_end_unix_ms AS period_end, g.created_at_unix_ms AS created_at, g.line_start_unix_ms AS line_start, g.line_end_unix_ms AS line_end, g.subscription_id AS sub FROM account a JOIN ai_allowance_grant g ON g.account_id = a.id AND g.period_end_unix_ms > a.plan_until_unix_ms - ? WHERE a.plan_tier = 'plus' AND a.plan_interval = 'year' AND a.plan_until_unix_ms > ? AND a.deleting_since_unix_ms IS NULL AND (a.ai_allowance_expires_ms IS NULL OR a.ai_allowance_expires_ms <= ?) ORDER BY a.id",
      [YEAR_BACK_MS, now, now],
    );
  } catch (err) {
    // The deploy order slipped (Worker before migration 0007): one clear line per run, not a stack every hour.
    if (/no such column/i.test(String(err))) {
      console.error("plus grants need migration 0007");
      return 0;
    }
    throw err;
  }
  type Grant = { key: string; micros: number; periodEnd: number; createdAt: number; lineStart: number | null; lineEnd: number | null; sub: string | null };
  const byAccount = new Map<string, { planUntil: number; currentSub: string | null; grants: Grant[] }>();
  const num = (v: unknown) => (v === null || v === undefined ? null : Number(v));
  const str = (v: unknown) => (v === null || v === undefined ? null : String(v));
  for (const r of rows) {
    const id = String(r.account_id);
    let entry = byAccount.get(id);
    if (!entry) byAccount.set(id, (entry = { planUntil: Number(r.plan_until), currentSub: str(r.current_sub), grants: [] }));
    entry.grants.push({
      key: String(r.grant_key),
      micros: Number(r.micros),
      periodEnd: Number(r.period_end),
      createdAt: Number(r.created_at),
      lineStart: num(r.line_start),
      lineEnd: num(r.line_end),
      sub: str(r.sub),
    });
  }
  let cap: number | null = null;
  let micros: number | null = null;
  let calls = 0;
  let refilled = 0;
  let left = 0;
  for (const [accountId, { planUntil, currentSub, grants }] of byAccount) {
    if (!Number.isSafeInteger(planUntil)) continue;
    const derived = { start: addMonthsUtc(planUntil, -12), end: planUntil };
    let g1: (Grant & { year: { start: number; end: number }; fromLine: boolean }) | null = null;
    for (const g of grants) {
      if (g.key.startsWith("zeroed:") || g.key.includes(":m") || !(g.micros > 0)) continue;
      // Another (earlier, cancelled) subscription's row is never this subscription's base grant.
      if (g.sub !== null && currentSub !== null && g.sub !== currentSub) continue;
      let year = derived;
      const fromLine = g.lineStart !== null && g.lineEnd !== null && isYearlyLine(g.lineStart, g.lineEnd);
      if (fromLine) {
        if (!(g.lineEnd! > now)) continue;
        year = { start: g.lineStart!, end: g.lineEnd! };
      } else if (!(g.periodEnd > derived.start)) continue;
      // (Fallback, a year derived from plan_until. Accepted for v1: after a
      // monthly-to-yearly switch, which grants nothing on the switch
      // invoice, the first yearly year has no stored yearly line, so a
      // mid-year cancel_at still shifts its windows, one extra early refill
      // at most.)
      if (!g1 || g.periodEnd > g1.periodEnd || (g.periodEnd === g1.periodEnd && (g.createdAt > g1.createdAt || (g.createdAt === g1.createdAt && g.key > g1.key)))) g1 = { ...g, year, fromLine };
    }
    if (!g1) continue;
    const base = g1;
    const month = yearlyMonthAt(base.year.start, base.year.end, now);
    if (!month || month.n < 2) continue; // month 1 is the invoice's
    // Only a zero of G1's own subscription blocks (NULL on either side: any zero blocks, the rule before the column).
    // It blocks when it ends at or after G1, or, for G1's own known subscription, when it was written at or after
    // G1's row (a cancel-mode zero ends at now + 1, before G1's window; both are server clocks).
    if (
      grants.some(
        (g) =>
          g.key.startsWith("zeroed:") &&
          (g.periodEnd >= base.periodEnd || (g.sub !== null && g.sub === base.sub && g.createdAt >= base.createdAt)) &&
          (g.sub === null || base.sub === null || g.sub === base.sub),
      )
    )
      continue;
    if (base.periodEnd >= month.endMs) continue; // the invoice's own grant already covers this month
    const grantKey = `${base.key}:m${month.n}`;
    if (grants.some((g) => g.key === grantKey)) continue;
    cap ??= await readYearlyGrantsPerRun(db);
    if (calls >= cap) {
      left++;
      continue;
    }
    calls++;
    micros ??= await readAllowanceSetting(db);
    try {
      // A month grant records G1's stored line; a year derived from plan_until is not a line, so it stays NULL.
      const line = base.fromLine ? { lineStartUnixMs: base.year.start, lineEndUnixMs: base.year.end } : {};
      const subscriptionId = base.sub ?? currentSub ?? undefined;
      const args: GrantArgs = { accountId, grantKey, micros, periodEndUnixMs: month.endMs, ...line, ...(subscriptionId !== undefined ? { subscriptionId } : {}) };
      if ((await writeGrant(db, args, now)).refilled) refilled++;
    } catch (err) {
      console.error("plus yearly grant failed, account", accountId, String(err));
    }
  }
  if (left > 0) console.warn("plus yearly grants: per-run limit reached,", left, "due accounts left for the next run");
  return refilled;
}
