import { afterEach, beforeEach, expect, it, vi } from "vitest";
import * as googleAuth from "../src/googleAuth";
import * as accounts from "../src/accounts";
import { creditPurchase, grantAllowance, zeroAllowance } from "../src/aiCredit";
import { handleAccountPlan, handleAiBalance } from "../src/index";
import type { SqlExecutor } from "../src/accounts";
import type { Env } from "../src/index";
import type { PlanState } from "../src/stripe";
import { realSqlExecutor } from "./testHelpers/realSqlExecutor";

// The plan tier and the balance's allowance fields against REAL SQLite:
// applyPlan's tier write and clear, and the reads the balance and plan
// routes answer from, run the SQL that ships.

const NOW = Date.UTC(2026, 10, 19, 9, 0, 0);
const END = NOW + 30 * 24 * 60 * 60 * 1000;

let open: (SqlExecutor & { close(): void })[] = [];
beforeEach(() => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: "g-1", audience: "test-client-id" });
});
afterEach(() => {
  for (const db of open) db.close();
  open = [];
  vi.restoreAllMocks();
});

const kv = { get: async () => null, put: async () => undefined };
const env = {
  GOOGLE_CLIENT_ID: "test-client-id",
  ALLOWED_GOOGLE_EMAILS: "owner@example.com",
  OPEN_SIGN_IN: "true",
  ENROLLMENT_HANDOFF: kv,
} as unknown as Env;

const post = (path: string) =>
  new Request(`https://example.com${path}`, { method: "POST", body: JSON.stringify({ idToken: "t", devicePublicKey: "mac-key" }) });

async function account() {
  const db = realSqlExecutor();
  open.push(db);
  const a = await accounts.getOrCreateAccount(db, "owner@example.com", "g-1", 1, "db", "https://db.test");
  await accounts.enrollDevice(db, a.id, "mac-key", "approved", 2, "mac");
  return { db, id: a.id };
}

const paid = (tier: "cloud" | "plus", untilUnixMs = END): PlanState => ({ plan: "cloud", tier, untilUnixMs, interval: "month", renews: true });
const ended: PlanState = { plan: "free", tier: null, untilUnixMs: NOW, interval: null, renews: false };

async function storedTier(db: SqlExecutor, id: string): Promise<unknown> {
  return (await db.execute("SELECT plan_tier FROM account WHERE id = ?", [id]))[0].plan_tier;
}

const balance = async (db: SqlExecutor, now = NOW) => (await handleAiBalance(post("/v1/ai/balance"), env, db, now)).json() as Promise<any>;
const plan = async (db: SqlExecutor, now = NOW) => (await handleAccountPlan(post("/v1/account/plan"), env, db, now)).json() as Promise<any>;

it("applyPlan_stores_the_tier_on_the_paid_branch", async () => {
  const { db, id } = await account();
  expect(await accounts.applyPlan(db, id, "sub_1", paid("plus"))).toBe(true);
  expect(await storedTier(db, id)).toBe("plus");
  expect(await accounts.applyPlan(db, id, "sub_1", paid("cloud"))).toBe(true);
  expect(await storedTier(db, id)).toBe("cloud");
});

it("a_lapsed_plus_account_clears_its_tier", async () => {
  const { db, id } = await account();
  expect(await accounts.applyPlan(db, id, "sub_1", paid("plus"))).toBe(true);
  expect(await storedTier(db, id)).toBe("plus");
  // A late free event for another subscription changes nothing, tier included.
  expect(await accounts.applyPlan(db, id, "sub_old", ended)).toBe(false);
  expect(await storedTier(db, id)).toBe("plus");
  expect(await accounts.applyPlan(db, id, "sub_1", ended)).toBe(true);
  expect(await storedTier(db, id)).toBeNull();
  expect((await accounts.findAccountById(db, id))?.planTier).toBeNull();
  expect((await plan(db)).plan_tier).toBeNull();
  expect((await balance(db)).plan_tier).toBeNull();
});

it("endPlan_clears_the_tier", async () => {
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", paid("plus"));
  await accounts.endPlan(db, id);
  expect(await storedTier(db, id)).toBeNull();
});

it("a_free_account_reports_no_plan_tier", async () => {
  const { db, id } = await account();
  expect((await plan(db)).plan_tier).toBeNull();
  expect((await balance(db)).plan_tier).toBeNull();
  // A tier still stored past the plan's end (no free event yet) is not reported.
  await accounts.applyPlan(db, id, "sub_1", paid("plus", NOW - 1));
  expect(await storedTier(db, id)).toBe("plus");
  expect(await plan(db)).toMatchObject({ plan: "free", plan_tier: null });
  expect((await balance(db)).plan_tier).toBeNull();
});

it("account_plan_returns_plan_tier", async () => {
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", paid("plus"));
  expect(await plan(db)).toEqual({ plan: "cloud", until: END, interval: "month", renews: true, plan_tier: "plus", billing: false, plus_available: false });
  await accounts.applyPlan(db, id, "sub_1", paid("cloud"));
  expect((await plan(db)).plan_tier).toBe("cloud");
});

it("a_paid_account_stored_before_tiers_reports_cloud", async () => {
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", paid("cloud"));
  await db.execute("UPDATE account SET plan_tier = NULL WHERE id = ?", [id]);
  expect((await plan(db)).plan_tier).toBe("cloud");
  expect((await balance(db)).plan_tier).toBe("cloud");
});

it("balance_adds_allowance_fields_and_keeps_the_old_ones", async () => {
  const { db, id } = await account();
  await creditPurchase(db, id, 10_000_000, "pi_1", NOW - 10);
  expect(await balance(db)).toEqual({
    balance_micros: 10_000_000,
    last_purchase_micros: 10_000_000,
    allowance_micros: 0,
    allowance_expires_ms: null,
    plan_tier: null,
  });
  await accounts.applyPlan(db, id, "sub_1", paid("plus"));
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_1", micros: 6_000_000, periodEndUnixMs: END }, NOW)).toBe(true);
  // Prepaid stays prepaid only; the allowance is reported beside it.
  expect(await balance(db)).toEqual({
    balance_micros: 10_000_000,
    last_purchase_micros: 10_000_000,
    allowance_micros: 6_000_000,
    allowance_expires_ms: END,
    plan_tier: "plus",
  });
  // Expired: 0 and no date.
  expect(await balance(db, END)).toMatchObject({ balance_micros: 10_000_000, allowance_micros: 0, allowance_expires_ms: null });
  // Zeroed: the stored expiry is still in the future, but no date is reported.
  await zeroAllowance(db, id, NOW, { subscriptionId: "sub_1", mode: "refund" });
  const row = (await db.execute("SELECT ai_allowance_expires_ms FROM account WHERE id = ?", [id]))[0];
  expect(Number(row.ai_allowance_expires_ms)).toBeGreaterThan(NOW);
  expect(await balance(db)).toEqual({
    balance_micros: 10_000_000,
    last_purchase_micros: 10_000_000,
    allowance_micros: 0,
    allowance_expires_ms: null,
    plan_tier: "plus",
  });
});

it("plus_available_is_present_for_free_cloud_and_plus_accounts", async () => {
  const billingEnv = {
    ...env,
    STRIPE_SECRET_KEY: "sk_test_x",
    STRIPE_WEBHOOK_SECRET: "whsec_x",
    STRIPE_PRICE_MONTHLY: "price_monthly",
    STRIPE_PRICE_YEARLY: "price_yearly",
    STRIPE_PRICE_PLUS_MONTHLY: "price_plus_monthly",
    STRIPE_PRICE_PLUS_YEARLY: "price_plus_yearly",
  } as Env;
  const noPlus = { ...billingEnv, STRIPE_PRICE_PLUS_YEARLY: "" } as Env;
  const read = async (db: SqlExecutor, e: Env) => (await handleAccountPlan(post("/v1/account/plan"), e, db, NOW)).json() as Promise<any>;
  const { db, id } = await account();
  const seen: [string, unknown, unknown][] = [];
  for (const step of ["free", "cloud", "plus"] as const) {
    if (step !== "free") await accounts.applyPlan(db, id, "sub_1", paid(step));
    const on = await read(db, billingEnv);
    const off = await read(db, noPlus);
    expect(Object.hasOwn(on, "plus_available") && Object.hasOwn(off, "plus_available")).toBe(true);
    seen.push([on.plan_tier ?? "free", on.plus_available, off.plus_available]);
  }
  expect(seen).toEqual([
    ["free", true, false],
    ["cloud", true, false],
    ["plus", true, false],
  ]);
});
