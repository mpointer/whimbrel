import { it, expect } from "vitest";
import * as accounts from "../src/accounts";
import {
  STALE_HOLD_MS,
  aiBalance,
  allowanceView,
  creditPurchase,
  grantAllowance,
  readAllowanceSetting,
  spendable,
  spendableAllowance,
  zeroAllowance,
  lastPurchaseMicros,
  addMonthsUtc,
  grantDueYearlyMonths,
  yearlyMonthAt,
  monthWindowEnd,
  purchaseByStripeRef,
  placeHold,
  refundPurchase,
  settleHold,
  settleStaleHolds,
} from "../src/aiCredit";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";

const NOW = Date.UTC(2026, 10, 10, 9, 0, 0);

/** A fresh account funded through the real purchase path. */
async function funded(micros: number) {
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "a@example.com", "sub-a", 1, "db-a", "https://db-a");
  if (micros > 0) expect(await creditPurchase(db, account.id, micros, "cs_seed", NOW)).toBe(true);
  return { db, id: account.id };
}

it("placeHold_debits_and_refuses_when_short", async () => {
  const { db, id } = await funded(1_000_000);
  const first = await placeHold(db, id, 600_000, NOW);
  expect(first).toEqual(expect.any(String));
  expect(await aiBalance(db, id)).toBe(400_000);
  expect(await placeHold(db, id, 600_000, NOW)).toBeNull();
  expect(await aiBalance(db, id)).toBe(400_000);
  expect(db.rawAiHolds()).toHaveLength(1);
});

it("parallel_holds_never_overdraw", async () => {
  const { db, id } = await funded(2_000_000);
  const results = await Promise.all(Array.from({ length: 10 }, () => placeHold(db, id, 900_000, NOW)));
  expect(results.filter((r) => r !== null)).toHaveLength(2);
  const balance = await aiBalance(db, id);
  expect(balance).toBeGreaterThanOrEqual(0);
  expect(balance).toBe(200_000);
  expect(db.rawAiHolds()).toHaveLength(2);
});

it("fifth_concurrent_hold_is_refused", async () => {
  const { db, id } = await funded(10_000_000);
  const results = await Promise.all(Array.from({ length: 6 }, () => placeHold(db, id, 100_000, NOW)));
  expect(results.filter((r) => r !== null)).toHaveLength(4);
  expect(await aiBalance(db, id)).toBe(9_600_000);
  expect(db.rawAiHolds()).toHaveLength(4);
  expect(db.rawAccount(id)?.aiOpenHolds).toBe(4);
});

it("a settled hold frees its slot", async () => {
  const { db, id } = await funded(10_000_000);
  const holds: string[] = [];
  for (let i = 0; i < 4; i++) holds.push((await placeHold(db, id, 100_000, NOW))!);
  await settleHold(db, holds[0], null, NOW);
  expect(db.rawAccount(id)?.aiOpenHolds).toBe(3);
  expect(await placeHold(db, id, 100_000, NOW)).not.toBeNull();
});

it("settleHold_charges_real_cost", async () => {
  const start = 1_000_000;
  const { db, id } = await funded(start);
  const hold = (await placeHold(db, id, 600_000, NOW))!;
  await settleHold(db, hold, { model: "claude-sonnet-4-5", inputTokens: 1200, outputTokens: 340, costMicros: 130_100 }, NOW + 5);
  expect(await aiBalance(db, id)).toBe(start - 130_100);
  expect(db.rawAiHolds()).toHaveLength(0);
  expect(db.rawAccount(id)?.aiOpenHolds).toBe(0);
  const usage = db.rawAiLedger().filter((r) => r.kind === "usage");
  expect(usage).toHaveLength(1);
  expect(usage[0]).toEqual({
    id: expect.any(String),
    account_id: id,
    kind: "usage",
    micros: -130_100,
    model: "claude-sonnet-4-5",
    input_tokens: 1200,
    output_tokens: 340,
    stripe_ref: null,
    created_at_unix_ms: NOW + 5,
    shortfall_micros: null,
    from_allowance_micros: 0,
  });
});

it("settle_cost_above_hold_never_goes_below_zero", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 600_000, NOW))!;
  // Hold 600,000 + balance 400,000 = 1,000,000 available; the real cost is more.
  await settleHold(db, hold, { model: "m", inputTokens: 9, outputTokens: 9, costMicros: 1_500_000 }, NOW);
  expect(await aiBalance(db, id)).toBe(0);
  expect(db.rawAiLedger().filter((r) => r.kind === "usage")).toEqual([expect.objectContaining({ micros: -1_500_000 })]);
});

it("settle cost above the hold but within the balance takes the full cost", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 100_000, NOW))!;
  await settleHold(db, hold, { model: "m", inputTokens: 9, outputTokens: 9, costMicros: 250_000 }, NOW);
  expect(await aiBalance(db, id)).toBe(750_000);
});

it("settleHold_without_usage_keeps_the_hold_as_charge", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 600_000, NOW))!;
  await settleHold(db, hold, null, NOW);
  expect(await aiBalance(db, id)).toBe(400_000);
  expect(db.rawAiHolds()).toHaveLength(0);
  const usage = db.rawAiLedger().filter((r) => r.kind === "usage");
  expect(usage).toHaveLength(1);
  expect(usage[0]).toMatchObject({ micros: -600_000, model: null, input_tokens: null, output_tokens: null });
});

it("settling the same hold twice charges once", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 600_000, NOW))!;
  await settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: 100 }, NOW);
  await settleHold(db, hold, null, NOW);
  expect(await aiBalance(db, id)).toBe(999_900);
  expect(db.rawAiLedger().filter((r) => r.kind === "usage")).toHaveLength(1);
});

it("settleStaleHolds_closes_old_holds", async () => {
  const { db, id } = await funded(1_000_000);
  const old = (await placeHold(db, id, 300_000, NOW - STALE_HOLD_MS - 1))!;
  const fresh = (await placeHold(db, id, 200_000, NOW - 1000))!;
  expect(await settleStaleHolds(db, NOW)).toBe(1);
  expect(db.rawAiHolds().map((h) => h.id)).toEqual([fresh]);
  expect(await aiBalance(db, id)).toBe(500_000);
  const usage = db.rawAiLedger().filter((r) => r.kind === "usage");
  expect(usage).toHaveLength(1);
  expect(usage[0]).toMatchObject({ micros: -300_000, model: null });
  expect(old).not.toBe(fresh);
});

it("creditPurchase is idempotent per stripe ref", async () => {
  const { db, id } = await funded(0);
  expect(await creditPurchase(db, id, 10_000_000, "cs_1", NOW)).toBe(true);
  expect(await creditPurchase(db, id, 10_000_000, "cs_1", NOW)).toBe(false);
  expect(await aiBalance(db, id)).toBe(10_000_000);
  expect(db.rawAiLedger()).toEqual([
    expect.objectContaining({ kind: "purchase", micros: 10_000_000, stripe_ref: "cs_1", account_id: id }),
  ]);
});

it("refundPurchase removes at most the balance, once per refund id", async () => {
  const { db, id } = await funded(10_000_000);
  const hold = (await placeHold(db, id, 4_000_000, NOW))!;
  await settleHold(db, hold, null, NOW);
  expect(await refundPurchase(db, id, 10_000_000, "re_1", NOW)).toBe(6_000_000);
  expect(await aiBalance(db, id)).toBe(0);
  expect(await refundPurchase(db, id, 10_000_000, "re_1", NOW)).toBe(0);
  expect(db.rawAiLedger().filter((r) => r.kind === "refund")).toEqual([
    expect.objectContaining({ micros: -6_000_000, stripe_ref: "refund:re_1", shortfall_micros: 4_000_000 }),
  ]);
});

it("two partial refunds of one purchase each apply once", async () => {
  const { db, id } = await funded(25_000_000);
  expect(await refundPurchase(db, id, 5_000_000, "re_1", NOW)).toBe(5_000_000);
  expect(await refundPurchase(db, id, 5_000_000, "re_2", NOW)).toBe(5_000_000);
  expect(await refundPurchase(db, id, 5_000_000, "re_2", NOW)).toBe(0);
  expect(await aiBalance(db, id)).toBe(15_000_000);
});

it("a refund with no balance left removes 0 and still records the refund id", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 1_000_000, NOW))!;
  expect(await refundPurchase(db, id, 1_000_000, "re_1", NOW)).toBe(0);
  await settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: 100 }, NOW);
  expect(await refundPurchase(db, id, 1_000_000, "re_1", NOW)).toBe(0);
  expect(await aiBalance(db, id)).toBe(999_900);
});

it("every money path is one batch", async () => {
  const { db, id } = await funded(1_000_000);
  const before = db.batches;
  const hold = (await placeHold(db, id, 100_000, NOW))!;
  await settleHold(db, hold, null, NOW);
  await creditPurchase(db, id, 1, "cs_2", NOW);
  await refundPurchase(db, id, 1, "re_9", NOW);
  expect(db.batches - before).toBe(4);
});

it("a batch that fails partway applies nothing", async () => {
  const { db, id } = await funded(1_000_000);
  await expect(
    db.batch([
      { sql: "UPDATE account SET ai_credit_micros = ai_credit_micros + ? WHERE id = ? AND EXISTS (SELECT 1 FROM ai_ledger WHERE id = ?)", params: [5, id, db.rawAiLedger()[0].id] },
      { sql: "SELECT no_such_thing", params: [] },
    ]),
  ).rejects.toThrow(/unrecognized query/);
  expect(await aiBalance(db, id)).toBe(1_000_000);
});

it("deleting an account forfeits its credit, ledger and holds", async () => {
  const { db, id } = await funded(1_000_000);
  await placeHold(db, id, 100_000, NOW);
  await accounts.deleteAccountRows(db, id);
  expect(db.rawAiHolds()).toHaveLength(0);
  expect(db.rawAiLedger()).toHaveLength(0);
});

it("aiBalance of an unknown account is 0", async () => {
  expect(await aiBalance(new FakeSqlExecutor(), "nobody")).toBe(0);
});

it("a negative or zero hold throws", async () => {
  const { db, id } = await funded(1_000_000);
  await expect(placeHold(db, id, -100, NOW)).rejects.toThrow(RangeError);
  await expect(placeHold(db, id, 0, NOW)).rejects.toThrow(RangeError);
  await expect(placeHold(db, id, 1.5, NOW)).rejects.toThrow(RangeError);
  expect(await aiBalance(db, id)).toBe(1_000_000);
});

it("a fractional or negative cost throws and leaves the hold open", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 600_000, NOW))!;
  await expect(settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: 130_100.5 }, NOW)).rejects.toThrow(RangeError);
  await expect(settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: -1 }, NOW)).rejects.toThrow(RangeError);
  expect(db.rawAiHolds()).toHaveLength(1);
  expect(await aiBalance(db, id)).toBe(400_000);
});

it("bad purchase and refund amounts throw", async () => {
  const { db, id } = await funded(1_000_000);
  await expect(creditPurchase(db, id, -1, "cs_bad", NOW)).rejects.toThrow(RangeError);
  await expect(creditPurchase(db, id, Number.MAX_SAFE_INTEGER + 2, "cs_bad", NOW)).rejects.toThrow(RangeError);
  await expect(refundPurchase(db, id, 0.5, "re_bad", NOW)).rejects.toThrow(RangeError);
  await expect(refundPurchase(db, id, Number.NaN, "re_bad", NOW)).rejects.toThrow(RangeError);
  expect(await aiBalance(db, id)).toBe(1_000_000);
});

it("a cost of 130,101 is debited as 130,200", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 600_000, NOW))!;
  await settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: 130_101 }, NOW);
  expect(await aiBalance(db, id)).toBe(1_000_000 - 130_200);
  expect(db.rawAiLedger().filter((r) => r.kind === "usage")).toEqual([expect.objectContaining({ micros: -130_200 })]);
});

it("a hold amount rounds up to 100 micros", async () => {
  const { db, id } = await funded(1_000_000);
  const hold = (await placeHold(db, id, 1, NOW))!;
  expect(await aiBalance(db, id)).toBe(999_900);
  expect(db.rawAiHolds()).toEqual([expect.objectContaining({ id: hold, micros: 100 })]);
});

it("a refund that meets held credit records the shortfall", async () => {
  const { db, id } = await funded(1_000_000);
  await placeHold(db, id, 700_000, NOW);
  expect(await refundPurchase(db, id, 1_000_000, "re_1", NOW)).toBe(300_000);
  expect(db.rawAiLedger().filter((r) => r.kind === "refund")).toEqual([
    expect.objectContaining({ micros: -300_000, shortfall_micros: 700_000 }),
  ]);
});

it("lastPurchaseMicros is the newest purchase, 0 when none", async () => {
  const { db, id } = await funded(0);
  expect(await lastPurchaseMicros(db, id)).toBe(0);
  await creditPurchase(db, id, 25_000_000, "cs_1", NOW);
  await creditPurchase(db, id, 10_000_000, "cs_2", NOW + 1);
  const hold = (await placeHold(db, id, 1_000_000, NOW + 2))!;
  await settleHold(db, hold, null, NOW + 3);
  expect(await lastPurchaseMicros(db, id)).toBe(10_000_000);
});

it("purchaseByStripeRef finds the purchase, null otherwise", async () => {
  const { db, id } = await funded(0);
  await creditPurchase(db, id, 10_000_000, "pi_1", NOW);
  expect(await purchaseByStripeRef(db, "pi_1")).toEqual({ accountId: id, micros: 10_000_000 });
  expect(await purchaseByStripeRef(db, "pi_none")).toBeNull();
});

// ---- Plus allowance: fake twins of the SQL that aiCredit.realsql.test.ts
// runs against real SQLite. These keep callers built on the fake honest;
// the real-SQLite tests are what prove the SQL.

const END = NOW + 30 * 24 * 60 * 60 * 1000;

async function plus(prepaid: number, allowance: number) {
  const { db, id } = await funded(prepaid);
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_1", micros: allowance, periodEndUnixMs: END }, NOW)).toBe(true);
  return { db, id };
}

it("fake: a hold draws the allowance first and the settle charges each bucket", async () => {
  const { db, id } = await plus(1_000_000, 600_000);
  const hold = (await placeHold(db, id, 1_000_000, NOW))!;
  expect(db.rawAccount(id)).toMatchObject({ aiCreditMicros: 600_000, aiAllowanceMicros: 0, aiOpenHolds: 1 });
  expect(db.rawAiHolds()).toEqual([expect.objectContaining({ id: hold, micros: 1_000_000, from_allowance_micros: 600_000, allowance_expires_ms: END })]);
  await settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: 700_050 }, NOW);
  expect(db.rawAccount(id)).toMatchObject({ aiCreditMicros: 899_900, aiAllowanceMicros: 0, aiOpenHolds: 0 });
  expect(db.rawAiLedger().filter((r) => r.kind === "usage")).toEqual([expect.objectContaining({ micros: -700_100, from_allowance_micros: 600_000 })]);
  expect(db.rawAiHolds()).toHaveLength(0);
});

it("fake: a renewal during an open hold is not topped up, and a stale hold charges per bucket", async () => {
  const { db, id } = await plus(1_000_000, 6_000_000);
  const hold = (await placeHold(db, id, 1_000_000, NOW))!;
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_2", micros: 6_000_000, periodEndUnixMs: END * 2 }, NOW)).toBe(true);
  await settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: 0 }, NOW);
  expect(db.rawAccount(id)).toMatchObject({ aiCreditMicros: 1_000_000, aiAllowanceMicros: 6_000_000, aiAllowanceExpiresMs: END * 2 });
  const stale = (await placeHold(db, id, 6_500_000, NOW - STALE_HOLD_MS - 1))!;
  expect(await settleStaleHolds(db, NOW)).toBe(1);
  expect(db.rawAccount(id)).toMatchObject({ aiCreditMicros: 500_000, aiAllowanceMicros: 0, aiOpenHolds: 0 });
  expect(db.rawAiLedger().filter((r) => r.kind === "usage").at(-1)).toMatchObject({ micros: -6_500_000, from_allowance_micros: 6_000_000 });
  expect(stale).toEqual(expect.any(String));
});

it("fake: grants are once per key, forward only, and never for a deleting account", async () => {
  const { db, id } = await plus(0, 6_000_000);
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_1", micros: 6_000_000, periodEndUnixMs: END * 2 }, NOW)).toBe(false);
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_old", micros: 1, periodEndUnixMs: END - 1 }, NOW)).toBe(true);
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 6_000_000, aiAllowanceExpiresMs: END });
  expect(db.rawAllowanceGrants().map((g) => g.grant_key)).toEqual(["in_1", "in_old"]);
  expect(await spendableAllowance(db, id, NOW)).toBe(6_000_000);
  expect(await spendable(db, id, NOW)).toBe(6_000_000);
  expect(await spendable(db, id, END)).toBe(0);
  expect(allowanceView(db.rawAccount(id) as { aiAllowanceMicros: number; aiAllowanceExpiresMs: number | null }, NOW)).toEqual({ allowanceMicros: 6_000_000, allowanceExpiresMs: END });
  await zeroAllowance(db, id, NOW, { subscriptionId: null, mode: "refund" });
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 0, aiAllowanceExpiresMs: END + 1 });
  // The refunded period itself cannot be granted again.
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_again", micros: 6_000_000, periodEndUnixMs: END }, NOW)).toBe(true);
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 0, aiAllowanceExpiresMs: END + 1 });
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_zero", micros: 0, periodEndUnixMs: END }, NOW)).toBe(false);
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_past", micros: 1, periodEndUnixMs: NOW }, NOW)).toBe(false);
  const other = await accounts.getOrCreateAccount(db, "b@example.com", "sub-b", 1, "db-b", "https://db-b");
  db.rawAccount(other.id)!.deletingSinceUnixMs = NOW;
  expect(await grantAllowance(db, { accountId: other.id, grantKey: "in_b", micros: 6_000_000, periodEndUnixMs: END }, NOW)).toBe(false);
  expect(await grantAllowance(db, { accountId: "nobody", grantKey: "in_c", micros: 6_000_000, periodEndUnixMs: END }, NOW)).toBe(false);
  expect(db.rawAllowanceGrants().map((g) => g.grant_key)).toEqual(["in_1", "in_old", `zeroed:${id}:${NOW}`, "in_again"]);
  expect(db.rawAllowanceGrants()[2]).toMatchObject({ account_id: id, micros: 0, period_end_unix_ms: END + 1, created_at_unix_ms: NOW });
});

it("fake: a cancel zero, another subscription after a refund zero, and a refund tombstone", async () => {
  const { db, id } = await plus(0, 6_000_000);
  await zeroAllowance(db, id, NOW, { subscriptionId: "sub_A", mode: "refund", tombstoneKey: "in_A" });
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 0, aiAllowanceExpiresMs: END + 1 });
  expect(db.rawAllowanceGrants().slice(1)).toEqual([
    expect.objectContaining({ grant_key: `zeroed:${id}:${NOW}`, micros: 0, period_end_unix_ms: END + 1, subscription_id: "sub_A" }),
    expect.objectContaining({ grant_key: "in_A", micros: 0, period_end_unix_ms: 0, subscription_id: "sub_A" }),
  ]);
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_A", micros: 6_000_000, periodEndUnixMs: END, subscriptionId: "sub_A" }, NOW)).toBe(false);
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_A2", micros: 6_000_000, periodEndUnixMs: END - 1000, subscriptionId: "sub_A" }, NOW)).toBe(true);
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 0, aiAllowanceExpiresMs: END + 1 });
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_B", micros: 6_000_000, periodEndUnixMs: END - 1000, subscriptionId: "sub_B" }, NOW)).toBe(true);
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 6_000_000, aiAllowanceExpiresMs: END - 1000 });
  expect(db.rawAllowanceGrants().at(-1)).toMatchObject({ grant_key: "in_B", subscription_id: "sub_B" });
  await zeroAllowance(db, id, NOW + 5, { subscriptionId: "sub_B", mode: "cancel" });
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 0, aiAllowanceExpiresMs: NOW + 6 });
});

it("fake: the yearly cron's SELECT, scoped to the current subscription and its own markers", async () => {
  const { db, id } = await funded(0);
  const start = NOW;
  const yearEnd = addMonthsUtc(start, 12);
  const month1End = addMonthsUtc(start, 1);
  db.setPlan(id, { plan: "cloud", planUntilUnixMs: yearEnd, planInterval: "year", stripeSubscriptionId: "sub_B", planTier: "plus" });
  // An earlier subscription's row and refund marker, both later than B's month 1: neither counts.
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_A", micros: 6_000_000, periodEndUnixMs: month1End + 1000, subscriptionId: "sub_A" }, NOW)).toBe(true);
  await zeroAllowance(db, id, NOW, { subscriptionId: "sub_A", mode: "refund" });
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_B", micros: 6_000_000, periodEndUnixMs: month1End, lineStartUnixMs: start, lineEndUnixMs: yearEnd, subscriptionId: "sub_B" }, NOW + 1)).toBe(true);
  expect(db.rawAccount(id)).toMatchObject({ aiAllowanceMicros: 6_000_000, aiAllowanceExpiresMs: month1End });
  expect(await grantDueYearlyMonths(db, month1End + 1000)).toBe(1);
  expect(db.rawAllowanceGrants().at(-1)).toMatchObject({ grant_key: "in_B:m2", subscription_id: "sub_B", period_end_unix_ms: addMonthsUtc(start, 2) });
  // B's own cancel marker (written after B's base grant) blocks the next month.
  await zeroAllowance(db, id, month1End + 2000, { subscriptionId: "sub_B", mode: "cancel" });
  expect(await grantDueYearlyMonths(db, addMonthsUtc(start, 2) + 1000)).toBe(0);
});

it("fake: deleting an account removes its allowance grants and only its own", async () => {
  const { db, id } = await plus(0, 6_000_000);
  const other = await accounts.getOrCreateAccount(db, "b@example.com", "sub-b", 1, "db-b", "https://db-b");
  expect(await grantAllowance(db, { accountId: other.id, grantKey: "in_b", micros: 6_000_000, periodEndUnixMs: END }, NOW)).toBe(true);
  await accounts.deleteAccountRows(db, id);
  expect(db.rawAllowanceGrants()).toEqual([expect.objectContaining({ grant_key: "in_b", account_id: other.id })]);
});

it("fake: the allowance setting falls back on missing, invalid or out of range", async () => {
  const db = new FakeSqlExecutor();
  expect(await readAllowanceSetting(db)).toBe(6_000_000);
  db.setSetting("plus_allowance_micros", "6e6");
  expect(await readAllowanceSetting(db)).toBe(6_000_000);
  db.setSetting("plus_allowance_micros", "50000001");
  expect(await readAllowanceSetting(db)).toBe(6_000_000);
  db.setSetting("plus_allowance_micros", "7500000.0");
  expect(await readAllowanceSetting(db)).toBe(6_000_000);
  db.setSetting("plus_allowance_micros", "7500000");
  expect(await readAllowanceSetting(db)).toBe(7_500_000);
});

it("fake: a grant batch that fails partway restores the grant rows", async () => {
  const { db, id } = await plus(0, 6_000_000);
  await expect(
    db.batch([
      { sql: "INSERT INTO ai_allowance_grant (grant_key, account_id, micros, period_end_unix_ms, created_at_unix_ms) SELECT ?, id, ?, ?, ? FROM account WHERE id = ? AND deleting_since_unix_ms IS NULL ON CONFLICT(grant_key) DO NOTHING RETURNING grant_key", params: ["in_x", 1, END * 2, NOW, id] },
      { sql: "SELECT no_such_thing", params: [] },
    ]),
  ).rejects.toThrow(/unrecognized query/);
  expect(db.rawAllowanceGrants().map((g) => g.grant_key)).toEqual(["in_1"]);
});

// ---- monthWindowEnd: calendar months in UTC, clamped ----
const FAR = Date.UTC(2100, 0, 1);
it("monthWindowEnd_adds_calendar_months_keeping_day_and_time", () => {
  expect(monthWindowEnd(Date.UTC(2026, 10, 19, 8, 59, 0, 123), 1, FAR)).toBe(Date.UTC(2026, 11, 19, 8, 59, 0, 123));
  expect(monthWindowEnd(Date.UTC(2026, 11, 15, 23, 30), 1, FAR)).toBe(Date.UTC(2027, 0, 15, 23, 30));
  expect(monthWindowEnd(Date.UTC(2026, 0, 10), 12, FAR)).toBe(Date.UTC(2027, 0, 10));
  expect(monthWindowEnd(Date.UTC(2026, 0, 10), 0, FAR)).toBe(Date.UTC(2026, 0, 10));
});
it("monthWindowEnd_from_the_31st_clamps_to_the_last_day_of_a_shorter_month", () => {
  const jan31 = Date.UTC(2026, 0, 31, 10, 0);
  expect(monthWindowEnd(jan31, 1, FAR)).toBe(Date.UTC(2026, 1, 28, 10, 0));
  expect(monthWindowEnd(jan31, 2, FAR)).toBe(Date.UTC(2026, 2, 31, 10, 0));
  expect(monthWindowEnd(jan31, 3, FAR)).toBe(Date.UTC(2026, 3, 30, 10, 0));
  expect(monthWindowEnd(Date.UTC(2026, 9, 31), 1, FAR)).toBe(Date.UTC(2026, 10, 30));
});
it("monthWindowEnd_handles_february_in_a_leap_year", () => {
  expect(monthWindowEnd(Date.UTC(2028, 0, 31, 6), 1, FAR)).toBe(Date.UTC(2028, 1, 29, 6));
  expect(monthWindowEnd(Date.UTC(2028, 1, 29), 12, FAR)).toBe(Date.UTC(2029, 1, 28));
  expect(monthWindowEnd(Date.UTC(2027, 1, 28), 1, FAR)).toBe(Date.UTC(2027, 2, 28));
});
it("monthWindowEnd_never_passes_the_period_end", () => {
  const start = Date.UTC(2026, 2, 31);
  const yearEnd = Date.UTC(2027, 2, 31);
  expect(monthWindowEnd(start, 12, yearEnd)).toBe(yearEnd);
  expect(monthWindowEnd(start, 13, yearEnd)).toBe(yearEnd);
  expect(monthWindowEnd(Date.UTC(2026, 0, 1), 1, Date.UTC(2026, 0, 20))).toBe(Date.UTC(2026, 0, 20));
});
// ---- addMonthsUtc: the yearly cron's start of year (yearEnd - 12 months) ----
it("addMonthsUtc_matches_monthWindowEnd_going_forward", () => {
  for (const start of [Date.UTC(2026, 0, 31, 10), Date.UTC(2026, 10, 19, 8, 59, 0, 123), Date.UTC(2028, 1, 29), Date.UTC(2027, 7, 30, 23, 59, 59, 999)]) {
    for (let n = 0; n <= 13; n++) expect(addMonthsUtc(start, n)).toBe(monthWindowEnd(start, n, FAR));
  }
});
it("addMonthsUtc_back_twelve_months_from_the_31st_30th_and_february", () => {
  expect(addMonthsUtc(Date.UTC(2027, 0, 31, 10), -12)).toBe(Date.UTC(2026, 0, 31, 10));
  expect(addMonthsUtc(Date.UTC(2027, 3, 30, 10), -12)).toBe(Date.UTC(2026, 3, 30, 10));
  expect(addMonthsUtc(Date.UTC(2027, 2, 31), -1)).toBe(Date.UTC(2027, 1, 28));
  expect(addMonthsUtc(Date.UTC(2028, 2, 31), -1)).toBe(Date.UTC(2028, 1, 29));
  expect(addMonthsUtc(Date.UTC(2027, 1, 28, 9), -12)).toBe(Date.UTC(2026, 1, 28, 9));
  expect(addMonthsUtc(Date.UTC(2029, 0, 15), -13)).toBe(Date.UTC(2027, 11, 15));
});
it("addMonthsUtc_leap_day_anniversaries_differ_from_stripe_by_a_day", () => {
  // A year that began on 29 Feb 2028 ends (Stripe clamps) on 28 Feb 2029;
  // twelve months back from that is 28 Feb 2028, a day before the real start.
  expect(addMonthsUtc(Date.UTC(2029, 1, 28, 9), -12)).toBe(Date.UTC(2028, 1, 28, 9));
  // A year that ends on 29 Feb 2028 began on 28 Feb 2027 (anchor day 29 or
  // later): S + 12 months is 28 Feb 2028, a day before the paid-through
  // date (which is why yearlyMonthAt ends month 12 at the year end).
  const start = addMonthsUtc(Date.UTC(2028, 1, 29, 9), -12);
  expect(start).toBe(Date.UTC(2027, 1, 28, 9));
  expect(addMonthsUtc(start, 12)).toBe(Date.UTC(2028, 1, 28, 9));
});
it("yearlyMonthAt_finds_the_month_and_ends_month_twelve_at_the_year_end", () => {
  const start = Date.UTC(2027, 0, 31, 10);
  const end = Date.UTC(2028, 0, 31, 10);
  expect(yearlyMonthAt(start, end, start - 1)).toEqual({ n: 1, endMs: Date.UTC(2027, 1, 28, 10) });
  expect(yearlyMonthAt(start, end, start)).toEqual({ n: 1, endMs: Date.UTC(2027, 1, 28, 10) });
  expect(yearlyMonthAt(start, end, Date.UTC(2027, 1, 28, 10) - 1)).toEqual({ n: 1, endMs: Date.UTC(2027, 1, 28, 10) });
  expect(yearlyMonthAt(start, end, Date.UTC(2027, 1, 28, 10))).toEqual({ n: 2, endMs: Date.UTC(2027, 2, 31, 10) });
  expect(yearlyMonthAt(start, end, Date.UTC(2027, 11, 31, 10))).toEqual({ n: 12, endMs: end });
  expect(yearlyMonthAt(start, end, end - 1)).toEqual({ n: 12, endMs: end });
  expect(yearlyMonthAt(start, end, end)).toBeNull();
  // A year paid through 29 Feb: month 12 ends on the 29th, not S + 12 months (the 28th).
  const leapEnd = Date.UTC(2028, 1, 29, 9);
  expect(yearlyMonthAt(Date.UTC(2027, 1, 28, 9), leapEnd, Date.UTC(2028, 1, 28, 12))).toEqual({ n: 12, endMs: leapEnd });
});
it("addMonthsUtc_refuses_non_whole_inputs", () => {
  expect(() => addMonthsUtc(1.5, 1)).toThrow(RangeError);
  expect(() => addMonthsUtc(0, 0.5)).toThrow(RangeError);
  expect(() => addMonthsUtc(NaN, 1)).toThrow(RangeError);
});
it("monthWindowEnd_refuses_non_whole_inputs", () => {
  expect(() => monthWindowEnd(1.5, 1, FAR)).toThrow(RangeError);
  expect(() => monthWindowEnd(0, -1, FAR)).toThrow(RangeError);
  expect(() => monthWindowEnd(0, 1, NaN)).toThrow(RangeError);
});
