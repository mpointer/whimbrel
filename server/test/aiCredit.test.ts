import { it, expect } from "vitest";
import * as accounts from "../src/accounts";
import {
  STALE_HOLD_MS,
  aiBalance,
  creditPurchase,
  lastPurchaseMicros,
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
