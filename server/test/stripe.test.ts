import { it, expect } from "vitest";
import { billingConfigured, isAiCreditEvent, planFromSubscription, stripeRequest, subscriptionIdOf, verifyStripeSignature } from "../src/stripe";
import type { Env } from "../src/index";

const SECRET = "whsec_test_secret";
const NOW = 1_800_000_000_000;
const enc = new TextEncoder();

async function mac(body: string, secret: string, t: number): Promise<string> {
  const key = await crypto.subtle.importKey("raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const sig = new Uint8Array(await crypto.subtle.sign("HMAC", key, enc.encode(`${t}.${body}`)));
  return [...sig].map((b) => b.toString(16).padStart(2, "0")).join("");
}
async function sign(body: string, secret: string, t: number): Promise<string> {
  return `t=${t},v1=${await mac(body, secret, t)}`;
}

const T = NOW / 1000;

it("accepts_a_correctly_signed_body", async () => {
  expect(await verifyStripeSignature("{}", await sign("{}", SECRET, T), SECRET, NOW)).toBe(true);
});
it("rejects_a_tampered_body", async () => {
  expect(await verifyStripeSignature("{ }", await sign("{}", SECRET, T), SECRET, NOW)).toBe(false);
});
it("rejects_a_wrong_secret", async () => {
  expect(await verifyStripeSignature("{}", await sign("{}", "whsec_other", T), SECRET, NOW)).toBe(false);
});
it("rejects_a_signature_older_than_300_seconds", async () => {
  expect(await verifyStripeSignature("{}", await sign("{}", SECRET, T - 301), SECRET, NOW)).toBe(false);
  expect(await verifyStripeSignature("{}", await sign("{}", SECRET, T - 299), SECRET, NOW)).toBe(true);
  expect(await verifyStripeSignature("{}", await sign("{}", SECRET, T + 301), SECRET, NOW)).toBe(false);
});
it("rejects_a_missing_header", async () => {
  expect(await verifyStripeSignature("{}", null, SECRET, NOW)).toBe(false);
  expect(await verifyStripeSignature("{}", "", SECRET, NOW)).toBe(false);
  expect(await verifyStripeSignature("{}", `t=${T}`, SECRET, NOW)).toBe(false);
  expect(await verifyStripeSignature("{}", "v1=abcd", SECRET, NOW)).toBe(false);
  expect(await verifyStripeSignature("{}", `t=${T},v1=zz`, SECRET, NOW)).toBe(false);
});
it("accepts_when_any_v1_matches", async () => {
  const good = await mac("{}", SECRET, T);
  expect(await verifyStripeSignature("{}", `t=${T},v1=${"0".repeat(64)},v1=${good}`, SECRET, NOW)).toBe(true);
});

const END = 1_900_000_000;
function sub(status: string, extra: any = {}): any {
  return { status, items: { data: [{ current_period_end: END, price: { recurring: { interval: "month" } } }] }, ...extra };
}

it("past_due_stays_cloud_until_period_end", () => {
  expect(planFromSubscription(sub("past_due"), NOW)).toEqual({ plan: "cloud", untilUnixMs: END * 1000, interval: "month", renews: true });
  expect(planFromSubscription(sub("active"), NOW)?.plan).toBe("cloud");
  expect(planFromSubscription(sub("trialing"), NOW)?.plan).toBe("cloud");
});
it("a_cancelled_subscription_stays_cloud_until_it_ends_and_does_not_renew", () => {
  // The customer portal sets cancel_at (flexible billing) or cancel_at_period_end.
  expect(planFromSubscription(sub("active", { cancel_at: END }), NOW)).toEqual({ plan: "cloud", untilUnixMs: END * 1000, interval: "month", renews: false });
  expect(planFromSubscription(sub("active", { cancel_at_period_end: true }), NOW)?.renews).toBe(false);
  expect(planFromSubscription(sub("active", { cancel_at: END - 100 }), NOW)?.untilUnixMs).toBe((END - 100) * 1000);
  expect(planFromSubscription(sub("active", { cancel_at: null, cancel_at_period_end: false }), NOW)?.renews).toBe(true);
});
it("unpaid_and_canceled_end_the_plan_at_ended_at_or_now", () => {
  expect(planFromSubscription(sub("canceled", { ended_at: 1_799_000_000 }), NOW)).toEqual({ plan: "free", untilUnixMs: 1_799_000_000_000, interval: null, renews: false });
  expect(planFromSubscription(sub("unpaid"), NOW)).toEqual({ plan: "free", untilUnixMs: NOW, interval: null, renews: false });
  expect(planFromSubscription(sub("incomplete_expired"), NOW)?.plan).toBe("free");
  expect(planFromSubscription(sub("paused"), NOW)?.plan).toBe("free");
});
it("incomplete_changes_nothing", () => {
  expect(planFromSubscription(sub("incomplete"), NOW)).toBeNull();
  expect(planFromSubscription(sub("something_new"), NOW)).toBeNull();
});
it("reads_period_end_from_items_or_the_root", () => {
  const root = { status: "active", current_period_end: END, items: { data: [{ price: { recurring: { interval: "month" } } }] } };
  expect(planFromSubscription(root, NOW)?.untilUnixMs).toBe(END * 1000);
  expect(planFromSubscription(sub("active", { current_period_end: 5 }), NOW)?.untilUnixMs).toBe(END * 1000);
});
it("yearly_interval_is_read_from_the_price", () => {
  const s = sub("active");
  s.items.data[0].price.recurring.interval = "year";
  expect(planFromSubscription(s, NOW)?.interval).toBe("year");
});

it("subscription_id_from_invoice_parent_or_legacy_field", () => {
  expect(subscriptionIdOf({ type: "invoice.paid", data: { object: { parent: { subscription_details: { subscription: "sub_new" } }, subscription: "sub_old" } } })).toBe("sub_new");
  expect(subscriptionIdOf({ type: "invoice.payment_failed", data: { object: { subscription: "sub_old" } } })).toBe("sub_old");
  expect(subscriptionIdOf({ type: "checkout.session.completed", data: { object: { subscription: "sub_c" } } })).toBe("sub_c");
  expect(subscriptionIdOf({ type: "customer.subscription.updated", data: { object: { id: "sub_u" } } })).toBe("sub_u");
  expect(subscriptionIdOf({ type: "charge.succeeded", data: { object: { id: "ch_1" } } })).toBeNull();
});

it("stripe_request_sends_form_body_and_bearer", async () => {
  let url = "";
  let init: RequestInit = {};
  const fetcher = (async (u: string, i: RequestInit) => {
    url = u;
    init = i;
    return new Response(JSON.stringify({ id: "cs_1" }), { status: 200 });
  }) as unknown as typeof fetch;
  const r = await stripeRequest("sk_test_x", "POST", "checkout/sessions", { "line_items[0][price]": "price_m", mode: "subscription" }, fetcher);
  expect(r).toEqual({ ok: true, status: 200, body: { id: "cs_1" } });
  expect(url).toBe("https://api.stripe.com/v1/checkout/sessions");
  expect(init.method).toBe("POST");
  expect(new Headers(init.headers).get("authorization")).toBe("Bearer sk_test_x");
  expect(init.body).toBe("line_items%5B0%5D%5Bprice%5D=price_m&mode=subscription");
  expect(new Headers(init.headers).get("content-type")).toBe("application/x-www-form-urlencoded");
});
it("stripe_request_puts_get_params_in_the_query", async () => {
  let url = "";
  const fetcher = (async (u: string) => ((url = u), new Response("{}", { status: 404 }))) as unknown as typeof fetch;
  const r = await stripeRequest("k", "GET", "subscriptions/sub_1", { a: "b" }, fetcher);
  expect(url).toBe("https://api.stripe.com/v1/subscriptions/sub_1?a=b");
  expect(r.ok).toBe(false);
  expect(r.status).toBe(404);
});
it("network_failure_returns_status_0", async () => {
  const fetcher = (async () => {
    throw new Error("boom");
  }) as unknown as typeof fetch;
  expect(await stripeRequest("k", "DELETE", "customers/c", {}, fetcher)).toEqual({ ok: false, status: 0, body: null });
});

it("billingConfigured_needs_all_four", () => {
  const all = { STRIPE_SECRET_KEY: "a", STRIPE_WEBHOOK_SECRET: "b", STRIPE_PRICE_MONTHLY: "c", STRIPE_PRICE_YEARLY: "d" };
  expect(billingConfigured(all as unknown as Env)).toBe(true);
  expect(billingConfigured({ ...all, STRIPE_PRICE_YEARLY: "" } as unknown as Env)).toBe(false);
  expect(billingConfigured({} as Env)).toBe(false);
});

it("stripe_request_refuses_redirects_so_the_key_never_leaves", async () => {
  let init: RequestInit | undefined;
  const fetcher = (async (_u: string, i: RequestInit) => {
    init = i;
    return new Response(null, { status: 302, headers: { location: "https://evil.example/" } });
  }) as unknown as typeof fetch;
  expect(await stripeRequest("sk_test_x", "POST", "checkout/sessions", {}, fetcher)).toEqual({ ok: false, status: 302, body: null });
  expect(init?.redirect).toBe("manual");
  // The Workers runtime accepts these options (it throws on redirect: "error").
  expect(() => new Request("https://api.stripe.com/v1/checkout/sessions", init)).not.toThrow();
});

it("isAiCreditEvent_needs_payment_mode_and_the_ai_credit_kind", () => {
  const ev = (type: string, object: object) => ({ id: "evt", type, data: { object } });
  const ai = { kind: "ai_credit", account_id: "a" };
  expect(isAiCreditEvent(ev("checkout.session.completed", { mode: "payment", metadata: ai }))).toBe(true);
  expect(isAiCreditEvent(ev("checkout.session.async_payment_succeeded", { mode: "payment", metadata: ai }))).toBe(true);
  expect(isAiCreditEvent(ev("checkout.session.completed", { mode: "subscription", metadata: ai }))).toBe(false);
  expect(isAiCreditEvent(ev("checkout.session.completed", { mode: "payment", metadata: {} }))).toBe(false);
  expect(isAiCreditEvent(ev("charge.refunded", { metadata: ai }))).toBe(true);
  expect(isAiCreditEvent(ev("charge.refunded", { metadata: {} }))).toBe(false);
  expect(isAiCreditEvent(ev("invoice.paid", { metadata: ai }))).toBe(false);
  expect(isAiCreditEvent(null)).toBe(false);
});
