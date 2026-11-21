import { afterEach, describe, expect, it, vi } from "vitest";
import {
  STALE_HOLD_MS,
  allowanceView,
  creditPurchase,
  grantAllowance,
  placeHold,
  readAllowanceSetting,
  refundPurchase,
  settleHold,
  settleStaleHolds,
  spendable,
  spendableAllowance,
  zeroAllowance,
} from "../src/aiCredit";
import type { SqlExecutor } from "../src/accounts";
import { realSqlExecutor } from "./testHelpers/realSqlExecutor";

// The Plus allowance money path against REAL SQLite: these run the SQL text
// that ships, so a mutation of that SQL makes them fail (the fake twins in
// aiCredit.test.ts cannot prove that).

const NOW = Date.UTC(2026, 10, 19, 9, 0, 0);
const DAY = 24 * 60 * 60 * 1000;
const END1 = NOW + 30 * DAY;
const END2 = END1 + 30 * DAY;
const ACCOUNT = "acct-1";
/** A refund-mode zero with no subscription id: a marker as written before migration 0007's column. */
const LEGACY_REFUND = { subscriptionId: null, mode: "refund" } as const;

const ACCOUNT_INSERT =
  "INSERT INTO account (id, email, google_sub, data_db_name, data_db_url, created_at_unix_ms) VALUES (?, ?, ?, ?, ?, ?)";

let open: (SqlExecutor & { close(): void })[] = [];
afterEach(() => {
  for (const db of open) db.close();
  open = [];
  vi.restoreAllMocks();
});

/** A fresh account with `prepaid` micros of prepaid credit and, when
 * `allowance` is given, one grant of it expiring at `allowanceEnd`. */
async function account(prepaid: number, allowance?: number, allowanceEnd = END1) {
  const db = realSqlExecutor();
  open.push(db);
  await db.execute(ACCOUNT_INSERT, [ACCOUNT, "a@x.test", "g-1", "db-1", "https://db-1.test", 1]);
  if (prepaid > 0) await db.execute("UPDATE account SET ai_credit_micros = ? WHERE id = ?", [prepaid, ACCOUNT]);
  if (allowance !== undefined) {
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_seed", micros: allowance, periodEndUnixMs: allowanceEnd }, NOW)).toBe(true);
  }
  return db;
}

/** Both buckets and the in-flight count, straight from the account row. */
async function buckets(db: SqlExecutor) {
  const [row] = await db.execute(
    "SELECT ai_credit_micros AS prepaid, ai_allowance_micros AS allowance, ai_allowance_expires_ms AS expires, ai_open_holds AS openHolds FROM account WHERE id = ?",
    [ACCOUNT],
  );
  return row;
}

async function holds(db: SqlExecutor) {
  return db.execute(
    "SELECT id, account_id, micros, from_allowance_micros, allowance_expires_ms, created_at_unix_ms FROM ai_hold ORDER BY created_at_unix_ms, id",
    [],
  );
}

async function usageRows(db: SqlExecutor) {
  return db.execute(
    "SELECT account_id, kind, micros, from_allowance_micros, model, input_tokens, output_tokens, created_at_unix_ms FROM ai_ledger WHERE kind = 'usage' ORDER BY created_at_unix_ms, rowid",
    [],
  );
}

async function grants(db: SqlExecutor) {
  return db.execute("SELECT grant_key, account_id, micros, period_end_unix_ms FROM ai_allowance_grant ORDER BY rowid", []);
}

const usage = (costMicros: number) => ({ model: "m", inputTokens: 10, outputTokens: 20, costMicros });

describe("placeHold", () => {
  it("places_a_hold_from_the_allowance_first", async () => {
    // Prepaid alone could cover the hold; the allowance must still go first.
    const db = await account(10_000_000, 6_000_000);
    const id = await placeHold(db, ACCOUNT, 1_000_000, NOW);
    expect(id).toEqual(expect.any(String));
    expect(await buckets(db)).toEqual({ prepaid: 10_000_000, allowance: 5_000_000, expires: END1, openHolds: 1 });
    expect(await holds(db)).toEqual([
      { id, account_id: ACCOUNT, micros: 1_000_000, from_allowance_micros: 1_000_000, allowance_expires_ms: END1, created_at_unix_ms: NOW },
    ]);
    expect(await usageRows(db)).toEqual([]);
  });

  it("splits_a_hold_larger_than_the_allowance_across_both_buckets", async () => {
    const db = await account(1_000_000, 600_000);
    const id = await placeHold(db, ACCOUNT, 1_000_000, NOW);
    expect(await buckets(db)).toEqual({ prepaid: 600_000, allowance: 0, expires: END1, openHolds: 1 });
    expect(await holds(db)).toEqual([
      { id, account_id: ACCOUNT, micros: 1_000_000, from_allowance_micros: 600_000, allowance_expires_ms: END1, created_at_unix_ms: NOW },
    ]);
    expect(await usageRows(db)).toEqual([]);
  });

  it("refuses_when_the_prepaid_remainder_is_short", async () => {
    const db = await account(300_000, 600_000);
    // 600,000 from the allowance leaves 400,000 for a prepaid balance of 300,000.
    expect(await placeHold(db, ACCOUNT, 1_000_000, NOW)).toBeNull();
    expect(await buckets(db)).toEqual({ prepaid: 300_000, allowance: 600_000, expires: END1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    // Exactly enough: 600,000 + 300,000.
    const id = await placeHold(db, ACCOUNT, 900_000, NOW);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: END1, openHolds: 1 });
    expect(await holds(db)).toEqual([expect.objectContaining({ id, micros: 900_000, from_allowance_micros: 600_000 })]);
    // An expired allowance is not spendable: a fresh account whose allowance
    // window has passed is refused for what prepaid cannot cover.
    const late = await account(300_000, 6_000_000);
    expect(await placeHold(late, ACCOUNT, 1_000_000, END1)).toBeNull();
    expect(await buckets(late)).toEqual({ prepaid: 300_000, allowance: 6_000_000, expires: END1, openHolds: 0 });
    expect(await holds(late)).toEqual([]);
    expect(await usageRows(late)).toEqual([]);
    expect(await usageRows(db)).toEqual([]);
  });

  it("four_open_holds_is_the_cap", async () => {
    const db = await account(0, 6_000_000);
    const ids: (string | null)[] = [];
    for (let i = 0; i < 5; i++) ids.push(await placeHold(db, ACCOUNT, 100_000, NOW));
    expect(ids.filter((x) => x !== null)).toHaveLength(4);
    expect(ids[4]).toBeNull();
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 5_600_000, expires: END1, openHolds: 4 });
    expect(await holds(db)).toEqual(
      ids
        .slice(0, 4)
        .map((id) => ({ id, account_id: ACCOUNT, micros: 100_000, from_allowance_micros: 100_000, allowance_expires_ms: END1, created_at_unix_ms: NOW }))
        .sort((x, y) => (x.id! < y.id! ? -1 : 1)),
    );
    await settleHold(db, ids[0]!, usage(0), NOW);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 5_700_000, expires: END1, openHolds: 3 });
    expect(await placeHold(db, ACCOUNT, 100_000, NOW)).not.toBeNull();
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 5_600_000, expires: END1, openHolds: 4 });
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: 0, from_allowance_micros: 0 })]);
  });
});

describe("settleHold", () => {
  it("settles_the_cost_to_the_allowance_then_prepaid", async () => {
    const db = await account(1_000_000, 600_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    // 700,050 rounds up once, on the total, to 700,100: 600,000 from the
    // allowance, 100,100 from prepaid.
    await settleHold(db, id, usage(700_050), NOW + 5);
    expect(await buckets(db)).toEqual({ prepaid: 899_900, allowance: 0, expires: END1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([
      { account_id: ACCOUNT, kind: "usage", micros: -700_100, from_allowance_micros: 600_000, model: "m", input_tokens: 10, output_tokens: 20, created_at_unix_ms: NOW + 5 },
    ]);
  });

  it("caps_the_charge_so_neither_bucket_goes_below_zero", async () => {
    // Over the hold, the same window's remaining allowance is drawn first.
    const db = await account(1_000_000, 2_000_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    await settleHold(db, id, usage(2_500_000), NOW);
    expect(await buckets(db)).toEqual({ prepaid: 500_000, allowance: 0, expires: END1, openHolds: 0 });
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: -2_500_000, from_allowance_micros: 2_000_000 })]);
    // Far over everything: hold + both balances is the most that is taken;
    // the usage row still records the real cost.
    const db2 = await account(1_000_000, 600_000);
    const id2 = (await placeHold(db2, ACCOUNT, 1_000_000, NOW))!;
    await settleHold(db2, id2, usage(5_000_000), NOW);
    expect(await buckets(db2)).toEqual({ prepaid: 0, allowance: 0, expires: END1, openHolds: 0 });
    expect(await holds(db2)).toEqual([]);
    expect(await usageRows(db2)).toEqual([expect.objectContaining({ micros: -5_000_000, from_allowance_micros: 600_000 })]);
  });

  it("usage_row_records_the_real_cost_even_when_both_buckets_run_dry", async () => {
    const db = await account(400_000, 600_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    // 1,500,050 rounds once to 1,500,100; only 1,000,000 can be taken.
    await settleHold(db, id, usage(1_500_050), NOW + 7);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: END1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([
      { account_id: ACCOUNT, kind: "usage", micros: -1_500_100, from_allowance_micros: 600_000, model: "m", input_tokens: 10, output_tokens: 20, created_at_unix_ms: NOW + 7 },
    ]);
  });

  it("returns_the_unspent_allowance_only_to_the_same_window", async () => {
    const db = await account(1_000_000, 6_000_000);
    const first = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    await settleHold(db, first, usage(200_000), NOW);
    // Same window: 800,000 comes back.
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 5_800_000, expires: END1, openHolds: 0 });
    const second = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 4_800_000, expires: END1, openHolds: 1 });
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_2", micros: 6_000_000, periodEndUnixMs: END2 }, END1)).toBe(true);
    await settleHold(db, second, usage(200_000), END1 + 1);
    // New window: the unspent 800,000 is dropped, never added to the new grant.
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 6_000_000, expires: END2, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([
      expect.objectContaining({ micros: -200_000, from_allowance_micros: 200_000 }),
      expect.objectContaining({ micros: -200_000, from_allowance_micros: 200_000 }),
    ]);
  });

  it("a_hold_settled_after_expiry_still_charges_its_allowance_part", async () => {
    const db = await account(1_000_000, 6_000_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, END1 - 1000))!;
    await settleHold(db, id, usage(700_000), END1 + 1000);
    // The allowance part was reserved while valid, so it pays; prepaid is untouched.
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 5_300_000, expires: END1, openHolds: 0 });
    expect(await spendableAllowance(db, ACCOUNT, END1 + 1000)).toBe(0);
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: -700_000, from_allowance_micros: 700_000 })]);
    // A hold placed after expiry draws prepaid only and records no window.
    const late = await placeHold(db, ACCOUNT, 500_000, END1 + 2000);
    expect(await buckets(db)).toEqual({ prepaid: 500_000, allowance: 5_300_000, expires: END1, openHolds: 1 });
    expect(await holds(db)).toEqual([expect.objectContaining({ id: late, micros: 500_000, from_allowance_micros: 0, allowance_expires_ms: null })]);
    // Over its hold after expiry: the expired allowance is not drawn for the
    // overage; it comes from prepaid.
    const over = await account(1_000_000, 6_000_000);
    const overId = (await placeHold(over, ACCOUNT, 1_000_000, END1 - 1000))!;
    await settleHold(over, overId, usage(1_300_000), END1 + 1000);
    expect(await buckets(over)).toEqual({ prepaid: 700_000, allowance: 5_000_000, expires: END1, openHolds: 0 });
    expect(await holds(over)).toEqual([]);
    expect(await usageRows(over)).toEqual([expect.objectContaining({ micros: -1_300_000, from_allowance_micros: 1_000_000 })]);
  });

  it("a_renewal_grant_during_an_open_hold_is_not_topped_up_by_the_settle", async () => {
    const db = await account(1_000_000, 6_000_000);
    const over = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    const under = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_2", micros: 6_000_000, periodEndUnixMs: END2 }, NOW + 1)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 6_000_000, expires: END2, openHolds: 2 });
    // Under its hold: the unspent 700,000 of last window's share is dropped,
    // never added to the new grant.
    await settleHold(db, under, usage(300_000), NOW + 2);
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 6_000_000, expires: END2, openHolds: 1 });
    // Over its hold: the old hold pays its own allowance share, and the
    // overage comes from prepaid, never from the new window.
    await settleHold(db, over, usage(1_300_000), NOW + 3);
    expect(await buckets(db)).toEqual({ prepaid: 700_000, allowance: 6_000_000, expires: END2, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([
      expect.objectContaining({ micros: -300_000, from_allowance_micros: 300_000 }),
      expect.objectContaining({ micros: -1_300_000, from_allowance_micros: 1_000_000 }),
    ]);
  });

  it("a_dead_request_before_acceptance_releases_the_hold_at_zero_cost", async () => {
    const db = await account(1_000_000, 600_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    await settleHold(db, id, usage(0), NOW);
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 600_000, expires: END1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: 0, from_allowance_micros: 0 })]);
    // Settling it again matches nothing.
    await settleHold(db, id, null, NOW);
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 600_000, expires: END1, openHolds: 0 });
    expect(await usageRows(db)).toHaveLength(1);
  });

  it("a_stale_hold_charges_the_whole_hold_per_bucket", async () => {
    const db = await account(1_000_000, 600_000);
    const small = (await placeHold(db, ACCOUNT, 200_000, NOW))!; // all allowance: 400,000 left
    const old = (await placeHold(db, ACCOUNT, 1_000_000, NOW - STALE_HOLD_MS - 1))!; // 400,000 allowance + 600,000 prepaid
    expect(await buckets(db)).toEqual({ prepaid: 400_000, allowance: 0, expires: END1, openHolds: 2 });
    // The small hold is released, so allowance is back in the window when
    // the stale one settles; the stale charge must still be split exactly
    // as the hold was, not shifted onto the allowance.
    await settleHold(db, small, usage(0), NOW);
    expect(await buckets(db)).toEqual({ prepaid: 400_000, allowance: 200_000, expires: END1, openHolds: 1 });
    expect(await settleStaleHolds(db, NOW)).toBe(1);
    expect(await buckets(db)).toEqual({ prepaid: 400_000, allowance: 200_000, expires: END1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([
      expect.objectContaining({ micros: 0, from_allowance_micros: 0 }),
      expect.objectContaining({ micros: -1_000_000, from_allowance_micros: 400_000, model: null, input_tokens: null }),
    ]);
    expect(old).not.toBe(small);
  });

  it("usage_rows_record_the_allowance_share", async () => {
    // Prepaid only (no allowance), then allowance only, then a split.
    const db = await account(2_000_000);
    const a = (await placeHold(db, ACCOUNT, 300_000, NOW))!;
    await settleHold(db, a, { model: "m1", inputTokens: 1, outputTokens: 2, costMicros: 100_000 }, NOW + 1);
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_1", micros: 500_000, periodEndUnixMs: END1 }, NOW + 2)).toBe(true);
    const b = (await placeHold(db, ACCOUNT, 300_000, NOW + 3))!;
    await settleHold(db, b, { model: "m2", inputTokens: 3, outputTokens: 4, costMicros: 250_000 }, NOW + 4);
    const c = (await placeHold(db, ACCOUNT, 600_000, NOW + 5))!;
    await settleHold(db, c, { model: "m3", inputTokens: 5, outputTokens: 6, costMicros: 400_000 }, NOW + 6);
    expect(await usageRows(db)).toEqual([
      { account_id: ACCOUNT, kind: "usage", micros: -100_000, from_allowance_micros: 0, model: "m1", input_tokens: 1, output_tokens: 2, created_at_unix_ms: NOW + 1 },
      { account_id: ACCOUNT, kind: "usage", micros: -250_000, from_allowance_micros: 250_000, model: "m2", input_tokens: 3, output_tokens: 4, created_at_unix_ms: NOW + 4 },
      { account_id: ACCOUNT, kind: "usage", micros: -400_000, from_allowance_micros: 250_000, model: "m3", input_tokens: 5, output_tokens: 6, created_at_unix_ms: NOW + 6 },
    ]);
    expect(await buckets(db)).toEqual({ prepaid: 1_750_000, allowance: 0, expires: END1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
  });

  it("a_prepaid_refund_does_not_touch_the_allowance", async () => {
    const db = await account(0, 6_000_000);
    expect(await creditPurchase(db, ACCOUNT, 10_000_000, "pi_1", NOW)).toBe(true);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    expect(await refundPurchase(db, ACCOUNT, 4_000_000, "re_1", NOW)).toBe(4_000_000);
    expect(await buckets(db)).toEqual({ prepaid: 6_000_000, allowance: 5_000_000, expires: END1, openHolds: 1 });
    await settleHold(db, id, usage(300_000), NOW);
    expect(await buckets(db)).toEqual({ prepaid: 6_000_000, allowance: 5_700_000, expires: END1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await db.execute("SELECT kind, micros, from_allowance_micros FROM ai_ledger ORDER BY rowid", [])).toEqual([
      { kind: "purchase", micros: 10_000_000, from_allowance_micros: null },
      { kind: "refund", micros: -4_000_000, from_allowance_micros: null },
      { kind: "usage", micros: -300_000, from_allowance_micros: 300_000 },
    ]);
  });
});

describe("grantAllowance", () => {
  it("grant_is_idempotent_by_key", async () => {
    const db = await account(0, 6_000_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    await settleHold(db, id, usage(1_000_000), NOW);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 5_000_000, expires: END1, openHolds: 0 });
    // The same key again, even with a later period, changes nothing.
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_seed", micros: 6_000_000, periodEndUnixMs: END2 }, NOW + 1)).toBe(false);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 5_000_000, expires: END1, openHolds: 0 });
    expect(await grants(db)).toEqual([{ grant_key: "in_seed", account_id: ACCOUNT, micros: 6_000_000, period_end_unix_ms: END1 }]);
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: -1_000_000, from_allowance_micros: 1_000_000 })]);
  });

  it("a_second_grant_in_a_window_replaces_never_adds", async () => {
    const db = await account(0, 6_000_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    await settleHold(db, id, usage(1_000_000), NOW);
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_2", micros: 4_000_000, periodEndUnixMs: END2 }, NOW + 1)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 4_000_000, expires: END2, openHolds: 0 });
    // Only a later period end replaces. A different key for the same period
    // end is the same window: recorded, a no-op, no refill.
    const spend = (await placeHold(db, ACCOUNT, 1_000_000, NOW + 2))!;
    await settleHold(db, spend, usage(1_000_000), NOW + 2);
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_3", micros: 4_000_000, periodEndUnixMs: END2 }, NOW + 3)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 3_000_000, expires: END2, openHolds: 0 });
    expect((await grants(db)).map((g) => g.grant_key)).toEqual(["in_seed", "in_2", "in_3"]);
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toHaveLength(2);
  });

  it("a_late_grant_never_moves_expiry_backwards", async () => {
    const db = await account(0, 6_000_000, END2);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    await settleHold(db, id, usage(1_000_000), NOW);
    // An out-of-order invoice for the earlier period arrives late.
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_old", micros: 6_000_000, periodEndUnixMs: END1 }, NOW + 1)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 5_000_000, expires: END2, openHolds: 0 });
    // Its row is still kept for the audit trail.
    expect(await grants(db)).toEqual([
      { grant_key: "in_seed", account_id: ACCOUNT, micros: 6_000_000, period_end_unix_ms: END2 },
      { grant_key: "in_old", account_id: ACCOUNT, micros: 6_000_000, period_end_unix_ms: END1 },
    ]);
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toHaveLength(1);
  });

  it("a_deleting_account_gets_no_grant", async () => {
    const db = await account(0);
    await db.execute("UPDATE account SET deleting_since_unix_ms = ? WHERE id = ?", [NOW, ACCOUNT]);
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_1", micros: 6_000_000, periodEndUnixMs: END1 }, NOW)).toBe(false);
    expect(await grantAllowance(db, { accountId: "nobody", grantKey: "in_2", micros: 6_000_000, periodEndUnixMs: END1 }, NOW)).toBe(false);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: null, openHolds: 0 });
    expect(await grants(db)).toEqual([]);
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([]);
  });

  it("allowance_setting_falls_back_on_missing_invalid_or_out_of_range", async () => {
    const db = await account(0);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(await readAllowanceSetting(db)).toBe(6_000_000);
    expect(warn).not.toHaveBeenCalled(); // a missing row is the normal default
    // A REAL written into the TEXT column is stored as "7500000.0".
    await db.execute("INSERT OR REPLACE INTO server_setting (key, value) VALUES ('plus_allowance_micros', 7500000.0)", []);
    expect(await readAllowanceSetting(db)).toBe(6_000_000);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toBe(
      'server_setting plus_allowance_micros "7500000.0" is not a whole number in 1..50000000; using 6000000',
    );
    const set = (v: string) => db.execute("INSERT OR REPLACE INTO server_setting (key, value) VALUES ('plus_allowance_micros', ?)", [v]);
    for (const bad of ["", "abc", "6e6", "0", "-5", "1.5", "50000001", "0x10", " 7000000x", "99999999999999999999"]) {
      await set(bad);
      expect(await readAllowanceSetting(db), bad).toBe(6_000_000);
    }
    expect(warn).toHaveBeenCalledTimes(11); // one line per unusable read
    for (const [good, micros] of [["1", 1], ["7500000", 7_500_000], ["50000000", 50_000_000], [" 4000000 ", 4_000_000]] as const) {
      await set(good);
      expect(await readAllowanceSetting(db), good).toBe(micros);
    }
    expect(warn).toHaveBeenCalledTimes(11);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: null, openHolds: 0 });
  });
});

describe("reading and zeroing", () => {
  it("spendable is the live allowance plus prepaid, and the view zeroes an expired allowance", async () => {
    const db = await account(1_000_000, 6_000_000);
    expect(await spendableAllowance(db, ACCOUNT, NOW)).toBe(6_000_000);
    expect(await spendable(db, ACCOUNT, NOW)).toBe(7_000_000);
    expect(await spendableAllowance(db, ACCOUNT, END1)).toBe(0);
    expect(await spendable(db, ACCOUNT, END1)).toBe(1_000_000);
    expect(await spendable(db, "nobody", NOW)).toBe(0);
    expect(await spendableAllowance(db, "nobody", NOW)).toBe(0);
    expect(allowanceView({ aiAllowanceMicros: 6_000_000, aiAllowanceExpiresMs: END1 }, NOW)).toEqual({ allowanceMicros: 6_000_000, allowanceExpiresMs: END1 });
    // Expired: nothing to spend, the stored expiry still reported (null only when none).
    expect(allowanceView({ aiAllowanceMicros: 6_000_000, aiAllowanceExpiresMs: END1 }, END1)).toEqual({ allowanceMicros: 0, allowanceExpiresMs: END1 });
    expect(allowanceView({ aiAllowanceMicros: 0, aiAllowanceExpiresMs: null }, NOW)).toEqual({ allowanceMicros: 0, allowanceExpiresMs: null });
  });

  it("zeroAllowance empties the allowance and an open hold cannot refill it", async () => {
    const db = await account(1_000_000, 6_000_000);
    const id = (await placeHold(db, ACCOUNT, 1_000_000, NOW))!;
    await zeroAllowance(db, ACCOUNT, NOW, LEGACY_REFUND);
    // The expiry moves to the old window end + 1 ms: no hold's window matches it.
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 0, expires: END1 + 1, openHolds: 1 });
    expect(await spendableAllowance(db, ACCOUNT, NOW)).toBe(0);
    await settleHold(db, id, usage(0), NOW);
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 0, expires: END1 + 1, openHolds: 0 });
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: 0, from_allowance_micros: 0 })]);
    // A later subscription's grant still applies.
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_new", micros: 6_000_000, periodEndUnixMs: END2 }, NOW)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 1_000_000, allowance: 6_000_000, expires: END2, openHolds: 0 });
  });

  it("zero_records_one_marker_row_and_a_repeat_in_the_same_ms_is_idempotent", async () => {
    const db = await account(0, 6_000_000);
    await zeroAllowance(db, ACCOUNT, NOW, LEGACY_REFUND);
    await zeroAllowance(db, ACCOUNT, NOW, LEGACY_REFUND);
    const markers = (await grants(db)).filter((g) => String(g.grant_key).startsWith("zeroed:"));
    // One marker; its period end is the first zero's new expiry. The repeat
    // still moved the account's expiry 1 ms further (harmless).
    expect(markers).toEqual([{ grant_key: `zeroed:${ACCOUNT}:${NOW}`, account_id: ACCOUNT, micros: 0, period_end_unix_ms: END1 + 1 }]);
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: END1 + 2 });
    await zeroAllowance(db, ACCOUNT, NOW + 5, LEGACY_REFUND);
    expect((await grants(db)).filter((g) => String(g.grant_key).startsWith("zeroed:")).map((g) => g.grant_key)).toEqual([
      `zeroed:${ACCOUNT}:${NOW}`,
      `zeroed:${ACCOUNT}:${NOW + 5}`,
    ]);
    // A missing account gets neither a zero nor a marker.
    await zeroAllowance(db, "nobody", NOW, LEGACY_REFUND);
    expect((await grants(db)).some((g) => g.account_id === "nobody")).toBe(false);
  });

  it("zero_with_no_allowance_or_an_expired_one_still_moves_the_expiry_past_now", async () => {
    const none = await account(0);
    await zeroAllowance(none, ACCOUNT, NOW, LEGACY_REFUND);
    expect(await buckets(none)).toEqual({ prepaid: 0, allowance: 0, expires: NOW + 1, openHolds: 0 });
    const expired = await account(0, 6_000_000);
    await zeroAllowance(expired, ACCOUNT, END1 + 1000, LEGACY_REFUND);
    expect(await buckets(expired)).toEqual({ prepaid: 0, allowance: 0, expires: END1 + 1001, openHolds: 0 });
    for (const db of [none, expired]) {
      expect(await holds(db)).toEqual([]);
      expect(await usageRows(db)).toEqual([]);
    }
  });

  it("zero_then_a_later_period_grants_in_full_and_an_old_hold_never_tops_it_up", async () => {
    // A genuinely later period end (END2 > END1 + 1) after a zero grants the
    // full configured amount; an old open hold from END1 settling afterwards
    // returns nothing to it.
    const db = await account(0, 600_000);
    const id = (await placeHold(db, ACCOUNT, 500_000, NOW))!; // all allowance, window END1
    await zeroAllowance(db, ACCOUNT, NOW + 1, LEGACY_REFUND);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: END1 + 1, openHolds: 1 });
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_new", micros: 6_000_000, periodEndUnixMs: END2 }, NOW + 2)).toBe(true);
    await settleHold(db, id, usage(0), NOW + 3);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 6_000_000, expires: END2, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: 0, from_allowance_micros: 0 })]);
  });

  it("zero_then_a_late_grant_ending_inside_the_old_window_is_refused", async () => {
    // Reviewer case 2 (probe P2): after a zero, a late grant for a period
    // ending after the zero but before the old window end changes nothing.
    const db = await account(0, 6_000_000);
    await zeroAllowance(db, ACCOUNT, NOW + 100, LEGACY_REFUND);
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_p2", micros: 6_000_000, periodEndUnixMs: END1 - DAY }, NOW + 200)).toBe(true);
    // Already ended by the caller's clock: refused outright, nothing written.
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_late", micros: 6_000_000, periodEndUnixMs: NOW + 100 }, NOW + 100)).toBe(false);
    // A caller clock behind the zero changes nothing either.
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_skew", micros: 6_000_000, periodEndUnixMs: NOW + 50 }, NOW)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: END1 + 1, openHolds: 0 });
    // Late grants that reached the insert are kept for the audit trail only.
    // The zero leaves its marker row (micros 0, period end = the new expiry).
    expect((await grants(db)).map((g) => g.grant_key)).toEqual(["in_seed", `zeroed:${ACCOUNT}:${NOW + 100}`, "in_p2", "in_skew"]);
    expect((await grants(db))[1]).toMatchObject({ micros: 0, period_end_unix_ms: END1 + 1 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([]);
  });

  it("zero_then_a_grant_ending_at_the_old_window_end_is_refused_and_the_old_hold_cannot_refill", async () => {
    // Reviewer case 1 (probe P1): re-granting the zeroed period itself.
    const db = await account(0, 600_000);
    const id = (await placeHold(db, ACCOUNT, 500_000, NOW))!; // window END1
    await zeroAllowance(db, ACCOUNT, NOW + 1, LEGACY_REFUND);
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_again", micros: 6_000_000, periodEndUnixMs: END1 }, NOW + 2)).toBe(true);
    // One ms past the old end is still at or before the zero's expiry.
    expect(await grantAllowance(db, { accountId: ACCOUNT, grantKey: "in_again_1", micros: 6_000_000, periodEndUnixMs: END1 + 1 }, NOW + 2)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: END1 + 1, openHolds: 1 });
    await settleHold(db, id, usage(0), NOW + 3);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: END1 + 1, openHolds: 0 });
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([expect.objectContaining({ micros: 0, from_allowance_micros: 0 })]);
  });

  it("a_grant_of_no_micros_or_for_an_ended_period_is_refused", async () => {
    const db = await account(0);
    const grant = (micros: number, periodEndUnixMs: number, key: string) =>
      grantAllowance(db, { accountId: ACCOUNT, grantKey: key, micros, periodEndUnixMs }, NOW);
    expect(await grant(0, END1, "in_zero")).toBe(false);
    expect(await grant(-1, END1, "in_neg")).toBe(false);
    expect(await grant(6_000_000, NOW, "in_now")).toBe(false);
    expect(await grant(6_000_000, NOW - 1, "in_past")).toBe(false);
    await expect(grant(1.5, END1, "in_frac")).rejects.toThrow(RangeError);
    await expect(grant(6_000_000, END1 + 0.5, "in_frac_end")).rejects.toThrow(RangeError);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: null, openHolds: 0 });
    expect(await grants(db)).toEqual([]);
    expect(await holds(db)).toEqual([]);
    expect(await usageRows(db)).toEqual([]);
    // The keys were not used up: a valid grant under one still applies.
    expect(await grant(6_000_000, END1, "in_zero")).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 6_000_000, expires: END1, openHolds: 0 });
  });
});

describe("zeros scoped to a subscription", () => {
  const A = { subscriptionId: "sub_A", mode: "refund" } as const;
  const grantFor = (db: SqlExecutor, key: string, end: number, sub: string | undefined, now: number) =>
    grantAllowance(db, { accountId: ACCOUNT, grantKey: key, micros: 6_000_000, periodEndUnixMs: end, ...(sub ? { subscriptionId: sub } : {}) }, now);
  const subOf = async (db: SqlExecutor, key: string) => (await db.execute("SELECT subscription_id FROM ai_allowance_grant WHERE grant_key = ?", [key]))[0]?.subscription_id;

  it("a_cancel_zero_sets_the_expiry_to_now_plus_one_and_an_open_hold_cannot_refill", async () => {
    const db = await account(0, 600_000);
    const id = (await placeHold(db, ACCOUNT, 500_000, NOW))!; // window END1
    await zeroAllowance(db, ACCOUNT, NOW + 10, { subscriptionId: "sub_A", mode: "cancel" });
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: NOW + 11, openHolds: 1 });
    expect((await grants(db)).at(-1)).toEqual({ grant_key: `zeroed:${ACCOUNT}:${NOW + 10}`, account_id: ACCOUNT, micros: 0, period_end_unix_ms: NOW + 11 });
    expect(await subOf(db, `zeroed:${ACCOUNT}:${NOW + 10}`)).toBe("sub_A");
    // A repeat in the same ms changes nothing.
    await zeroAllowance(db, ACCOUNT, NOW + 10, { subscriptionId: "sub_A", mode: "cancel" });
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: NOW + 11 });
    await settleHold(db, id, usage(0), NOW + 20);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 0, expires: NOW + 11, openHolds: 0 });
    // A new subscription's window ending before the old one is a later end than now + 1: it grants.
    expect(await grantFor(db, "in_B", END1 - DAY, "sub_B", NOW + 30)).toBe(true);
    expect(await buckets(db)).toEqual({ prepaid: 0, allowance: 6_000_000, expires: END1 - DAY, openHolds: 0 });
  });

  it("after_a_refund_zero_only_another_subscriptions_grant_lands_inside_the_window_and_only_once", async () => {
    const db = await account(0, 6_000_000);
    await zeroAllowance(db, ACCOUNT, NOW, A);
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: END1 + 1 });
    // The zeroed subscription itself, or a grant without a subscription id: refused (row kept).
    expect(await grantFor(db, "in_A2", END1 - DAY, "sub_A", NOW + 1)).toBe(true);
    expect(await grantFor(db, "in_none", END1 - DAY, undefined, NOW + 1)).toBe(true);
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: END1 + 1 });
    // Another subscription: lands although it ends before the zeroed window, and records its subscription.
    expect(await grantFor(db, "in_B", END1 - DAY, "sub_B", NOW + 2)).toBe(true);
    expect(await buckets(db)).toMatchObject({ allowance: 6_000_000, expires: END1 - DAY });
    expect(await subOf(db, "in_B")).toBe("sub_B");
    // Used up: spent to 0, another grant of B (or of a third subscription) for an earlier or equal end is a no-op.
    const hold = (await placeHold(db, ACCOUNT, 6_000_000, NOW + 3))!;
    await settleHold(db, hold, usage(6_000_000), NOW + 3);
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: END1 - DAY });
    expect(await grantFor(db, "in_B2", END1 - DAY, "sub_B", NOW + 4)).toBe(true);
    expect(await grantFor(db, "in_C", END1 - 2 * DAY, "sub_C", NOW + 4)).toBe(true);
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: END1 - DAY });
    // B's next window (later) grants as usual.
    expect(await grantFor(db, "in_B3", END2, "sub_B", NOW + 5)).toBe(true);
    expect(await buckets(db)).toMatchObject({ allowance: 6_000_000, expires: END2 });
  });

  it("a_repeat_refund_zero_in_the_same_ms_still_lets_another_subscription_land", async () => {
    const db = await account(0, 6_000_000);
    await zeroAllowance(db, ACCOUNT, NOW, A);
    await zeroAllowance(db, ACCOUNT, NOW, A);
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: END1 + 2 }); // the marker still says END1 + 1
    expect(await grantFor(db, "in_B", END1 - DAY, "sub_B", NOW + 1)).toBe(true);
    expect(await buckets(db)).toMatchObject({ allowance: 6_000_000, expires: END1 - DAY });
  });

  it("a_live_window_of_another_subscription_is_never_replaced_by_an_earlier_grant", async () => {
    // No zero: the plain guard, whatever the subscriptions (a kept duplicate's grant is a no-op).
    const db = await account(0);
    expect(await grantFor(db, "in_2", END2, "sub_2", NOW)).toBe(true);
    const hold = (await placeHold(db, ACCOUNT, 6_000_000, NOW + 1))!;
    await settleHold(db, hold, usage(6_000_000), NOW + 1);
    expect(await grantFor(db, "in_1", END1, "sub_1", NOW + 2)).toBe(true);
    expect(await buckets(db)).toMatchObject({ allowance: 0, expires: END2 });
  });

  it("a_refund_tombstone_makes_the_invoices_grant_a_repeat_key", async () => {
    const db = await account(0);
    await zeroAllowance(db, ACCOUNT, NOW, { subscriptionId: "sub_A", mode: "refund", tombstoneKey: "in_A" });
    expect((await grants(db)).find((g) => g.grant_key === "in_A")).toEqual({ grant_key: "in_A", account_id: ACCOUNT, micros: 0, period_end_unix_ms: 0 });
    expect(await subOf(db, "in_A")).toBe("sub_A");
    expect(await grantFor(db, "in_A", END1, "sub_A", NOW + 1)).toBe(false);
    expect(await buckets(db)).toMatchObject({ allowance: 0 });
    // A repeat with the tombstone already there is harmless.
    await zeroAllowance(db, ACCOUNT, NOW + 1, { subscriptionId: "sub_A", mode: "refund", tombstoneKey: "in_A" });
    expect((await grants(db)).filter((g) => g.grant_key === "in_A")).toHaveLength(1);
  });
});
