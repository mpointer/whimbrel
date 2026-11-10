import { it, expect, vi, beforeEach } from "vitest";
import * as googleAuth from "../src/googleAuth";
import * as turso from "../src/turso";
import * as accounts from "../src/accounts";
import { handleAccountPlan, handleBillingCheckout, handleBillingPortal, handleBillingWebhook, handleDeleteAccount, handleGoogleAuth } from "../src/index";
import { deleteAccount } from "../src/inactivity";
import type { Env } from "../src/index";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";
import { env } from "cloudflare:test";

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
  for (const p of ["/v1/account/plan", "/v1/billing/checkout", "/v1/billing/portal"]) {
    const res = await worker.fetch(new Request(`https://example.com${p}`, { method: "POST" }), makeEnv());
    expect(res.status).toBe(400);
  }
  const hook = await worker.fetch(new Request("https://example.com/v1/billing/webhook", { method: "POST", body: "{}" }), makeEnv());
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
