import { it, expect, vi, beforeEach } from "vitest";
import * as googleAuth from "../src/googleAuth";
import * as turso from "../src/turso";
import * as accounts from "../src/accounts";
import { handleAccountPlan, handleAiBalance, handleAiCheckout, handleBillingCheckout, handleBillingPortal, handleBillingWebhook, handleDeleteAccount, handleGoogleAuth } from "../src/index";
import { aiBalance, creditPurchase, placeHold, settleHold } from "../src/aiCredit";
import { deleteAccount } from "../src/inactivity";
import type { Env } from "../src/index";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";
import { createExecutionContext, env } from "cloudflare:test";

const NOW = 1_800_000_000_000;
const SUCCESS = "https://whimbrelapp.com/cloud/thanks/";
const CANCEL = "https://whimbrelapp.com/cloud/cancelled/";

function makeEnv(overrides: Partial<Env> = {}): Env {
  return {
    GOOGLE_CLIENT_ID: "test-client-id",
    TURSO_DATABASE_URL: "https://test-control-plane.example.turso.io",
    TURSO_DATABASE_AUTH_TOKEN: "test-database-token",
    TURSO_PLATFORM_API_TOKEN: "test-platform-token",
    TURSO_ORG_SLUG: "test-org",
    TURSO_DB_GROUP: "test-group",
    ALLOWED_GOOGLE_EMAILS: "owner@example.com",
    OPEN_SIGN_IN: "true",
    STRIPE_SECRET_KEY: "sk_test_x",
    STRIPE_WEBHOOK_SECRET: "whsec_test_secret",
    STRIPE_PRICE_MONTHLY: "price_monthly",
    STRIPE_PRICE_YEARLY: "price_yearly",
    ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF,
    ...overrides,
  };
}

const post = (path: string, body: unknown) =>
  new Request(`https://example.com${path}`, { method: "POST", body: JSON.stringify(body) });
const creds = { idToken: "t", devicePublicKey: "caller-key" };

let sub = "";
beforeEach(() => {
  vi.restoreAllMocks();
  sub = `bill-${crypto.randomUUID()}`;
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: sub, audience: "test-client-id" });
  vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({ name: "n", url: "https://n" });
});

async function seeded() {
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", sub, 1, "old-db", "https://old-db");
  await accounts.enrollDevice(db, account.id, "mac-key", "approved", 2, "mac");
  return { db, account };
}

function fakeFetch(res: () => Response = () => new Response(JSON.stringify({ url: "https://checkout.stripe.com/c/test" }), { status: 200 })) {
  const calls: { url: string; form: URLSearchParams }[] = [];
  const fetcher = (async (u: string, init: RequestInit) => {
    calls.push({ url: u, form: new URLSearchParams(String(init.body ?? "")) });
    return res();
  }) as unknown as typeof fetch;
  return { fetcher, calls };
}

const checkout = (db: FakeSqlExecutor, fetcher: typeof fetch, body: object = {}, e: Env = makeEnv()) =>
  handleBillingCheckout(post("/v1/billing/checkout", { ...creds, interval: "month", ...body }), e, db, fetcher, NOW);

it("plan_reports_free_with_billing_flag", async () => {
  const { db } = await seeded();
  const res = await handleAccountPlan(post("/v1/account/plan", creds), makeEnv(), db, NOW);
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ plan: "free", until: null, interval: null, renews: null, billing: true });
});

it("plan_reports_cloud_until_and_interval", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, planInterval: "year" });
  const res = await handleAccountPlan(post("/v1/account/plan", creds), makeEnv(), db, NOW);
  expect(await res.json()).toEqual({ plan: "cloud", until: NOW + 1000, interval: "year", renews: true, billing: true });
});

it("plan_reports_a_cancelled_cloud_plan_as_not_renewing", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, planInterval: "month", planRenews: false });
  const res = await handleAccountPlan(post("/v1/account/plan", creds), makeEnv(), db, NOW);
  expect(await res.json()).toEqual({ plan: "cloud", until: NOW + 1000, interval: "month", renews: false, billing: true });
});

it("plan_reports_free_once_the_period_has_ended", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW - 1, planInterval: "year" });
  const res = await handleAccountPlan(post("/v1/account/plan", creds), makeEnv(), db, NOW);
  expect(await res.json()).toEqual({ plan: "free", until: null, interval: null, renews: null, billing: true });
});

it("plan_billing_false_when_a_stripe_setting_is_missing", async () => {
  const { db } = await seeded();
  const res = await handleAccountPlan(post("/v1/account/plan", creds), makeEnv({ STRIPE_PRICE_YEARLY: undefined }), db, NOW);
  expect(res.status).toBe(200);
  expect((await res.json() as any).billing).toBe(false);
});

it("plan_without_an_account_is_403", async () => {
  const res = await handleAccountPlan(post("/v1/account/plan", creds), makeEnv(), new FakeSqlExecutor(), NOW);
  expect(res.status).toBe(403);
});

it("checkout_from_a_device_refused_at_the_limit_works", async () => {
  const { db, account } = await seeded();
  await accounts.enrollDevice(db, account.id, "iphone-key", "approved", 3, "ios");
  const auth = () => handleGoogleAuth(post("/v1/auth/google", { idToken: "t", devicePublicKey: "new-key", platform: "windows" }), makeEnv(), db);
  expect((await auth()).status).toBe(409);
  const { fetcher } = fakeFetch();
  const res = await handleBillingCheckout(post("/v1/billing/checkout", { idToken: "t", devicePublicKey: "new-key", interval: "month" }), makeEnv(), db, fetcher, NOW);
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ url: "https://checkout.stripe.com/c/test" });
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: Date.now() + 86_400_000 });
  const again = await auth();
  expect(again.status).toBe(200);
  expect((await again.json() as any).status).toBe("pending");
});

it("checkout_sends_price_reference_metadata_tax_and_urls", async () => {
  const { db, account } = await seeded();
  const { fetcher, calls } = fakeFetch();
  await checkout(db, fetcher, { interval: "year" });
  expect(calls).toHaveLength(1);
  expect(calls[0].url).toBe("https://api.stripe.com/v1/checkout/sessions");
  const f = calls[0].form;
  expect(f.get("mode")).toBe("subscription");
  expect(f.get("line_items[0][price]")).toBe("price_yearly");
  expect(f.get("line_items[0][quantity]")).toBe("1");
  expect(f.get("client_reference_id")).toBe(account.id);
  expect(f.get("subscription_data[metadata][account_id]")).toBe(account.id);
  expect(f.get("automatic_tax[enabled]")).toBe("true");
  expect(f.get("billing_address_collection")).toBe("required");
  expect(f.get("customer_email")).toBe("owner@example.com");
  expect(f.has("customer")).toBe(false);
  expect(f.get("success_url")).toBe(SUCCESS);
  expect(f.get("cancel_url")).toBe(CANCEL);
});

it("checkout_uses_the_monthly_price_for_month", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = fakeFetch();
  await checkout(db, fetcher);
  expect(calls[0].form.get("line_items[0][price]")).toBe("price_monthly");
});

it("checkout_reuses_an_existing_customer", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "free", planUntilUnixMs: null, stripeCustomerId: "cus_1" });
  const { fetcher, calls } = fakeFetch();
  await checkout(db, fetcher);
  const f = calls[0].form;
  expect(f.get("customer")).toBe("cus_1");
  expect(f.get("customer_update[address]")).toBe("auto");
  expect(f.has("customer_email")).toBe(false);
});

it("checkout_refuses_an_account_already_on_cloud", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000 });
  const { fetcher, calls } = fakeFetch();
  const res = await checkout(db, fetcher);
  expect(res.status).toBe(409);
  expect(await res.json()).toEqual({ error: "already_cloud" });
  expect(calls).toHaveLength(0);
});

it("checkout_bad_interval_is_400", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = fakeFetch();
  for (const interval of ["week", undefined, 5]) {
    const res = await checkout(db, fetcher, { interval });
    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "interval_required" });
  }
  expect(calls).toHaveLength(0);
});

it("billing_routes_503_without_settings", async () => {
  const { db } = await seeded();
  for (const key of ["STRIPE_SECRET_KEY", "STRIPE_WEBHOOK_SECRET", "STRIPE_PRICE_MONTHLY", "STRIPE_PRICE_YEARLY"] as const) {
    const { fetcher, calls } = fakeFetch();
    const e = makeEnv({ [key]: undefined });
    const c = await checkout(db, fetcher, {}, e);
    expect(c.status).toBe(503);
    expect(await c.json()).toEqual({ error: "billing_unavailable" });
    const p = await handleBillingPortal(post("/v1/billing/portal", creds), e, db, fetcher);
    expect(p.status).toBe(503);
    expect(calls).toHaveLength(0);
  }
});

it("stripe_failure_is_502", async () => {
  const { db } = await seeded();
  const leak = () => new Response(JSON.stringify({ error: { message: "secret detail sk_live_leak" } }), { status: 500 });
  const res = await checkout(db, fakeFetch(leak).fetcher);
  expect(res.status).toBe(502);
  expect(await res.text()).not.toContain("leak");
  const net = (async () => {
    throw new Error("boom");
  }) as unknown as typeof fetch;
  expect((await checkout(db, net)).status).toBe(502);
  const noUrl = await checkout(db, fakeFetch(() => new Response("{}", { status: 200 })).fetcher);
  expect(noUrl.status).toBe(502);
});

it("checkout_is_limited_to_20_per_hour", async () => {
  const { db } = await seeded();
  const { fetcher } = fakeFetch();
  const statuses = [];
  for (let i = 0; i < 21; i++) statuses.push((await checkout(db, fetcher)).status);
  expect(statuses.slice(0, 20).every((s) => s === 200)).toBe(true);
  expect(statuses[20]).toBe(429);
});

it("portal_returns_url_for_customer", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, stripeCustomerId: "cus_9" });
  const { fetcher, calls } = fakeFetch(() => new Response(JSON.stringify({ url: "https://billing.stripe.com/p/test" }), { status: 200 }));
  const res = await handleBillingPortal(post("/v1/billing/portal", creds), makeEnv(), db, fetcher);
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ url: "https://billing.stripe.com/p/test" });
  expect(calls[0].url).toBe("https://api.stripe.com/v1/billing_portal/sessions");
  expect(calls[0].form.get("customer")).toBe("cus_9");
  expect(calls[0].form.get("return_url")).toBe(SUCCESS);
});

it("portal_without_customer_is_409_no_subscription", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = fakeFetch();
  const res = await handleBillingPortal(post("/v1/billing/portal", creds), makeEnv(), db, fetcher);
  expect(res.status).toBe(409);
  expect(await res.json()).toEqual({ error: "no_subscription" });
  expect(calls).toHaveLength(0);
});

it("portal_stripe_failure_is_502_without_the_body", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "free", planUntilUnixMs: null, stripeCustomerId: "cus_9" });
  const res = await handleBillingPortal(post("/v1/billing/portal", creds), makeEnv(), db, fakeFetch(() => new Response('{"error":"leak"}', { status: 400 })).fetcher);
  expect(res.status).toBe(502);
  expect(await res.text()).not.toContain("leak");
});

it("setStripeCustomer_sets_it_unless_the_account_is_deleting", async () => {
  const { db, account } = await seeded();
  expect(await accounts.setStripeCustomer(db, account.id, "cus_2")).toBe(true);
  expect(db.rawAccount(account.id)?.stripeCustomerId).toBe("cus_2");
  expect(await accounts.setStripeCustomer(db, "nope", "cus_3")).toBe(false);
});

it("portal_shares_the_billing_rate_limit", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "free", planUntilUnixMs: null, stripeCustomerId: "cus_9" });
  const { fetcher } = fakeFetch();
  for (let i = 0; i < 20; i++) expect((await checkout(db, fetcher)).status).toBe(200);
  const res = await handleBillingPortal(post("/v1/billing/portal", creds), makeEnv(), db, fetcher);
  expect(res.status).toBe(429);
});

it("the_worker_routes_the_billing_paths", async () => {
  // No DB wired: a 400 on an empty body proves the route exists and parses.
  const worker = (await import("../src/index")).default;
  for (const p of ["/v1/account/plan", "/v1/billing/checkout", "/v1/billing/portal", "/v1/ai/checkout", "/v1/ai/balance"]) {
    const res = await worker.fetch(new Request(`https://example.com${p}`, { method: "POST" }), makeEnv(), createExecutionContext());
    expect(res.status).toBe(400);
  }
  const hook = await worker.fetch(new Request("https://example.com/v1/billing/webhook", { method: "POST", body: "{}" }), makeEnv(), createExecutionContext());
  expect(hook.status).toBe(400);
  expect(await hook.json()).toEqual({ error: "bad_signature" });
});

// ---- webhook ----

const enc = new TextEncoder();
/** Same scheme as stripe.test.ts: `t=…,v1=hex(HMAC-SHA256(secret, "t.body"))`. */
async function signature(body: string, secret = "whsec_test_secret", t = NOW / 1000): Promise<string> {
  const key = await crypto.subtle.importKey("raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const sig = new Uint8Array(await crypto.subtle.sign("HMAC", key, enc.encode(`${t}.${body}`)));
  return `t=${t},v1=${[...sig].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
}

let eventSeq = 0;
function event(type: string, object: object, id = `evt_${++eventSeq}`) {
  return { id, object: "event", type, data: { object } };
}
const completed = (accountId: string, extra: object = {}) =>
  event("checkout.session.completed", { id: "cs_1", object: "checkout.session", customer: "cus_1", subscription: "sub_1", client_reference_id: accountId, ...extra });
const subEvent = (type: string, id = "sub_1") => event(type, { id, object: "subscription" });

const PERIOD_END = NOW / 1000 + 30 * 86_400;
function subscription(accountId: string, over: Record<string, unknown> = {}) {
  return {
    id: "sub_1",
    object: "subscription",
    status: "active",
    customer: "cus_1",
    created: NOW / 1000 - 100,
    metadata: { account_id: accountId },
    items: { data: [{ current_period_end: PERIOD_END, price: { id: "price_monthly", recurring: { interval: "month" } } }] },
    ...over,
  };
}

/** Serves `GET /v1/subscriptions/{id}` from `subs`, `DELETE` as cancelled,
 * and Resend sends as accepted. Records method, URL and body of every call. */
function stripeFake(subs: Record<string, object>, stripeStatus = 200, deleteStatus = 200) {
  const calls: { method: string; url: string; body: string }[] = [];
  const fetcher = (async (u: string, init: RequestInit) => {
    const method = init.method ?? "GET";
    calls.push({ method, url: u, body: String(init.body ?? "") });
    if (u.startsWith("https://api.resend.com/")) return new Response('{"id":"e"}', { status: 200 });
    if (stripeStatus !== 200) return new Response('{"error":{"message":"down"}}', { status: stripeStatus });
    const id = u.split("/v1/subscriptions/")[1];
    const s = subs[id];
    if (!s) return new Response('{"error":{"code":"resource_missing"}}', { status: 404 });
    if (method === "DELETE" && deleteStatus !== 200) return new Response('{"error":{"message":"down"}}', { status: deleteStatus });
    return new Response(JSON.stringify(method === "DELETE" ? { ...s, status: "canceled" } : s), { status: 200 });
  }) as unknown as typeof fetch;
  return { fetcher, calls };
}

async function deliver(db: FakeSqlExecutor, fetcher: typeof fetch, evt: object, e: Env = makeEnv(), sig?: string | null) {
  const body = JSON.stringify(evt);
  const headers: Record<string, string> = {};
  const s = sig === undefined ? await signature(body) : sig;
  if (s !== null) headers["stripe-signature"] = s;
  return handleBillingWebhook(new Request("https://example.com/v1/billing/webhook", { method: "POST", body, headers }), e, db, fetcher, NOW);
}

it("unsigned_or_tampered_webhook_is_400_and_changes_nothing", async () => {
  const { db, account } = await seeded();
  const execute = vi.spyOn(db, "execute");
  const { fetcher, calls } = stripeFake({ sub_1: subscription(account.id) });
  const evt = completed(account.id);
  const good = await signature(JSON.stringify(evt));
  const tampered = await signature(JSON.stringify({ ...evt, id: "evt_other" }));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  for (const sig of [null, "", "t=1,v1=00", tampered, await signature(JSON.stringify(evt), "whsec_wrong")]) {
    const res = await deliver(db, fetcher, evt, makeEnv(), sig);
    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "bad_signature" });
  }
  expect(good).not.toBe(tampered);
  // One line per refusal, never the header or the body.
  expect(log.mock.calls).toEqual(Array(5).fill(["billing webhook: bad signature"]));
  expect(execute).not.toHaveBeenCalled();
  expect(calls).toHaveLength(0);
  expect(db.rawAccount(account.id)?.plan ?? "free").toBe("free");
});

it("webhook_503_without_billing_settings", async () => {
  const { db, account } = await seeded();
  const { fetcher, calls } = stripeFake({ sub_1: subscription(account.id) });
  const res = await deliver(db, fetcher, completed(account.id), makeEnv({ STRIPE_PRICE_MONTHLY: undefined }));
  expect(res.status).toBe(503);
  expect(await res.json()).toEqual({ error: "billing_unavailable" });
  expect(calls).toHaveLength(0);
});

it("checkout_completed_links_customer_and_makes_cloud", async () => {
  const { db, account } = await seeded();
  const { fetcher, calls } = stripeFake({ sub_1: subscription(account.id) });
  const res = await deliver(db, fetcher, completed(account.id));
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ received: true });
  expect(calls).toEqual([{ method: "GET", url: "https://api.stripe.com/v1/subscriptions/sub_1", body: "" }]);
  const a = db.rawAccount(account.id)!;
  expect(a.plan).toBe("cloud");
  expect(a.planUntilUnixMs).toBe(PERIOD_END * 1000);
  expect(a.planInterval).toBe("month");
  expect(a.stripeCustomerId).toBe("cus_1");
  expect(a.stripeSubscriptionId).toBe("sub_1");
});

it("checkout_completed_without_metadata_uses_client_reference_id", async () => {
  const { db, account } = await seeded();
  const { fetcher } = stripeFake({ sub_1: subscription(account.id, { metadata: {} }) });
  expect((await deliver(db, fetcher, completed(account.id))).status).toBe(200);
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
});

it("subscription_created_before_checkout_completed_still_lands", async () => {
  const { db, account } = await seeded();
  const { fetcher } = stripeFake({ sub_1: subscription(account.id) });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.created"))).status).toBe(200);
  const a = db.rawAccount(account.id)!;
  expect(a.plan).toBe("cloud");
  expect(a.stripeSubscriptionId).toBe("sub_1");
  expect(a.planUntilUnixMs).toBe(PERIOD_END * 1000);
});

it("same_event_id_twice_is_processed_once", async () => {
  const { db, account } = await seeded();
  const { fetcher, calls } = stripeFake({ sub_1: subscription(account.id) });
  const evt = completed(account.id);
  expect((await deliver(db, fetcher, evt)).status).toBe(200);
  const again = await deliver(db, fetcher, evt);
  expect(again.status).toBe(200);
  expect(await again.json()).toEqual({ received: true });
  expect(calls).toHaveLength(1);
});

it("old_subscription_deleted_does_not_end_a_newer_one", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "year", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_2" });
  const { fetcher } = stripeFake({ sub_1: subscription(account.id, { status: "canceled", ended_at: NOW / 1000 - 50 }) });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.deleted"))).status).toBe(200);
  const a = db.rawAccount(account.id)!;
  expect(a.plan).toBe("cloud");
  expect(a.stripeSubscriptionId).toBe("sub_2");
  expect(a.planUntilUnixMs).toBe(PERIOD_END * 1000);
  expect(a.planInterval).toBe("year");
});

it("a_second_live_subscription_is_cancelled_and_alerted", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const older = subscription(account.id, { created: NOW / 1000 - 1000 });
  const newer = subscription(account.id, { id: "sub_2", created: NOW / 1000 - 10, items: { data: [{ current_period_end: PERIOD_END + 365 * 86_400, price: { id: "price_yearly", recurring: { interval: "year" } } }] } });
  const { fetcher, calls } = stripeFake({ sub_1: older, sub_2: newer });
  const res = await deliver(db, fetcher, subEvent("customer.subscription.created", "sub_2"), makeEnv({ RESEND_API_KEY: "re_test" }));
  expect(res.status).toBe(200);
  expect(calls.filter((c) => c.method === "DELETE").map((c) => c.url)).toEqual(["https://api.stripe.com/v1/subscriptions/sub_2"]);
  const a = db.rawAccount(account.id)!;
  expect(a.plan).toBe("cloud");
  expect(a.stripeSubscriptionId).toBe("sub_1");
  expect(a.planInterval).toBe("month");
  const emails = calls.filter((c) => c.url === "https://api.resend.com/emails").map((c) => JSON.parse(c.body));
  expect(emails).toHaveLength(1);
  expect(emails[0].to).toEqual(["hello@whimbrelapp.com"]);
  for (const s of [account.id, "sub_1", "sub_2"]) expect(emails[0].text).toContain(s);
  expect(emails[0].text).not.toContain("owner@example.com");
  // The cancelled one's own deletion event later changes nothing.
  const after = stripeFake({ sub_2: { ...newer, status: "canceled", ended_at: NOW / 1000 } });
  expect((await deliver(db, after.fetcher, subEvent("customer.subscription.deleted", "sub_2"))).status).toBe(200);
  expect(db.rawAccount(account.id)?.stripeSubscriptionId).toBe("sub_1");
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
});

it("duplicate_keeps_the_kept_subscriptions_customer", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const older = subscription(account.id, { created: NOW / 1000 - 1000 });
  const newer = subscription(account.id, { id: "sub_2", customer: "cus_2", created: NOW / 1000 - 10 });
  const { fetcher } = stripeFake({ sub_1: older, sub_2: newer });
  vi.spyOn(console, "error").mockImplementation(() => {});
  expect((await deliver(db, fetcher, completed(account.id, { customer: "cus_2", subscription: "sub_2" }))).status).toBe(200);
  expect(db.rawAccount(account.id)?.stripeSubscriptionId).toBe("sub_1");
  expect(db.rawAccount(account.id)?.stripeCustomerId).toBe("cus_1");
});

it("an_older_live_subscription_arriving_late_keeps_the_older_and_cancels_the_stored_newer", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "year", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_2" });
  const older = subscription(account.id, { created: NOW / 1000 - 1000 });
  const newer = subscription(account.id, { id: "sub_2", created: NOW / 1000 - 10 });
  const { fetcher, calls } = stripeFake({ sub_1: older, sub_2: newer });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.updated", "sub_1"))).status).toBe(200);
  expect(calls.filter((c) => c.method === "DELETE").map((c) => c.url)).toEqual(["https://api.stripe.com/v1/subscriptions/sub_2"]);
  expect(db.rawAccount(account.id)?.stripeSubscriptionId).toBe("sub_1");
});

it("a_new_subscription_after_the_stored_one_ended_takes_over", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "free", planUntilUnixMs: NOW - 1000, stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const { fetcher, calls } = stripeFake({
    sub_1: subscription(account.id, { status: "canceled", ended_at: NOW / 1000 - 1 }),
    sub_2: subscription(account.id, { id: "sub_2" }),
  });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.created", "sub_2"))).status).toBe(200);
  expect(calls.some((c) => c.method === "DELETE")).toBe(false);
  expect(db.rawAccount(account.id)?.stripeSubscriptionId).toBe("sub_2");
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
});

it("invoice_payment_failed_while_past_due_stays_cloud", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const { fetcher } = stripeFake({ sub_1: subscription(account.id, { status: "past_due" }) });
  const invoice = event("invoice.payment_failed", { id: "in_1", object: "invoice", parent: { subscription_details: { subscription: "sub_1" } } });
  expect((await deliver(db, fetcher, invoice)).status).toBe(200);
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
  expect(db.rawAccount(account.id)?.planUntilUnixMs).toBe(PERIOD_END * 1000);
});

it("unpaid_ends_the_plan_and_keeps_until_for_the_clock", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const { fetcher } = stripeFake({ sub_1: subscription(account.id, { status: "unpaid" }) });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.updated"))).status).toBe(200);
  const a = db.rawAccount(account.id)!;
  expect(a.plan).toBe("free");
  expect(a.planUntilUnixMs).toBe(NOW);
  expect(a.planInterval).toBeNull();
  expect(a.stripeSubscriptionId).toBe("sub_1");
});

it("paying_clears_an_inactivity_warning", async () => {
  const { db, account } = await seeded();
  db.markWarned(account.id, NOW - 10 * 86_400_000);
  const { fetcher } = stripeFake({ sub_1: subscription(account.id) });
  expect((await deliver(db, fetcher, completed(account.id))).status).toBe(200);
  expect(db.rawAccount(account.id)?.warnedAtUnixMs).toBeNull();
});

it("webhook_for_a_deleted_account_cancels_its_cloud_subscription_and_alerts", async () => {
  const db = new FakeSqlExecutor();
  const { fetcher, calls } = stripeFake({ sub_1: subscription("acct_gone") });
  const evt = completed("acct_gone");
  const e = makeEnv({ RESEND_API_KEY: "re_test" });
  const res = await deliver(db, fetcher, evt, e);
  expect(res.status).toBe(200);
  expect(db.rawAccount("acct_gone")).toBeUndefined();
  expect(calls.filter((c) => c.method === "DELETE").map((c) => c.url)).toEqual(["https://api.stripe.com/v1/subscriptions/sub_1"]);
  const emails = calls.filter((c) => c.url === "https://api.resend.com/emails").map((c) => JSON.parse(c.body));
  expect(emails).toHaveLength(1);
  expect(emails[0].to).toEqual(["hello@whimbrelapp.com"]);
  expect(emails[0].text).toContain("refund by hand");
  for (const s of ["acct_gone", "sub_1", "cus_1"]) expect(emails[0].text).toContain(s);
  // Recorded: a second delivery calls nothing.
  const n = calls.length;
  await deliver(db, fetcher, evt, e);
  expect(calls).toHaveLength(n);
});

it("a_deleted_accounts_cancel_failure_is_500_then_retry_cancels_and_alerts_once", async () => {
  const db = new FakeSqlExecutor();
  const evt = completed("acct_gone");
  const e = makeEnv({ RESEND_API_KEY: "re_test" });
  vi.spyOn(console, "error").mockImplementation(() => {});
  const failing = stripeFake({ sub_1: subscription("acct_gone") }, 200, 500);
  expect((await deliver(db, failing.fetcher, evt, e)).status).toBe(500);
  expect(failing.calls.filter((c) => c.method === "DELETE")).toHaveLength(1);
  expect(failing.calls.some((c) => c.url.startsWith("https://api.resend.com/"))).toBe(false);
  const ok = stripeFake({ sub_1: subscription("acct_gone") });
  expect((await deliver(db, ok.fetcher, evt, e)).status).toBe(200);
  expect(ok.calls.filter((c) => c.method === "DELETE")).toHaveLength(1);
  expect(ok.calls.filter((c) => c.url.startsWith("https://api.resend.com/"))).toHaveLength(1);
});

it("a_deleted_accounts_ended_subscription_is_not_cancelled_again", async () => {
  const db = new FakeSqlExecutor();
  const { fetcher, calls } = stripeFake({ sub_1: subscription("acct_gone", { status: "canceled", ended_at: NOW / 1000 }) });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.deleted"), makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(calls.map((c) => c.method)).toEqual(["GET"]);
});

it("a_deleting_accounts_own_subscription_is_left_to_the_deletion", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  await accounts.claimForDeletion(db, account.id, NOW);
  const { fetcher, calls } = stripeFake({ sub_1: subscription(account.id) });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.updated"), makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(calls.map((c) => c.method)).toEqual(["GET"]);
});

it("webhook_for_an_account_being_deleted_writes_nothing", async () => {
  const { db, account } = await seeded();
  await accounts.claimForDeletion(db, account.id, NOW);
  const { fetcher } = stripeFake({ sub_1: subscription(account.id) });
  expect((await deliver(db, fetcher, completed(account.id))).status).toBe(200);
  expect(db.rawAccount(account.id)?.plan ?? "free").toBe("free");
  expect(db.rawAccount(account.id)?.stripeCustomerId ?? null).toBeNull();
});

it("stripe_down_returns_500_and_does_not_record_the_event", async () => {
  const { db, account } = await seeded();
  const evt = completed(account.id);
  const down = stripeFake({}, 500);
  const res = await deliver(db, down.fetcher, evt);
  expect(res.status).toBe(500);
  expect(await res.text()).not.toContain("down");
  expect(db.rawAccount(account.id)?.plan ?? "free").toBe("free");
  const up = stripeFake({ sub_1: subscription(account.id) });
  expect((await deliver(db, up.fetcher, evt)).status).toBe(200);
  expect(up.calls).toHaveLength(1);
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
});

it("events_without_a_subscription_are_recorded_and_ignored", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = stripeFake({});
  const res = await deliver(db, fetcher, event("customer.created", { id: "cus_1", object: "customer" }));
  expect(res.status).toBe(200);
  expect(calls).toHaveLength(0);
});

it("applyPlan_free_only_ends_the_stored_subscription", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, stripeSubscriptionId: "sub_2" });
  expect(await accounts.applyPlan(db, account.id, "sub_1", { plan: "free", untilUnixMs: NOW, interval: null, renews: false })).toBe(false);
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
  expect(await accounts.applyPlan(db, account.id, "sub_2", { plan: "free", untilUnixMs: NOW, interval: null, renews: false })).toBe(true);
  expect(db.rawAccount(account.id)?.plan).toBe("free");
});

it("another_products_subscription_is_ignored", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const before = { ...db.rawAccount(account.id) };
  const other = subscription(account.id, {
    id: "sub_fma",
    customer: "cus_fma",
    metadata: {},
    items: { data: [{ current_period_end: PERIOD_END, price: { id: "price_other_product", recurring: { interval: "month" } } }] },
  });
  const { fetcher, calls } = stripeFake({ sub_1: subscription(account.id), sub_fma: other });
  const e = makeEnv({ RESEND_API_KEY: "re_test" });
  const execute = vi.spyOn(db, "execute");
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  const evts = [
    subEvent("customer.subscription.created", "sub_fma"),
    event("checkout.session.completed", { id: "cs_fma", object: "checkout.session", customer: "cus_fma", subscription: "sub_fma", client_reference_id: null }),
    event("invoice.paid", { id: "in_fma", object: "invoice", parent: { subscription_details: { subscription: "sub_fma" } } }),
  ];
  for (const evt of evts) {
    const res = await deliver(db, fetcher, evt, e);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ received: true });
  }
  const writes = execute.mock.calls.map(([q]) => q.replace(/\s+/g, " ")).filter((q) => !q.startsWith("SELECT") && !q.startsWith("INSERT OR IGNORE INTO stripe_event"));
  expect(writes).toEqual([]);
  expect(calls.every((c) => c.method === "GET" && c.url === "https://api.stripe.com/v1/subscriptions/sub_fma")).toBe(true);
  expect(db.rawAccount(account.id)).toEqual(before);
  expect(log.mock.calls).toEqual(Array(3).fill(["billing: ignored subscription", "sub_fma", "active"]));
});

it("a_renewal_on_a_retired_price_with_our_metadata_still_applies", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const { fetcher } = stripeFake({ sub_1: subscription(account.id) });
  const invoice = event("invoice.paid", { id: "in_1", object: "invoice", parent: { subscription_details: { subscription: "sub_1" } } });
  expect((await deliver(db, fetcher, invoice, makeEnv({ STRIPE_PRICE_MONTHLY: "price_monthly_v2" }))).status).toBe(200);
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
  expect(db.rawAccount(account.id)?.planUntilUnixMs).toBe(PERIOD_END * 1000);
});

// The other product never writes Whimbrel account ids, so its own
// metadata.account_id names no Whimbrel account.
it("another_products_subscription_with_its_own_account_id_is_still_ignored", async () => {
  const { db, account } = await seeded();
  const { fetcher, calls } = stripeFake({
    sub_1: subscription(account.id, { metadata: { account_id: "fma_account_1" }, items: { data: [{ current_period_end: PERIOD_END, price: { id: "price_other_product", recurring: { interval: "month" } } }] } }),
  });
  expect((await deliver(db, fetcher, completed(account.id), makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(calls.map((c) => c.method)).toEqual(["GET"]);
  expect(db.rawAccount(account.id)?.plan ?? "free").toBe("free");
  expect(db.rawAccount(account.id)?.stripeCustomerId ?? null).toBeNull();
});

it("another_products_live_subscription_is_not_a_duplicate", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_fma" });
  const { fetcher, calls } = stripeFake({
    sub_fma: subscription(account.id, { id: "sub_fma", created: NOW / 1000 - 5000, metadata: { account_id: "fma_account_1" }, items: { data: [{ current_period_end: PERIOD_END, price: { id: "price_other_product" } }] } }),
    sub_2: subscription(account.id, { id: "sub_2" }),
  });
  expect((await deliver(db, fetcher, subEvent("customer.subscription.created", "sub_2"), makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(calls.some((c) => c.method === "DELETE" || c.url.startsWith("https://api.resend.com/"))).toBe(false);
  expect(db.rawAccount(account.id)?.stripeSubscriptionId).toBe("sub_2");
});

it("portal_uses_the_whimbrel_configuration_when_set", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, stripeCustomerId: "cus_9" });
  const portal = () => fakeFetch(() => new Response(JSON.stringify({ url: "https://billing.stripe.com/p/test" }), { status: 200 }));
  const set = portal();
  expect((await handleBillingPortal(post("/v1/billing/portal", creds), makeEnv({ STRIPE_PORTAL_CONFIGURATION: "bpc_whimbrel" }), db, set.fetcher)).status).toBe(200);
  expect(set.calls[0].form.get("configuration")).toBe("bpc_whimbrel");
  const unset = portal();
  expect((await handleBillingPortal(post("/v1/billing/portal", creds), makeEnv(), db, unset.fetcher)).status).toBe(200);
  expect(unset.calls[0].form.has("configuration")).toBe(false);
});

it("subscription_404_is_retried_not_recorded", async () => {
  const { db, account } = await seeded();
  const before = { ...db.rawAccount(account.id) };
  const evt = completed(account.id);
  const execute = vi.spyOn(db, "execute");
  const missing = stripeFake({});
  const res = await deliver(db, missing.fetcher, evt);
  expect(res.status).toBe(500);
  const writes = execute.mock.calls.map(([q]) => q).filter((q) => !q.trimStart().startsWith("SELECT"));
  expect(writes).toEqual([]);
  expect(db.rawAccount(account.id)).toEqual(before);
  const up = stripeFake({ sub_1: subscription(account.id) });
  expect((await deliver(db, up.fetcher, evt)).status).toBe(200);
  expect(up.calls).toHaveLength(1);
  expect(db.rawAccount(account.id)?.plan).toBe("cloud");
});

it("duplicate_cancel_failure_is_500_without_email_then_retry_cancels_and_alerts_once", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: PERIOD_END * 1000, planInterval: "month", stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const before = { ...db.rawAccount(account.id) };
  const older = subscription(account.id, { created: NOW / 1000 - 1000 });
  const newer = subscription(account.id, { id: "sub_2", created: NOW / 1000 - 10, items: { data: [{ current_period_end: PERIOD_END + 365 * 86_400, price: { id: "price_yearly", recurring: { interval: "year" } } }] } });
  const e = makeEnv({ RESEND_API_KEY: "re_test" });
  const evt = subEvent("customer.subscription.created", "sub_2");
  const execute = vi.spyOn(db, "execute");
  const failing = stripeFake({ sub_1: older, sub_2: newer }, 200, 500);
  const res = await deliver(db, failing.fetcher, evt, e);
  expect(res.status).toBe(500);
  expect(failing.calls.filter((c) => c.method === "DELETE")).toHaveLength(1);
  expect(failing.calls.some((c) => c.url.startsWith("https://api.resend.com/"))).toBe(false);
  expect(execute.mock.calls.map(([q]) => q).filter((q) => !q.trimStart().startsWith("SELECT"))).toEqual([]);
  expect(db.rawAccount(account.id)).toEqual(before);
  const ok = stripeFake({ sub_1: older, sub_2: newer });
  expect((await deliver(db, ok.fetcher, evt, e)).status).toBe(200);
  expect(ok.calls.filter((c) => c.method === "DELETE").map((c) => c.url)).toEqual(["https://api.stripe.com/v1/subscriptions/sub_2"]);
  expect(ok.calls.filter((c) => c.url.startsWith("https://api.resend.com/"))).toHaveLength(1);
  expect(db.rawAccount(account.id)?.stripeSubscriptionId).toBe("sub_1");
  // Recorded: a third delivery does nothing.
  const third = stripeFake({ sub_1: older, sub_2: newer });
  expect((await deliver(db, third.fetcher, evt, e)).status).toBe(200);
  expect(third.calls).toHaveLength(0);
});

// ---- Account deletion cancels the subscription ----
async function cloudAccount(subId: string | null = "sub_1") {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, stripeCustomerId: "cus_1", stripeSubscriptionId: subId });
  await accounts.claimForDeletion(db, account.id, NOW);
  return { db, account };
}

/** Dispatches on host: Stripe answers `stripeStatus`, Resend 200. Records every call in order. */
function stripeAndResend(stripeStatus: number, stripeBody: unknown = {}) {
  const calls: string[] = [];
  const emails: { to: string[]; subject: string; text: string }[] = [];
  const fetcher = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    const u = String(url);
    if (u.startsWith("https://api.stripe.com/")) {
      calls.push(`${init?.method} ${u}`);
      return new Response(JSON.stringify(stripeBody), { status: stripeStatus });
    }
    calls.push("email");
    emails.push(JSON.parse(String(init?.body)));
    return new Response("{}", { status: 200 });
  }) as unknown as typeof fetch;
  return { calls, emails, fetcher };
}

function trackTurso(calls: string[]) {
  return vi.spyOn(turso, "deleteAccountDatabase").mockImplementation(async () => void calls.push("turso"));
}

it("deleting_a_cloud_account_cancels_the_subscription_first", async () => {
  const { db, account } = await cloudAccount();
  const s = stripeAndResend(200, { id: "sub_1", status: "canceled" });
  trackTurso(s.calls);
  await deleteAccount(db, makeEnv(), account, s.fetcher);
  expect(s.calls[0]).toBe("DELETE https://api.stripe.com/v1/subscriptions/sub_1");
  expect(s.calls).toContain("turso");
  expect(s.calls.indexOf("turso")).toBeGreaterThan(0);
  expect(s.emails).toHaveLength(0);
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("stripe_failure_still_deletes_and_alerts", async () => {
  const { db, account } = await cloudAccount();
  const s = stripeAndResend(500);
  trackTurso(s.calls);
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  await deleteAccount(db, makeEnv({ RESEND_API_KEY: "re_x" }), account, s.fetcher);
  expect(db.rawAccount(account.id)).toBeUndefined();
  expect(s.calls).toContain("turso");
  expect(s.emails).toHaveLength(1);
  expect(s.emails[0].to).toEqual(["hello@whimbrelapp.com"]);
  expect(s.emails[0].subject).toBe("Cancel a Whimbrel Cloud subscription by hand");
  expect(s.emails[0].text).toContain("sub_1");
  expect(s.emails[0].text).toContain("cus_1");
  expect(s.emails[0].text).not.toContain("owner@example.com");
  expect(err).toHaveBeenCalledWith("billing: cancel failed", 500, "sub_1");
});

it("no_secret_key_still_deletes_and_alerts", async () => {
  const { db, account } = await cloudAccount();
  const s = stripeAndResend(200);
  trackTurso(s.calls);
  vi.spyOn(console, "error").mockImplementation(() => {});
  await deleteAccount(db, makeEnv({ STRIPE_SECRET_KEY: undefined, RESEND_API_KEY: "re_x" }), account, s.fetcher);
  expect(s.calls.filter((c) => c.startsWith("DELETE"))).toHaveLength(0);
  expect(s.emails).toHaveLength(1);
  expect(s.emails[0].text).toContain("sub_1");
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("a_missing_subscription_on_delete_alerts", async () => {
  const { db, account } = await cloudAccount();
  const s = stripeAndResend(404, { error: { code: "resource_missing" } });
  trackTurso(s.calls);
  vi.spyOn(console, "error").mockImplementation(() => {});
  await deleteAccount(db, makeEnv({ RESEND_API_KEY: "re_x" }), account, s.fetcher);
  expect(s.emails).toHaveLength(1);
  expect(s.emails[0].text).toContain("sub_1");
  expect(s.emails[0].text).toContain("not found");
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("an_unsent_alert_logs_the_ids", async () => {
  const { db, account } = await cloudAccount();
  const s = stripeAndResend(500);
  trackTurso(s.calls);
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  await deleteAccount(db, makeEnv(), account, s.fetcher);
  expect(err).toHaveBeenCalledWith("billing: cancel alert not sent", "account", account.id, "subscription", "sub_1", "customer", "cus_1");
});

it("a_retried_deletion_after_a_failed_cancel_alerts_once", async () => {
  const { db, account } = await cloudAccount();
  const s = stripeAndResend(500);
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.spyOn(turso, "deleteAccountDatabase").mockRejectedValueOnce(new Error("turso down"));
  const e = makeEnv({ RESEND_API_KEY: "re_x" });
  await expect(deleteAccount(db, e, account, s.fetcher)).rejects.toThrow();
  trackTurso(s.calls);
  await deleteAccount(db, e, account, s.fetcher);
  expect(s.emails).toHaveLength(1);
  expect(s.calls.filter((c) => c.startsWith("DELETE"))).toHaveLength(1);
});

it("deleting_an_unpaid_account_still_cancels_its_subscription", async () => {
  const { db, account } = await seeded();
  // An unpaid (or paused) subscription ended the plan but still exists in Stripe.
  db.setPlan(account.id, { plan: "free", planUntilUnixMs: NOW - 1000, stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  await accounts.claimForDeletion(db, account.id, NOW);
  const s = stripeAndResend(200, { id: "sub_1", status: "unpaid" });
  trackTurso(s.calls);
  await deleteAccount(db, makeEnv({ RESEND_API_KEY: "re_x" }), account, s.fetcher);
  expect(s.calls).toContain("DELETE https://api.stripe.com/v1/subscriptions/sub_1");
  expect(s.emails).toHaveLength(0);
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("deleting_an_account_whose_subscription_already_ended_does_not_cancel", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "free", planUntilUnixMs: NOW - 1000, stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  await accounts.claimForDeletion(db, account.id, NOW);
  const s = stripeAndResend(200, { id: "sub_1", status: "canceled" });
  trackTurso(s.calls);
  await deleteAccount(db, makeEnv({ RESEND_API_KEY: "re_x" }), account, s.fetcher);
  expect(s.calls.filter((c) => c.startsWith("DELETE"))).toHaveLength(0);
  expect(s.emails).toHaveLength(0);
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("a_free_account_deletes_without_calling_stripe", async () => {
  const { db, account } = await seeded();
  await accounts.claimForDeletion(db, account.id, NOW);
  const s = stripeAndResend(200);
  trackTurso(s.calls);
  await deleteAccount(db, makeEnv({ RESEND_API_KEY: "re_x" }), account, s.fetcher);
  expect(s.calls).toEqual(["turso"]);
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("a_cloud_account_without_a_subscription_id_deletes_without_calling_stripe", async () => {
  const { db, account } = await cloudAccount(null);
  const s = stripeAndResend(200);
  trackTurso(s.calls);
  await deleteAccount(db, makeEnv({ RESEND_API_KEY: "re_x" }), account, s.fetcher);
  expect(s.calls).toEqual(["turso"]);
});

it("a_retried_deletion_skips_the_cancel", async () => {
  const { db, account } = await cloudAccount();
  const s = stripeAndResend(200);
  vi.spyOn(turso, "deleteAccountDatabase").mockRejectedValueOnce(new Error("turso down"));
  await expect(deleteAccount(db, makeEnv(), account, s.fetcher)).rejects.toThrow();
  trackTurso(s.calls);
  await deleteAccount(db, makeEnv(), account, s.fetcher);
  expect(s.calls.filter((c) => c.startsWith("DELETE"))).toHaveLength(1);
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("the_self_serve_route_passes_its_fetcher", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000, stripeCustomerId: "cus_1", stripeSubscriptionId: "sub_1" });
  const s = stripeAndResend(200);
  trackTurso(s.calls);
  const res = await handleDeleteAccount(post("/account/delete", { ...creds, devicePublicKey: "mac-key", confirm: true }), makeEnv(), db, s.fetcher);
  expect(res.status).toBe(200);
  expect(s.calls[0]).toBe("DELETE https://api.stripe.com/v1/subscriptions/sub_1");
});

it("a_webhook_for_a_deleted_account_returns_200_and_writes_nothing", async () => {
  const { db, account } = await cloudAccount();
  trackTurso([]);
  await deleteAccount(db, makeEnv(), account, stripeAndResend(200).fetcher);
  const stripe = stripeFake({ sub_1: { id: "sub_1", status: "canceled", metadata: { account_id: account.id } } });
  const res = await deliver(db, stripe.fetcher, { id: "evt_late", type: "customer.subscription.deleted", data: { object: { id: "sub_1", metadata: { account_id: account.id } } } });
  expect(res.status).toBe(200);
  expect(db.rawAccount(account.id)).toBeUndefined();
});

// ---- Whimbrel AI credit ----

const aiCheckout = (db: FakeSqlExecutor, fetcher: typeof fetch, body: object = {}, e: Env = makeEnv()) =>
  handleAiCheckout(post("/v1/ai/checkout", { ...creds, pack: "10", ...body }), e, db, fetcher, NOW);
const aiBalanceRoute = (db: FakeSqlExecutor) => handleAiBalance(post("/v1/ai/balance", creds), makeEnv(), db, NOW);

it("ai_checkout_sends_payment_mode_with_metadata_on_both", async () => {
  const { db, account } = await seeded();
  const { fetcher, calls } = fakeFetch();
  const res = await aiCheckout(db, fetcher);
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ url: "https://checkout.stripe.com/c/test" });
  expect(calls).toHaveLength(1);
  expect(calls[0].url).toBe("https://api.stripe.com/v1/checkout/sessions");
  const f = calls[0].form;
  expect(f.get("mode")).toBe("payment");
  expect(f.get("line_items[0][price_data][currency]")).toBe("usd");
  expect(f.get("line_items[0][price_data][unit_amount]")).toBe("1000");
  expect(f.get("line_items[0][price_data][product_data][name]")).toBe("Whimbrel AI credit");
  expect(f.get("line_items[0][price_data][tax_behavior]")).toBe("exclusive");
  expect(f.get("line_items[0][quantity]")).toBe("1");
  expect(f.get("metadata[kind]")).toBe("ai_credit");
  expect(f.get("metadata[account_id]")).toBe(account.id);
  expect(f.get("payment_intent_data[metadata][kind]")).toBe("ai_credit");
  expect(f.get("payment_intent_data[metadata][account_id]")).toBe(account.id);
  expect(f.get("automatic_tax[enabled]")).toBe("true");
  expect(f.get("adaptive_pricing[enabled]")).toBe("false");
  expect(f.get("billing_address_collection")).toBe("required");
  expect(f.get("success_url")).toBe("https://whimbrelapp.com/ai/thanks/");
  expect(f.get("cancel_url")).toBe("https://whimbrelapp.com/ai/cancelled/");
  expect(f.get("line_items[0][price_data][product_data][tax_code]")).toBe("txcd_10000000");
  expect(f.get("customer_email")).toBe("owner@example.com");
  expect(f.has("customer_creation")).toBe(false);
  expect(f.has("customer")).toBe(false);
  expect(f.has("subscription_data[metadata][account_id]")).toBe(false);
});

it("ai_checkout_pack_25_is_2500_and_reuses_the_customer", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "free", planUntilUnixMs: null, stripeCustomerId: "cus_9" });
  const { fetcher, calls } = fakeFetch();
  expect((await aiCheckout(db, fetcher, { pack: "25" })).status).toBe(200);
  const f = calls[0].form;
  expect(f.get("line_items[0][price_data][unit_amount]")).toBe("2500");
  expect(f.get("customer")).toBe("cus_9");
  expect(f.get("customer_update[address]")).toBe("auto");
  expect(f.has("customer_email")).toBe(false);
  expect(f.has("customer_creation")).toBe(false);
});

it("ai_checkout_rejects_unknown_pack", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = fakeFetch();
  for (const pack of ["5", 10, undefined, "100"]) {
    const res = await aiCheckout(db, fetcher, { pack });
    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "pack_required" });
  }
  expect(calls).toHaveLength(0);
});

it("ai_checkout_is_open_to_cloud_accounts_and_shares_the_billing_rate_limit", async () => {
  const { db, account } = await seeded();
  db.setPlan(account.id, { plan: "cloud", planUntilUnixMs: NOW + 1000 });
  const { fetcher } = fakeFetch();
  for (let i = 0; i < 20; i++) expect((await aiCheckout(db, fetcher)).status).toBe(200);
  expect((await aiCheckout(db, fetcher)).status).toBe(429);
  // The balance has its own bucket: a spent checkout limit never blocks it.
  expect((await aiBalanceRoute(db)).status).toBe(200);
});

it("ai_balance_polling_never_blocks_checkout_and_is_limited_to_120_per_hour", async () => {
  const { db } = await seeded();
  const statuses = [];
  for (let i = 0; i < 121; i++) statuses.push((await aiBalanceRoute(db)).status);
  expect(statuses.slice(0, 120).every((s) => s === 200)).toBe(true);
  expect(statuses[120]).toBe(429);
  expect((await aiCheckout(db, fakeFetch().fetcher)).status).toBe(200);
});

it("ai_checkout_503_without_billing_settings_and_502_on_stripe_failure", async () => {
  const { db } = await seeded();
  const { fetcher, calls } = fakeFetch();
  expect((await aiCheckout(db, fetcher, {}, makeEnv({ STRIPE_SECRET_KEY: undefined }))).status).toBe(503);
  expect(calls).toHaveLength(0);
  const leak = () => new Response(JSON.stringify({ error: { message: "sk_live_leak" } }), { status: 500 });
  const res = await aiCheckout(db, fakeFetch(leak).fetcher);
  expect(res.status).toBe(502);
  expect(await res.text()).not.toContain("leak");
});

it("ai_balance_reports_balance_and_last_purchase", async () => {
  const { db, account } = await seeded();
  const empty = await aiBalanceRoute(db);
  expect(empty.status).toBe(200);
  expect(await empty.json()).toEqual({ balance_micros: 0, last_purchase_micros: 0 });
  await creditPurchase(db, account.id, 25_000_000, "cs_a", NOW - 10);
  await creditPurchase(db, account.id, 10_000_000, "cs_b", NOW);
  expect(await (await aiBalanceRoute(db)).json()).toEqual({ balance_micros: 35_000_000, last_purchase_micros: 10_000_000 });
});

it("ai_balance_without_an_account_is_403", async () => {
  const res = await handleAiBalance(post("/v1/ai/balance", creds), makeEnv(), new FakeSqlExecutor(), NOW);
  expect(res.status).toBe(403);
});

const aiSession = (accountId: string, over: Record<string, unknown> = {}, type = "checkout.session.completed") =>
  event(type, {
    id: "cs_ai_1",
    object: "checkout.session",
    mode: "payment",
    payment_status: "paid",
    payment_intent: "pi_1",
    customer: "cus_1",
    subscription: null,
    currency: "usd",
    amount_subtotal: 1000,
    amount_total: 1080,
    total_details: { amount_discount: 0, amount_shipping: 0, amount_tax: 80 },
    metadata: { kind: "ai_credit", account_id: accountId },
    ...over,
  });

/** Serves `GET /v1/payment_intents/{id}`, `GET /v1/refunds?charge=…` and
 * Resend sends. Anything else is a 404. Records every URL, and each email. */
function aiStripeFake(intents: Record<string, object> = {}, refunds: Record<string, object[]> = {}, status = 200) {
  const calls: string[] = [];
  const emails: any[] = [];
  const fetcher = (async (u: string, init: RequestInit) => {
    if (u.startsWith("https://api.resend.com/")) {
      emails.push(JSON.parse(String(init.body)));
      return new Response('{"id":"e"}', { status: 200 });
    }
    calls.push(`${init.method ?? "GET"} ${u}`);
    if (status !== 200) return new Response('{"error":{"message":"down"}}', { status });
    const url = new URL(u);
    const pi = url.pathname.split("/v1/payment_intents/")[1];
    if (pi && intents[pi]) return new Response(JSON.stringify(intents[pi]), { status: 200 });
    const charge = url.pathname === "/v1/refunds" ? url.searchParams.get("charge") : null;
    if (charge && refunds[charge]) return new Response(JSON.stringify({ object: "list", data: refunds[charge], has_more: false }), { status: 200 });
    return new Response('{"error":{"code":"resource_missing"}}', { status: 404 });
  }) as unknown as typeof fetch;
  return { fetcher, calls, emails };
}

const refundOf = (id: string, amount: number, status = "succeeded") => ({ id, object: "refund", amount, status });
const refundList = (...data: object[]) => ({ object: "list", data, has_more: false });
/** A $10 pack with $0.80 tax: the charge is 1080 cents. */
const chargeRefunded = (over: Record<string, unknown>) =>
  event("charge.refunded", { id: "ch_1", object: "charge", payment_intent: "pi_1", amount: 1080, metadata: {}, ...over });

/** Buys a $10 pack (10,000,000 micros) on pi_1 and $25 of other credit on pi_other. */
async function boughtTen() {
  const { db, account } = await seeded();
  await creditPurchase(db, account.id, 25_000_000, "pi_other", NOW - 1);
  await deliver(db, aiStripeFake().fetcher, aiSession(account.id));
  expect(await aiBalance(db, account.id)).toBe(35_000_000);
  return { db, account };
}

it("ai_purchase_credits_the_pretax_subtotal_keyed_on_the_payment_intent", async () => {
  const { db, account } = await seeded();
  const s = aiStripeFake();
  const res = await deliver(db, s.fetcher, aiSession(account.id));
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ received: true });
  expect(await aiBalance(db, account.id)).toBe(10_000_000);
  expect(db.rawAiLedger()).toEqual([expect.objectContaining({ kind: "purchase", micros: 10_000_000, stripe_ref: "pi_1" })]);
  expect(s.calls).toHaveLength(0);
  expect(db.rawAccount(account.id)?.plan ?? "free").toBe("free");
});

it("duplicate_purchase_event_credits_once", async () => {
  const { db, account } = await seeded();
  const s = aiStripeFake();
  const evt = aiSession(account.id);
  expect((await deliver(db, s.fetcher, evt)).status).toBe(200);
  expect((await deliver(db, s.fetcher, evt)).status).toBe(200);
  // A new event id for the same payment: creditPurchase's own idempotency.
  expect((await deliver(db, s.fetcher, { ...evt, id: "evt_redelivered" })).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(10_000_000);
  expect(db.rawAiLedger().filter((r) => r.kind === "purchase")).toHaveLength(1);
});

it("non_ai_payment_session_is_ignored", async () => {
  const { db, account } = await seeded();
  const s = aiStripeFake();
  const res = await deliver(db, s.fetcher, aiSession(account.id, { metadata: { account_id: account.id } }));
  expect(res.status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(0);
  expect(db.rawAiLedger()).toHaveLength(0);
  expect(s.calls).toHaveLength(0);
});

it("unpaid_ai_session_is_ignored", async () => {
  const { db, account } = await seeded();
  const res = await deliver(db, aiStripeFake().fetcher, aiSession(account.id, { payment_status: "unpaid" }));
  expect(res.status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(0);
  expect(db.rawAiLedger()).toHaveLength(0);
});

it("async_payment_succeeded_credits_a_delayed_payment_once", async () => {
  const { db, account } = await seeded();
  const s = aiStripeFake();
  expect((await deliver(db, s.fetcher, aiSession(account.id, { payment_status: "unpaid" }))).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(0);
  const paid = aiSession(account.id, {}, "checkout.session.async_payment_succeeded");
  expect((await deliver(db, s.fetcher, paid)).status).toBe(200);
  expect((await deliver(db, s.fetcher, { ...paid, id: "evt_again" })).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(10_000_000);
  expect(db.rawAiLedger()).toHaveLength(1);
});

it("paid_session_for_a_missing_or_deleting_account_alerts_and_writes_nothing", async () => {
  const { db, account } = await seeded();
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const e = makeEnv({ RESEND_API_KEY: "re_test" });
  const s = aiStripeFake();
  expect((await deliver(db, s.fetcher, aiSession("acct_gone"), e)).status).toBe(200);
  expect(await accounts.claimForDeletion(db, account.id, NOW)).toBe(true);
  expect((await deliver(db, s.fetcher, aiSession(account.id, { id: "cs_ai_2", payment_intent: "pi_2" }), e)).status).toBe(200);
  expect(db.rawAiLedger()).toHaveLength(0);
  expect(s.emails).toHaveLength(2);
  for (const m of s.emails) {
    expect(m.to).toEqual(["hello@whimbrelapp.com"]);
    expect(m.subject).toBe("Refund a Whimbrel AI credit purchase that was not credited");
    expect(m.text).toContain("10.80 USD");
  }
  expect(s.emails[0].text).toContain("cs_ai_1");
  expect(s.emails[0].text).toContain("account missing");
  expect(s.emails[1].text).toContain("cs_ai_2");
  expect(s.emails[1].text).toContain("account being deleted");
  expect(JSON.stringify(s.emails)).not.toContain("owner@example.com");
  expect(err).toHaveBeenCalled();
});

it("an_account_deleted_between_lookup_and_credit_alerts_and_credits_nothing", async () => {
  const { db, account } = await seeded();
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  // The account is read as live, then deleted before the credit lands.
  db.after("SELECT id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms, plan, plan_until_unix_ms, plan_interval, stripe_customer_id, stripe_subscription_id, plan_renews FROM account WHERE id = ?", async () => {
    expect(await accounts.claimForDeletion(db, account.id, NOW)).toBe(true);
    await accounts.deleteAccountRows(db, account.id);
  });
  const s = aiStripeFake();
  const evt = aiSession(account.id);
  expect((await deliver(db, s.fetcher, evt, makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(db.rawAiLedger()).toHaveLength(0);
  expect(s.emails).toHaveLength(1);
  expect(s.emails[0].subject).toBe("Refund a Whimbrel AI credit purchase that was not credited");
  expect(s.emails[0].text).toContain("cs_ai_1");
  expect(s.emails[0].text).toContain("pi_1");
  // Recorded once alerted: a redelivery does not alert again.
  expect((await deliver(db, s.fetcher, evt, makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(s.emails).toHaveLength(1);
  expect(err).toHaveBeenCalled();
});

it("an_account_claimed_for_deletion_between_lookup_and_credit_is_not_credited", async () => {
  const { db, account } = await seeded();
  vi.spyOn(console, "error").mockImplementation(() => {});
  db.after("SELECT id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms, plan, plan_until_unix_ms, plan_interval, stripe_customer_id, stripe_subscription_id, plan_renews FROM account WHERE id = ?", async () => {
    expect(await accounts.claimForDeletion(db, account.id, NOW)).toBe(true);
  });
  const s = aiStripeFake();
  expect((await deliver(db, s.fetcher, aiSession(account.id), makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(db.rawAiLedger()).toHaveLength(0);
  expect(await aiBalance(db, account.id)).toBe(0);
  expect(s.emails).toHaveLength(1);
});

it("an_uncredited_alert_that_fails_to_send_asks_stripe_to_retry", async () => {
  const { db } = await seeded();
  vi.spyOn(console, "error").mockImplementation(() => {});
  let resendUp = false;
  const s = aiStripeFake();
  const fetcher = (async (u: string, init: RequestInit) => {
    if (u.startsWith("https://api.resend.com/") && !resendUp) return new Response('{"message":"down"}', { status: 500 });
    return s.fetcher(u, init);
  }) as unknown as typeof fetch;
  const evt = aiSession("acct_gone");
  const e = makeEnv({ RESEND_API_KEY: "re_test" });
  // Resend down: not recorded, so Stripe redelivers and the alert is tried again.
  expect((await deliver(db, fetcher, evt, e)).status).toBe(500);
  expect(await accounts.stripeEventSeen(db, evt.id)).toBe(false);
  // No Resend key at all: the alert cannot go out either.
  expect((await deliver(db, fetcher, evt, makeEnv())).status).toBe(500);
  expect(await accounts.stripeEventSeen(db, evt.id)).toBe(false);
  resendUp = true;
  expect((await deliver(db, fetcher, evt, e)).status).toBe(200);
  expect(s.emails).toHaveLength(1);
  expect(await accounts.stripeEventSeen(db, evt.id)).toBe(true);
  expect((await deliver(db, fetcher, evt, e)).status).toBe(200);
  expect(s.emails).toHaveLength(1);
  expect(db.rawAiLedger()).toHaveLength(0);
});

it("ai_purchase_with_a_bad_amount_writes_nothing", async () => {
  const { db, account } = await seeded();
  vi.spyOn(console, "error").mockImplementation(() => {});
  const s = aiStripeFake();
  expect((await deliver(db, s.fetcher, aiSession(account.id, { amount_subtotal: "1000" }), makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(s.emails).toHaveLength(1);
  expect(db.rawAiLedger()).toHaveLength(0);
  expect(await aiBalance(db, account.id)).toBe(0);
});

it("full_refund_removes_the_pretax_credit_exactly", async () => {
  const { db, account } = await boughtTen();
  const s = aiStripeFake();
  expect((await deliver(db, s.fetcher, chargeRefunded({ refunds: refundList(refundOf("re_1", 1080)) }))).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(25_000_000);
  expect(db.rawAiLedger().filter((r) => r.kind === "refund")).toEqual([
    expect.objectContaining({ micros: -10_000_000, stripe_ref: "refund:re_1", shortfall_micros: 0 }),
  ]);
  expect(s.calls).toHaveLength(0);
});

it("half_refund_removes_half_the_credit", async () => {
  const { db, account } = await boughtTen();
  expect((await deliver(db, aiStripeFake().fetcher, chargeRefunded({ refunds: refundList(refundOf("re_1", 540)) }))).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(30_000_000);
});

it("refund_removes_only_unspent_credit", async () => {
  const { db, account } = await seeded();
  const s = aiStripeFake();
  await deliver(db, s.fetcher, aiSession(account.id));
  const hold = (await placeHold(db, account.id, 8_000_000, NOW))!;
  await settleHold(db, hold, { model: "m", inputTokens: 1, outputTokens: 1, costMicros: 7_000_000 }, NOW);
  expect(await aiBalance(db, account.id)).toBe(3_000_000);
  const res = await deliver(db, s.fetcher, chargeRefunded({ refunds: refundList(refundOf("re_1", 1080)) }));
  expect(res.status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(0);
  expect(db.rawAiLedger().filter((r) => r.kind === "refund")).toEqual([
    expect.objectContaining({ micros: -3_000_000, stripe_ref: "refund:re_1", shortfall_micros: 7_000_000 }),
  ]);
  expect(s.calls).toHaveLength(0);
});

it("refund_lists_the_refunds_when_the_charge_lacks_them", async () => {
  const { db, account } = await boughtTen();
  const s = aiStripeFake({}, { ch_1: [refundOf("re_1", 540)] });
  expect((await deliver(db, s.fetcher, chargeRefunded({}))).status).toBe(200);
  expect(s.calls).toEqual(["GET https://api.stripe.com/v1/refunds?charge=ch_1&limit=100"]);
  expect(await aiBalance(db, account.id)).toBe(30_000_000);
});

it("each_refund_on_a_charge_applies_once", async () => {
  const { db, account } = await boughtTen();
  const s = aiStripeFake();
  await deliver(db, s.fetcher, chargeRefunded({ refunds: refundList(refundOf("re_1", 270)) }));
  await deliver(db, s.fetcher, chargeRefunded({ refunds: refundList(refundOf("re_2", 540), refundOf("re_1", 270), refundOf("re_x", 999, "failed")) }));
  expect(await aiBalance(db, account.id)).toBe(35_000_000 - 2_500_000 - 5_000_000);
  expect(db.rawAiLedger().filter((r) => r.kind === "refund").map((r) => r.stripe_ref)).toEqual(["refund:re_1", "refund:re_2"]);
});

it("refund_before_purchase_is_retried_not_dropped", async () => {
  const { db, account } = await seeded();
  await creditPurchase(db, account.id, 25_000_000, "pi_other", NOW - 1);
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const s = aiStripeFake();
  const evt = chargeRefunded({ metadata: { kind: "ai_credit", account_id: account.id }, refunds: refundList(refundOf("re_1", 1080)) });
  const first = await deliver(db, s.fetcher, evt);
  expect(first.status).toBe(500);
  expect(err).toHaveBeenCalledWith("ai credit: refund with no purchase row yet", "ch_1", "payment", "pi_1");
  expect(db.rawAiLedger().filter((r) => r.kind === "refund")).toHaveLength(0);
  expect(await aiBalance(db, account.id)).toBe(25_000_000);
  // The purchase lands, then Stripe redelivers the same refund event.
  await deliver(db, s.fetcher, aiSession(account.id));
  expect(await aiBalance(db, account.id)).toBe(35_000_000);
  expect((await deliver(db, s.fetcher, evt)).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(25_000_000);
  expect(s.calls).toHaveLength(0);
});

/** Delivers `session` with the alert address configured; returns the emails. */
async function notCredited(over: Record<string, unknown>) {
  const { db, account } = await seeded();
  vi.spyOn(console, "error").mockImplementation(() => {});
  const s = aiStripeFake();
  const evt = aiSession(account.id, over);
  expect((await deliver(db, s.fetcher, evt, makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(db.rawAiLedger()).toHaveLength(0);
  expect(await aiBalance(db, account.id)).toBe(0);
  // Recorded: a redelivery of the same event is a no-op, not a second alert.
  expect((await deliver(db, s.fetcher, evt, makeEnv({ RESEND_API_KEY: "re_test" }))).status).toBe(200);
  expect(s.emails).toHaveLength(1);
  expect(s.emails[0].subject).toBe("Refund a Whimbrel AI credit purchase that was not credited");
  expect(s.emails[0].text).toContain("cs_ai_1");
  return s.emails[0].text as string;
}

it("non_usd_session_is_not_credited_and_alerts", async () => {
  const text = await notCredited({ currency: "eur", amount_subtotal: 930, amount_total: 1116 });
  expect(text).toContain("11.16 EUR");
});

it("odd_amount_is_not_credited", async () => {
  for (const amount_subtotal of [999, 2000, 0, 1000.5]) {
    const text = await notCredited({ amount_subtotal });
    expect(text).toContain("USD");
  }
});

it("discounted_session_is_not_credited", async () => {
  const text = await notCredited({ amount_subtotal: 1000, amount_total: 540, total_details: { amount_discount: 500, amount_tax: 40 } });
  expect(text).toContain("5.40 USD");
});

it("a_session_without_total_details_is_still_credited", async () => {
  const { db, account } = await seeded();
  expect((await deliver(db, aiStripeFake().fetcher, aiSession(account.id, { total_details: undefined }))).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(10_000_000);
});

it("refund_of_another_products_charge_is_ignored", async () => {
  const { db, account } = await seeded();
  await creditPurchase(db, account.id, 10_000_000, "pi_other", NOW);
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const s = aiStripeFake({ pi_1: { id: "pi_1", metadata: {} } }, { ch_1: [refundOf("re_1", 1080)] });
  expect((await deliver(db, s.fetcher, chargeRefunded({}))).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(10_000_000);
  expect(s.calls).toEqual(["GET https://api.stripe.com/v1/payment_intents/pi_1"]);
  expect(err).not.toHaveBeenCalled();
});

it("refund_retries_when_stripe_is_down", async () => {
  const { db, account } = await boughtTen();
  const evt = chargeRefunded({});
  vi.spyOn(console, "log").mockImplementation(() => {});
  expect((await deliver(db, aiStripeFake({}, {}, 503).fetcher, evt)).status).toBe(500);
  const s = aiStripeFake({}, { ch_1: [refundOf("re_1", 1080)] });
  expect((await deliver(db, s.fetcher, evt)).status).toBe(200);
  expect(await aiBalance(db, account.id)).toBe(25_000_000);
});
