import type { SqlExecutor } from "./accounts";

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
 * rare case where less was taken because the balance ran out. */

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

export async function aiBalance(db: SqlExecutor, accountId: string): Promise<number> {
  const rows = await db.execute("SELECT ai_credit_micros FROM account WHERE id = ?", [accountId]);
  return rows.length ? Number(rows[0].ai_credit_micros) : 0;
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

/** Takes `micros` from the balance up front. Returns the hold id, or null
 * when the balance is short or the account already has 4 holds open (at
 * most 4 requests in flight per account). The hold is rounded up to 100
 * micros. */
export async function placeHold(db: SqlExecutor, accountId: string, amount: number, now: number): Promise<string | null> {
  checkMicros("hold", amount, true);
  const micros = roundUpDebit(amount);
  const id = crypto.randomUUID();
  const [, inserted] = await db.batch([
    {
      sql: "UPDATE account SET ai_credit_micros = ai_credit_micros - ?, ai_open_holds = ai_open_holds + 1 WHERE id = ? AND ai_credit_micros >= ? AND ai_open_holds < 4 RETURNING ai_credit_micros, ai_open_holds",
      params: [micros, accountId, micros],
    },
    // changes() is the row count of the UPDATE just above, in this same
    // transaction: the hold row exists only if the debit happened.
    {
      sql: "INSERT INTO ai_hold (id, account_id, micros, created_at_unix_ms) SELECT ?, ?, ?, ? WHERE changes() = 1 RETURNING id",
      params: [id, accountId, micros, now],
    },
  ]);
  return inserted.length > 0 ? id : null;
}

/** Closes a hold in one transaction: gives the held amount back, takes the
 * charge, frees the in-flight slot, writes one `usage` ledger row and
 * deletes the hold. The charge is the real cost (or the whole hold when
 * `usage` is null, i.e. the request died without a bill), rounded up to 100
 * micros, capped at
 * hold + balance so the balance never goes below 0. Settling a hold that is
 * already gone matches nothing, so it cannot charge twice. */
export async function settleHold(db: SqlExecutor, holdId: string, usage: AiUsage | null, now: number): Promise<void> {
  if (usage) checkMicros("costMicros", usage.costMicros);
  const cost = usage ? roundUpDebit(usage.costMicros) : null;
  await db.batch([
    {
      sql: "UPDATE account SET ai_credit_micros = ai_credit_micros + (SELECT micros FROM ai_hold WHERE id = ?) - MIN(COALESCE(?, (SELECT micros FROM ai_hold WHERE id = ?)), ai_credit_micros + (SELECT micros FROM ai_hold WHERE id = ?)), ai_open_holds = MAX(ai_open_holds - 1, 0) WHERE id = (SELECT account_id FROM ai_hold WHERE id = ?)",
      params: [holdId, cost, holdId, holdId, holdId],
    },
    {
      sql: "INSERT INTO ai_ledger (id, account_id, kind, micros, model, input_tokens, output_tokens, stripe_ref, created_at_unix_ms) SELECT ?, account_id, 'usage', -COALESCE(?, micros), ?, ?, ?, NULL, ? FROM ai_hold WHERE id = ?",
      params: [crypto.randomUUID(), cost, usage?.model ?? null, usage?.inputTokens ?? null, usage?.outputTokens ?? null, now, holdId],
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
