import { afterEach, expect, it, vi } from "vitest";
import * as accounts from "../src/accounts";
import { DEFAULT_YEARLY_GRANTS_PER_RUN, YEAR_BACK_MS, addMonthsUtc, readYearlyGrantsPerRun, aiBalance, creditPurchase, grantAllowance, grantDueYearlyMonths, monthWindowEnd, zeroAllowance } from "../src/aiCredit";
import { handleBillingWebhook } from "../src/index";
import type { SqlExecutor } from "../src/accounts";
import type { Env } from "../src/index";
import { realSqlExecutor } from "./testHelpers/realSqlExecutor";

// The Plus allowance's money flow through the real webhook handler against
// REAL SQLite: the grant on a paid Plus invoice, and the zero on an
// immediate cancel or a refunded Plus invoice.

const NOW = Date.UTC(2026, 10, 19, 9, 0, 0);
const DAY = 86_400;
const START = NOW / 1000 - 60; // the subscription began a minute ago
const LINE_END = START + 30 * DAY; // the Plus line's period end (seconds)
const SETTING = 6_000_000;

let open: (SqlExecutor & { close(): void })[] = [];
afterEach(() => {
  for (const db of open) db.close();
  open = [];
  vi.restoreAllMocks();
});

const env = {
  GOOGLE_CLIENT_ID: "test-client-id",
  ALLOWED_GOOGLE_EMAILS: "owner@example.com",
  STRIPE_SECRET_KEY: "sk_test_x",
  STRIPE_WEBHOOK_SECRET: "whsec_test_secret",
  STRIPE_PRICE_MONTHLY: "price_monthly",
  STRIPE_PRICE_YEARLY: "price_yearly",
  STRIPE_PRICE_PLUS_MONTHLY: "price_plus_monthly",
  STRIPE_PRICE_PLUS_YEARLY: "price_plus_yearly",
} as unknown as Env;

async function account() {
  const db = realSqlExecutor();
  open.push(db);
  const a = await accounts.getOrCreateAccount(db, "owner@example.com", "g-1", 1, "db", "https://db.test");
  return { db, id: a.id };
}

function plusSub(accountId: string, over: Record<string, unknown> = {}, price = "price_plus_monthly", periodEnd = LINE_END) {
  return {
    id: "sub_1",
    object: "subscription",
    status: "active",
    customer: "cus_1",
    created: START,
    metadata: { account_id: accountId, tier: "plus" },
    items: { data: [{ id: "si_1", current_period_end: periodEnd, price: { id: price, recurring: { interval: "month" } } }] },
    ...over,
  };
}

function invoice(over: Record<string, unknown> = {}, line: { start?: number; end?: number; price?: string; item?: string; sub?: string; amount?: number } = {}) {
  const sub = line.sub ?? "sub_1";
  return {
    id: "in_1",
    object: "invoice",
    billing_reason: "subscription_create",
    amount_paid: 1500,
    // The first invoice's top-level period is zero-length.
    period_start: START,
    period_end: START,
    parent: { subscription_details: { subscription: sub } },
    lines: {
      data: [
        {
          id: "il_1",
          ...(line.amount === undefined ? {} : { amount: line.amount }),
          period: { start: line.start ?? START, end: line.end ?? LINE_END },
          pricing: { price_details: { price: line.price ?? "price_plus_monthly" } },
          parent: { type: "subscription_item_details", subscription_item_details: { subscription_item: line.item ?? "si_1", proration: false } },
        },
      ],
    },
    ...over,
  };
}

/** Serves subscriptions, invoices, invoice payments and payment intents
 * from `objects` (keyed by id); DELETE cancels; Resend accepts. */
function stripe(objects: Record<string, any>, invoicePayments: Record<string, string> = {}) {
  const calls: string[] = [];
  const fetcher = (async (u: string, init: RequestInit) => {
    const method = init.method ?? "GET";
    if (u.startsWith("https://api.resend.com/")) return new Response('{"id":"e"}', { status: 200 });
    calls.push(`${method} ${u}`);
    const url = new URL(u);
    if (url.pathname === "/v1/invoice_payments") {
      const inv = invoicePayments[url.searchParams.get("payment[payment_intent]") ?? ""];
      return Response.json({ object: "list", data: inv ? [{ object: "invoice_payment", invoice: inv }] : [], has_more: false });
    }
    if (url.pathname === "/v1/refunds") return Response.json({ object: "list", data: [], has_more: false });
    const id = decodeURIComponent(url.pathname.split("/").pop()!);
    // A PaymentIntent we were not given is a plain one (not AI credit).
    const o = objects[id] ?? (url.pathname.startsWith("/v1/payment_intents/") ? { id, object: "payment_intent", metadata: {} } : undefined);
    if (!o) return Response.json({ error: { code: "resource_missing" } }, { status: 404 });
    return Response.json(method === "DELETE" ? { ...o, status: "canceled" } : o);
  }) as unknown as typeof fetch;
  return { fetcher, calls };
}

const enc = new TextEncoder();
async function signature(body: string, t: number): Promise<string> {
  const key = await crypto.subtle.importKey("raw", enc.encode("whsec_test_secret"), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const sig = new Uint8Array(await crypto.subtle.sign("HMAC", key, enc.encode(`${t}.${body}`)));
  return `t=${t},v1=${[...sig].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
}

let seq = 0;
const event = (type: string, object: object, id = `evt_${++seq}`) => ({ id, object: "event", type, data: { object } });
const paid = (inv: object, id?: string) => event("invoice.paid", inv, id);
const subEvent = (type: string, id = "sub_1") => event(type, { id, object: "subscription" });

async function deliver(db: SqlExecutor, fetcher: typeof fetch, evt: object, now = NOW) {
  const body = JSON.stringify(evt);
  const req = new Request("https://example.com/v1/billing/webhook", { method: "POST", body, headers: { "stripe-signature": await signature(body, now / 1000) } });
  return handleBillingWebhook(req, env, db, fetcher, now);
}

async function allowance(db: SqlExecutor, id: string) {
  const r = (await db.execute("SELECT ai_allowance_micros, ai_allowance_expires_ms, plan, plan_tier FROM account WHERE id = ?", [id]))[0];
  return { micros: Number(r.ai_allowance_micros), expires: r.ai_allowance_expires_ms === null ? null : Number(r.ai_allowance_expires_ms), plan: r.plan, tier: r.plan_tier };
}
/** Real grants; a zero's marker rows (`zeroed:…`) are counted by markers(). */
const grants = async (db: SqlExecutor) => Number((await db.execute("SELECT COUNT(*) AS n FROM ai_allowance_grant WHERE grant_key NOT LIKE 'zeroed:%'", []))[0].n);
const markers = async (db: SqlExecutor) =>
  db.execute("SELECT grant_key, account_id, micros, period_end_unix_ms, created_at_unix_ms, subscription_id FROM ai_allowance_grant WHERE grant_key LIKE 'zeroed:%' ORDER BY rowid", []);
const recorded = async (db: SqlExecutor, evtId: string) => (await db.execute("SELECT 1 FROM stripe_event WHERE id = ?", [evtId])).length > 0;

it("first_invoice_grants_once_for_subscription_create", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  // checkout.session.completed makes the plan but grants nothing itself.
  const completed = event("checkout.session.completed", { id: "cs_1", object: "checkout.session", mode: "subscription", subscription: "sub_1", client_reference_id: id });
  expect((await deliver(db, fetcher, completed)).status).toBe(200);
  expect(await grants(db)).toBe(0);
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  expect(await grants(db)).toBe(1);
  const row = (await db.execute("SELECT grant_key, account_id, micros, period_end_unix_ms FROM ai_allowance_grant", []))[0];
  expect(row).toEqual({ grant_key: "in_1", account_id: id, micros: SETTING, period_end_unix_ms: LINE_END * 1000 });
});

it("the_configured_allowance_setting_is_what_is_granted", async () => {
  const { db, id } = await account();
  await db.execute("INSERT INTO server_setting (key, value) VALUES ('plus_allowance_micros', '7500000')", []);
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(7_500_000);
});

it("a_renewal_invoice_grants_for_subscription_cycle", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  // Spend some of month 1.
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  const later = (LINE_END + 60) * 1000;
  const next = LINE_END + 30 * DAY;
  const renewed = plusSub(id, {}, "price_plus_monthly", next);
  // A renewal's top-level period is the month just ended.
  const inv = invoice({ id: "in_2", billing_reason: "subscription_cycle", period_start: START, period_end: LINE_END }, { start: LINE_END, end: next });
  expect((await deliver(db, stripe({ sub_1: renewed }).fetcher, paid(inv), later)).status).toBe(200);
  // Replaced, not added: no rollover.
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: next * 1000, plan: "cloud", tier: "plus" });
  expect(await grants(db)).toBe(2);
});

it("subscription_update_invoices_grant_nothing", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  for (const billing_reason of ["subscription_update", "manual", "subscription_threshold", "upcoming", undefined]) {
    expect((await deliver(db, fetcher, paid(invoice({ id: `in_${billing_reason}`, billing_reason })))).status).toBe(200);
  }
  expect(await grants(db)).toBe(0);
  expect(await allowance(db, id)).toEqual({ micros: 0, expires: null, plan: "cloud", tier: "plus" });
});

// CHANGED ON PURPOSE (referral plan PR 3): this used to be "a_zero_amount_invoice_grants_nothing"
// and pinned that EVERY amount_paid of 0 gets no grant. A $0 invoice now grants when it has a positive
// Plus line (see the $0 tests below). What still grants nothing: a bad amount, and a $0 invoice with
// nothing priced on it.
it("an_unusable_amount_or_a_zero_invoice_with_nothing_priced_grants_nothing", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  const cases: [Record<string, unknown>, number | undefined][] = [
    [{ amount_paid: -5, subtotal: 1500 }, 1500], // negative
    [{ amount_paid: undefined, subtotal: 1500 }, 1500], // missing
    [{ amount_paid: "0", subtotal: 1500 }, 1500], // a string
    [{ amount_paid: 0.5, subtotal: 1500 }, 1500], // fractional
    [{ amount_paid: 0 }, 1500], // $0, no subtotal sent
    [{ amount_paid: 0, subtotal: 0 }, 0], // $0, free trial
    [{ amount_paid: 0, subtotal: 500 }, 0], // $0, subtotal from other lines, Plus line $0
    [{ amount_paid: 0, subtotal: 1500 }, undefined], // $0, Plus line amount missing
    [{ amount_paid: 0, subtotal: 1500 }, -1500], // $0, Plus line negative
  ];
  for (const [n, [over, amount]] of cases.entries()) {
    expect((await deliver(db, fetcher, paid(invoice({ id: `in_z${n}`, ...over }, { amount })))).status).toBe(200);
  }
  expect(await grants(db)).toBe(0);
  expect((await allowance(db, id)).micros).toBe(0);
});

/** A paid $0 invoice: a promotion code or the customer's credit balance covered the $15 Plus line. */
const zeroInvoice = (over: Record<string, unknown> = {}, line: Parameters<typeof invoice>[1] = {}) =>
  invoice({ amount_paid: 0, subtotal: 1500, total: 0, ...over }, { amount: 1500, ...line });

it("a_zero_dollar_paid_invoice_with_a_positive_plus_line_grants_once", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  expect((await deliver(db, fetcher, paid(zeroInvoice()))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  expect(await grants(db)).toBe(1);
  expect(await rowOf(db, "in_1")).toMatchObject({ micros: SETTING, period_end_unix_ms: LINE_END * 1000, subscription_id: "sub_1" });
  // Redelivered (the same event, then a new event for the same invoice): still one grant, no refill.
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  const evt = paid(zeroInvoice());
  expect((await deliver(db, fetcher, evt)).status).toBe(200);
  expect((await deliver(db, fetcher, evt)).status).toBe(200);
  expect((await deliver(db, fetcher, paid(zeroInvoice()))).status).toBe(200);
  expect(await grants(db)).toBe(1);
  expect((await allowance(db, id)).micros).toBe(1_000_000);
});

it("a_yearly_zero_dollar_invoice_grants_its_first_month", async () => {
  const { db, id } = await account();
  const yearEnd = START + 365 * DAY;
  const sub = plusSub(id, {}, "price_plus_yearly", yearEnd);
  sub.items.data[0].price.recurring.interval = "year";
  const inv = zeroInvoice({ subtotal: 14400 }, { price: "price_plus_yearly", end: yearEnd, amount: 14400 });
  expect((await deliver(db, stripe({ sub_1: sub }).fetcher, paid(inv))).status).toBe(200);
  expect(await grants(db)).toBe(1);
  expect((await allowance(db, id)).micros).toBe(SETTING);
  expect((await allowance(db, id)).expires).toBeLessThan(yearEnd * 1000);
});

it("a_renewal_paid_from_the_credit_balance_grants_and_replaces_the_allowance", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  const next = LINE_END + 30 * DAY;
  const inv = zeroInvoice({ id: "in_2", billing_reason: "subscription_cycle", period_start: START, period_end: LINE_END, starting_balance: -1500, ending_balance: 0 }, { start: LINE_END, end: next });
  expect((await deliver(db, stripe({ sub_1: plusSub(id, {}, "price_plus_monthly", next) }).fetcher, paid(inv), (LINE_END + 60) * 1000)).status).toBe(200);
  // Replaced, not added: no rollover.
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: next * 1000, plan: "cloud", tier: "plus" });
  expect(await grants(db)).toBe(2);
});

it("a_zero_dollar_invoice_only_grants_for_the_reasons_that_grant_when_paid", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  for (const billing_reason of ["subscription_update", "manual", "subscription_threshold", undefined]) {
    expect((await deliver(db, fetcher, paid(zeroInvoice({ id: `in_${billing_reason}`, billing_reason })))).status).toBe(200);
  }
  expect(await grants(db)).toBe(0);
});

it("a_zero_dollar_invoice_of_a_cloud_or_another_products_subscription_grants_nothing", async () => {
  // Cloud on a retired price: not Plus.
  const { db, id } = await account();
  const cloud = plusSub(id, { metadata: { account_id: id } }, "price_monthly");
  expect((await deliver(db, stripe({ sub_1: cloud }).fetcher, paid(zeroInvoice({}, { price: "price_monthly" })))).status).toBe(200);
  expect(await grants(db)).toBe(0);
  expect(await allowance(db, id)).toEqual({ micros: 0, expires: null, plan: "cloud", tier: "cloud" });
  // Another product on the shared Stripe account: a subscription with no Whimbrel price or metadata.
  const { db: db2, id: id2 } = await account();
  const other = { id: "sub_fma", object: "subscription", status: "active", customer: "cus_x", metadata: { product: "fma" }, items: { data: [{ id: "si_x", current_period_end: LINE_END, price: { id: "price_fma", recurring: { interval: "month" } } }] } };
  const inv = zeroInvoice({ id: "in_fma", parent: { subscription_details: { subscription: "sub_fma" } } }, { price: "price_fma", item: "si_x" });
  expect((await deliver(db2, stripe({ sub_fma: other }).fetcher, paid(inv))).status).toBe(200);
  expect(await grants(db2)).toBe(0);
  expect((await allowance(db2, id2)).micros).toBe(0);
});

it("a_zero_dollar_invoice_without_a_readable_plus_line_is_retried_not_granted", async () => {
  const { db, id } = await account();
  vi.spyOn(console, "error").mockImplementation(() => {});
  const inv = zeroInvoice({ lines: { data: [] } });
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(inv))).status).toBeGreaterThanOrEqual(500);
  expect(await grants(db)).toBe(0);
});

it("a_full_refund_of_last_months_paid_invoice_keeps_this_months_zero_dollar_grant", async () => {
  const { db, id } = await account();
  const next = LINE_END + 30 * DAY;
  const in2 = zeroInvoice({ id: "in_2", billing_reason: "subscription_cycle", period_start: START, period_end: LINE_END }, { start: LINE_END, end: next });
  const { fetcher } = stripe({ sub_1: plusSub(id, {}, "price_plus_monthly", next), in_1: invoice(), in_2: in2 }, { pi_1: "in_1" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect((await deliver(db, fetcher, paid(in2), LINE_END * 1000 + 60_000)).status).toBe(200);
  vi.spyOn(console, "log").mockImplementation(() => {});
  expect((await deliver(db, fetcher, refundOf("pi_1"), LINE_END * 1000 + 2 * HOUR)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: next * 1000, plan: "cloud", tier: "plus" });
  expect(await markers(db)).toEqual([]);
});

it("a_full_refund_of_the_charge_behind_a_zero_dollar_grant_zeroes_it_and_a_partial_one_does_not", async () => {
  // A $0-paid invoice is normally charge-less; this pins that if Stripe ever reports a charge for it
  // (say, tax collected on a credit-paid invoice), the existing refund rule applies to its grant row.
  const { db, id } = await account();
  const in1 = zeroInvoice();
  const { fetcher } = stripe({ sub_1: plusSub(id), in_1: in1 }, { pi_inv: "in_1" });
  expect((await deliver(db, fetcher, paid(in1))).status).toBe(200);
  expect((await deliver(db, fetcher, event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 500, payment_intent: "pi_inv" }), NOW + 60_000)).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(SETTING);
  expect((await deliver(db, fetcher, fullRefund(), NOW + 120_000)).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(0);
  expect(await grants(db)).toBe(1);
});

it("a_refund_that_lands_before_a_zero_dollar_invoice_is_paid_leaves_it_with_no_grant", async () => {
  const { db, id } = await account();
  const in1 = zeroInvoice();
  const { fetcher } = stripe({ sub_1: plusSub(id), in_1: in1 }, { pi_inv: "in_1" });
  vi.spyOn(console, "error").mockImplementation(() => {});
  // The subscription is already attached to the account (checkout completed); only the invoice is unpaid.
  const completed = event("checkout.session.completed", { id: "cs_1", object: "checkout.session", mode: "subscription", subscription: "sub_1", client_reference_id: id });
  expect((await deliver(db, fetcher, completed)).status).toBe(200);
  expect((await deliver(db, fetcher, fullRefund())).status).toBe(200);
  expect(await rowOf(db, "in_1")).toMatchObject({ micros: 0 }); // the tombstone
  expect((await deliver(db, fetcher, paid(in1), NOW + 60_000)).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(0);
  expect(await rowOf(db, "in_1")).toMatchObject({ micros: 0 }); // still only the tombstone
});

it("a_cloud_invoice_grants_nothing", async () => {
  const { db, id } = await account();
  const cloud = plusSub(id, { metadata: { account_id: id } }, "price_monthly");
  expect((await deliver(db, stripe({ sub_1: cloud }).fetcher, paid(invoice({}, { price: "price_monthly" })))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: 0, expires: null, plan: "cloud", tier: "cloud" });
  expect(await grants(db)).toBe(0);
});

it("a_duplicate_plus_subscription_being_cancelled_never_grants", async () => {
  const { db, id } = await account();
  // The account is on the older Plus subscription sub_1, already granted.
  const older = plusSub(id, { created: START - 1000 });
  expect((await deliver(db, stripe({ sub_1: older }).fetcher, paid(invoice()))).status).toBe(200);
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  // A second Plus subscription's first invoice is paid: it is cancelled, never granted.
  const newer = plusSub(id, { id: "sub_2", created: START, items: { data: [{ id: "si_2", current_period_end: LINE_END + DAY, price: { id: "price_plus_monthly", recurring: { interval: "month" } } }] } });
  const { fetcher, calls } = stripe({ sub_1: older, sub_2: newer });
  vi.spyOn(console, "error").mockImplementation(() => {});
  const inv = invoice({ id: "in_dup" }, { sub: "sub_2", item: "si_2", end: LINE_END + DAY });
  expect((await deliver(db, fetcher, paid(inv))).status).toBe(200);
  expect(calls).toContain("DELETE https://api.stripe.com/v1/subscriptions/sub_2");
  expect(await grants(db)).toBe(1);
  expect(await allowance(db, id)).toEqual({ micros: 1_000_000, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  expect((await accounts.findAccountById(db, id))?.stripeSubscriptionId).toBe("sub_1");
});

it("the_grant_expires_at_the_invoice_lines_period_end", async () => {
  const { db, id } = await account();
  // A renewal-shaped invoice whose top-level period (the month just ended,
  // still in the future here) differs from the Plus line's period.
  const topEnd = LINE_END + 5 * DAY;
  const inv = invoice({ billing_reason: "subscription_cycle", period_start: START, period_end: topEnd }, { start: START, end: LINE_END });
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(inv))).status).toBe(200);
  const a = await allowance(db, id);
  expect(a.expires).toBe(LINE_END * 1000);
  expect(a.expires).not.toBe(topEnd * 1000);
  // A retired Plus price still finds its line through the subscription item.
  const { db: db2, id: id2 } = await account();
  const retired = plusSub(id2, {}, "price_plus_2025");
  const inv2 = invoice({}, { price: "price_plus_2025", item: "si_1", end: LINE_END + DAY });
  expect((await deliver(db2, stripe({ sub_1: retired }).fetcher, paid(inv2))).status).toBe(200);
  expect(await allowance(db2, id2)).toMatchObject({ micros: SETTING, expires: (LINE_END + DAY) * 1000 });
});

it("a_repeated_event_grants_once", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  const evt = paid(invoice());
  expect((await deliver(db, fetcher, evt)).status).toBe(200);
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  // The same event again (de-duplicated) and the same invoice under a new event id (grant key).
  expect((await deliver(db, fetcher, evt)).status).toBe(200);
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect(await grants(db)).toBe(1);
  expect((await allowance(db, id)).micros).toBe(1_000_000);
});

it("invoice_paid_before_subscription_created_is_safe", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  expect((await deliver(db, fetcher, subEvent("customer.subscription.created"))).status).toBe(200);
  const completed = event("checkout.session.completed", { id: "cs_1", object: "checkout.session", mode: "subscription", subscription: "sub_1", client_reference_id: id });
  expect((await deliver(db, fetcher, completed)).status).toBe(200);
  expect(await grants(db)).toBe(1);
  expect(await allowance(db, id)).toEqual({ micros: 1_000_000, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
});

it("another_products_invoice_is_ignored", async () => {
  const { db, id } = await account();
  const before = await allowance(db, id);
  const other = plusSub("fma_account_1", { metadata: { account_id: "fma_account_1" } }, "price_other_product");
  const { fetcher, calls } = stripe({ sub_1: other, in_o: invoice({ id: "in_o" }, { price: "price_other_product" }) }, { pi_o: "in_o" });
  vi.spyOn(console, "log").mockImplementation(() => {});
  expect((await deliver(db, fetcher, paid(invoice({ id: "in_o" }, { price: "price_other_product" })))).status).toBe(200);
  // Its refunded charge is acknowledged and changes nothing.
  expect((await deliver(db, fetcher, event("charge.refunded", { id: "ch_o", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_o" }))).status).toBe(200);
  expect(await grants(db)).toBe(0);
  expect(await allowance(db, id)).toEqual(before);
  expect(calls.every((c) => c.startsWith("GET "))).toBe(true);
});

it("a_grant_failure_makes_the_webhook_retry", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) });
  const realBatch = db.batch.bind(db);
  let fail = true;
  db.batch = async (stmts) => {
    if (fail && stmts.some((s) => s.sql.startsWith("INSERT INTO ai_allowance_grant"))) throw new Error("turso down");
    return realBatch(stmts);
  };
  vi.spyOn(console, "error").mockImplementation(() => {});
  const evt = paid(invoice());
  const res = await deliver(db, fetcher, evt);
  expect(res.status).toBeGreaterThanOrEqual(500);
  expect(await recorded(db, evt.id)).toBe(false);
  expect(await grants(db)).toBe(0);
  // Stripe redelivers the same event: it grants now.
  fail = false;
  expect((await deliver(db, fetcher, evt)).status).toBe(200);
  expect(await recorded(db, evt.id)).toBe(true);
  expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: LINE_END * 1000 });
  // A paid Plus invoice with no readable Plus line is retried, not dropped.
  const noLine = paid(invoice({ id: "in_noline", lines: { data: [] } }));
  expect((await deliver(db, fetcher, noLine)).status).toBe(500);
  expect(await recorded(db, noLine.id)).toBe(false);
});

it("an_immediate_cancel_zeroes_the_allowance", async () => {
  for (const status of ["canceled", "unpaid", "incomplete_expired", "paused"]) {
    const { db, id } = await account();
    expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
    const ended = plusSub(id, { status, ended_at: NOW / 1000 });
    expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"))).status).toBe(200);
    const a = await allowance(db, id);
    expect(a).toMatchObject({ micros: 0, plan: "free", tier: null });
    // "cancel" mode: the expiry is the cancel time + 1 ms, not past the old
    // window end, so a new subscription bought straight after always grants.
    // The ended subscription's late invoices never reach a grant (its plan is free).
    expect(a.expires).toBe(NOW + 1);
  }
});

it("a_stale_cancel_of_another_subscription_zeroes_nothing", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  const old = plusSub(id, { id: "sub_old", status: "canceled", ended_at: NOW / 1000 });
  expect((await deliver(db, stripe({ sub_old: old }).fetcher, subEvent("customer.subscription.deleted", "sub_old"))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
});

it("a_cancel_at_period_end_keeps_the_allowance_until_expiry", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  for (const over of [{ cancel_at_period_end: true }, { cancel_at: LINE_END }]) {
    const cancelling = plusSub(id, over);
    expect((await deliver(db, stripe({ sub_1: cancelling }).fetcher, subEvent("customer.subscription.updated"))).status).toBe(200);
    expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
    expect((await accounts.findAccountById(db, id))?.planRenews).toBe(false);
  }
});

it("a_refunded_plus_invoice_zeroes_the_allowance_and_a_prepaid_refund_does_not", async () => {
  const { db, id } = await account();
  await creditPurchase(db, id, 10_000_000, "pi_pack", NOW - 1000);
  const objects = {
    sub_1: plusSub(id),
    in_1: invoice(),
    pi_pack: { id: "pi_pack", metadata: { kind: "ai_credit", account_id: id } },
  };
  const { fetcher, calls } = stripe(objects, { pi_inv: "in_1" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  // A prepaid pack refund: prepaid goes down, the allowance is untouched, no invoice lookup.
  const packRefund = event("charge.refunded", {
    id: "ch_pack",
    object: "charge",
    amount: 1000,
    amount_refunded: 1000,
    payment_intent: "pi_pack",
    metadata: { kind: "ai_credit" },
    refunds: { data: [{ id: "re_pack", amount: 1000, status: "succeeded" }], has_more: false },
  });
  expect((await deliver(db, fetcher, packRefund)).status).toBe(200);
  expect(await aiBalance(db, id)).toBe(0);
  expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: LINE_END * 1000 });
  expect(calls.some((c) => c.includes("invoice_payments") || c.includes("/invoices/"))).toBe(false);
  // The Plus invoice's charge is refunded: the allowance goes to 0, prepaid is unchanged.
  await creditPurchase(db, id, 5_000_000, "pi_pack2", NOW);
  const plusRefund = event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_inv" });
  expect((await deliver(db, fetcher, plusRefund)).status).toBe(200);
  expect(calls).toContain("GET https://api.stripe.com/v1/invoices/in_1");
  const a = await allowance(db, id);
  expect(a.micros).toBe(0);
  expect(a.expires).toBeGreaterThan(LINE_END * 1000);
  expect(await aiBalance(db, id)).toBe(5_000_000);
  expect(a.plan).toBe("cloud");
});

it("a_refunded_plus_charge_with_a_legacy_invoice_field_zeroes_the_allowance", async () => {
  const { db, id } = await account();
  const { fetcher, calls } = stripe({ sub_1: plusSub(id), in_1: invoice() });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  const refund = event("charge.refunded", { id: "ch_1", object: "charge", amount: 1500, amount_refunded: 1500, invoice: "in_1", payment_intent: "pi_x" });
  expect((await deliver(db, fetcher, refund)).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(0);
  expect(calls.some((c) => c.includes("invoice_payments"))).toBe(false);
});

it("a_refund_of_a_cancelled_duplicate_does_not_zero_the_kept_allowance", async () => {
  const { db, id } = await account();
  const dup = plusSub(id, { id: "sub_2", status: "canceled" });
  const { fetcher } = stripe({ sub_1: plusSub(id), sub_2: dup, in_dup: invoice({ id: "in_dup" }, { sub: "sub_2" }) }, { pi_dup: "in_dup" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  vi.spyOn(console, "log").mockImplementation(() => {});
  expect((await deliver(db, fetcher, event("charge.refunded", { id: "ch_dup", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_dup" }))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
});

it("a_refund_lookup_failure_is_retried", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id) }, { pi_inv: "in_missing" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  vi.spyOn(console, "log").mockImplementation(() => {});
  const refund = event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_inv" });
  expect((await deliver(db, fetcher, refund)).status).toBe(500);
  expect(await recorded(db, refund.id)).toBe(false);
  expect((await allowance(db, id)).micros).toBe(SETTING);
});

it("a_late_invoice_after_a_zero_does_not_restore_the_allowance", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id), in_1: invoice() }, { pi_inv: "in_1" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect((await deliver(db, fetcher, event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_inv" }))).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(0);
  // The old period's invoice again (new event id), and another invoice for the same period.
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect((await deliver(db, fetcher, paid(invoice({ id: "in_late" })))).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(0);
});

it("a_partial_refund_of_a_plus_invoice_does_not_zero_the_allowance", async () => {
  const { db, id } = await account();
  const { fetcher, calls } = stripe({ sub_1: plusSub(id), in_1: invoice() }, { pi_inv: "in_1" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  const before = calls.length;
  const partial = event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 500, payment_intent: "pi_inv" });
  expect((await deliver(db, fetcher, partial)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  expect(calls.slice(before).some((c) => c.includes("invoice"))).toBe(false);
  // The rest refunded later makes it a full refund: now it zeroes.
  const full = event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_inv" });
  expect((await deliver(db, fetcher, full)).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(0);
});

const YEAR_END = START + 365 * DAY;
function yearlySub(accountId: string, over: Record<string, unknown> = {}, end = YEAR_END) {
  return plusSub(accountId, {
    items: { data: [{ id: "si_1", current_period_end: end, price: { id: "price_plus_yearly", recurring: { interval: "year" } } }] },
    ...over,
  });
}
const yearlyInvoice = () => invoice({}, { price: "price_plus_yearly", start: START, end: YEAR_END });
// START is 2026-11-19 08:59:00 UTC.
const MONTH_1_END = Date.UTC(2026, 11, 19, 8, 59, 0);
const MONTH_2_END = Date.UTC(2027, 0, 19, 8, 59, 0);

it("a_yearly_first_invoice_grant_expires_at_the_end_of_month_one", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: yearlySub(id) }).fetcher, paid(yearlyInvoice()))).status).toBe(200);
  const a = await allowance(db, id);
  expect(a).toEqual({ micros: SETTING, expires: MONTH_1_END, plan: "cloud", tier: "plus" });
  expect(a.expires).not.toBe(YEAR_END * 1000);
  const row = (await db.execute("SELECT period_end_unix_ms FROM ai_allowance_grant WHERE grant_key = 'in_1'", []))[0];
  expect(Number(row.period_end_unix_ms)).toBe(MONTH_1_END);
});

it("a_yearly_month_two_grant_is_not_blocked_by_the_month_one_expiry", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: yearlySub(id) }).fetcher, paid(yearlyInvoice()))).status).toBe(200);
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  const later = MONTH_1_END + 60_000;
  const end2 = monthWindowEnd(START * 1000, 2, YEAR_END * 1000);
  expect(end2).toBe(MONTH_2_END);
  await grantAllowance(db, { accountId: id, grantKey: "in_1:m2", micros: SETTING, periodEndUnixMs: end2 }, later);
  expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: MONTH_2_END });
});

it("a_monthly_grant_still_expires_at_the_line_period_end", async () => {
  // Billing anchored on the 31st: the period that starts on 28 Feb ends on
  // 31 Mar, later than one calendar month from its start (28 Mar).
  const { db, id } = await account();
  const start = Date.UTC(2027, 1, 28, 9, 0, 0) / 1000;
  const end = Date.UTC(2027, 2, 31, 9, 0, 0) / 1000;
  expect(monthWindowEnd(start * 1000, 1, end * 1000)).toBe(Date.UTC(2027, 2, 28, 9, 0, 0));
  const sub = plusSub(id, {}, "price_plus_monthly", end);
  const inv = invoice({ billing_reason: "subscription_cycle", period_start: start - 28 * DAY, period_end: start }, { start, end });
  expect((await deliver(db, stripe({ sub_1: sub }).fetcher, paid(inv), (start + 60) * 1000)).status).toBe(200);
  expect((await allowance(db, id)).expires).toBe(end * 1000);
  // And the first month of the usual case: the line's own end.
  const { db: db2, id: id2 } = await account();
  expect((await deliver(db2, stripe({ sub_1: plusSub(id2) }).fetcher, paid(invoice()))).status).toBe(200);
  expect((await allowance(db2, id2)).expires).toBe(LINE_END * 1000);
});

// ---- review fixes: line selection, logging, more zero/duplicate cases, zero markers ----

const prorationLine = (over: Record<string, unknown> = {}) => ({
  id: "il_pr",
  // A pending proration invoice item that landed on the renewal, on the Plus price, for a period already ended.
  period: { start: START - 10 * DAY, end: START - DAY },
  pricing: { price_details: { price: "price_plus_monthly" } },
  parent: { type: "invoice_item_details", invoice_item_details: { invoice_item: "ii_1", proration: true, subscription: "sub_1" } },
  ...over,
});

it("a_renewal_whose_first_plus_price_line_is_an_invoice_item_proration_grants_from_the_real_line", async () => {
  const { db, id } = await account();
  const inv = invoice({ billing_reason: "subscription_cycle", period_start: START - 30 * DAY, period_end: START });
  inv.lines.data.unshift(prorationLine() as never);
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(inv))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  expect(err).not.toHaveBeenCalled();
  // A subscription-item proration line first is skipped too, as is a legacy-shape proration.
  const { db: db2, id: id2 } = await account();
  const inv2 = invoice({ billing_reason: "subscription_cycle" });
  inv2.lines.data.unshift(
    prorationLine({ parent: { type: "subscription_item_details", subscription_item_details: { subscription_item: "si_1", proration: true } } }) as never,
    legacyProrationLine() as never,
    legacyInvoiceItemLine() as never,
  );
  expect((await deliver(db2, stripe({ sub_1: plusSub(id2) }).fetcher, paid(inv2))).status).toBe(200);
  expect((await allowance(db2, id2)).expires).toBe(LINE_END * 1000);
  // Any invoice item, even one not marked as a proration, is never the Plus line.
  const { db: db3, id: id3 } = await account();
  const inv3 = invoice({ billing_reason: "subscription_cycle" });
  inv3.lines.data.unshift(prorationLine({ parent: { type: "invoice_item_details", invoice_item_details: { invoice_item: "ii_2", proration: false, subscription: "sub_1" } } }) as never);
  expect((await deliver(db3, stripe({ sub_1: plusSub(id3) }).fetcher, paid(inv3))).status).toBe(200);
  expect((await allowance(db3, id3)).expires).toBe(LINE_END * 1000);
});

// Legacy (pre-basil, no `parent`) lines on the Plus price for a period
// already ended: picking either one would grant nothing, so each skip in
// plusLinePeriod's legacy branch has its own test below.
const legacyProrationLine = () => ({
  id: "il_legacy_pr",
  type: "subscription",
  proration: true,
  period: { start: START - 10 * DAY, end: START - DAY },
  price: { id: "price_plus_monthly" },
  subscription_item: "si_1",
});
const legacyInvoiceItemLine = () => ({
  id: "il_legacy_ii",
  type: "invoiceitem",
  proration: false,
  period: { start: START - 10 * DAY, end: START - DAY },
  price: { id: "price_plus_monthly" },
});
const legacyRealLine = () => ({ id: "il_1", type: "subscription", proration: false, period: { start: START, end: LINE_END }, price: { id: "price_plus_monthly" }, subscription_item: "si_1" });

it("a_legacy_proration_line_is_never_the_plus_line", async () => {
  const { db, id } = await account();
  const inv = invoice({ billing_reason: "subscription_cycle" });
  inv.lines.data = [legacyProrationLine(), legacyRealLine()] as never;
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(inv))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
});

it("a_legacy_invoice_item_line_is_never_the_plus_line", async () => {
  const { db, id } = await account();
  const inv = invoice({ billing_reason: "subscription_cycle" });
  inv.lines.data = [legacyInvoiceItemLine(), legacyRealLine()] as never;
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(inv))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
});

it("a_cloud_immediate_cancel_writes_no_marker_and_touches_no_allowance", async () => {
  const { db, id } = await account();
  const cloud = plusSub(id, { metadata: { account_id: id } }, "price_monthly");
  expect((await deliver(db, stripe({ sub_1: cloud }).fetcher, paid(invoice({}, { price: "price_monthly" })))).status).toBe(200);
  // An allowance left on the account (say, from an earlier Plus plan) makes a stray zero visible.
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_old", micros: 2_000_000, periodEndUnixMs: LINE_END * 1000 }, NOW)).toBe(true);
  const ended = plusSub(id, { metadata: { account_id: id }, status: "canceled", ended_at: NOW / 1000 }, "price_monthly");
  expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: 2_000_000, expires: LINE_END * 1000, plan: "free", tier: null });
  expect(await markers(db)).toEqual([]);
  // And with no allowance at all, the expiry stays NULL.
  const { db: db2, id: id2 } = await account();
  expect((await deliver(db2, stripe({ sub_1: plusSub(id2, { metadata: { account_id: id2 } }, "price_monthly") }).fetcher, paid(invoice({}, { price: "price_monthly" })))).status).toBe(200);
  const ended2 = plusSub(id2, { metadata: { account_id: id2 }, status: "canceled", ended_at: NOW / 1000 }, "price_monthly");
  expect((await deliver(db2, stripe({ sub_1: ended2 }).fetcher, subEvent("customer.subscription.deleted"))).status).toBe(200);
  expect(await allowance(db2, id2)).toEqual({ micros: 0, expires: null, plan: "free", tier: null });
  expect(await markers(db2)).toEqual([]);
});

it("an_immediate_cancel_zeroes_when_either_the_stored_tier_or_the_subscription_says_plus", async () => {
  // The stored tier is plus, but the ended subscription no longer reads as Plus.
  {
    const { db, id } = await account();
    expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
    const ended = plusSub(id, { metadata: { account_id: id }, status: "canceled", ended_at: NOW / 1000 }, "price_monthly");
    expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"))).status).toBe(200);
    expect(await allowance(db, id)).toMatchObject({ micros: 0, plan: "free", tier: null });
    expect((await markers(db)).length).toBe(1);
  }
  // The stored tier is not plus yet, but the ended subscription is Plus by its price.
  {
    const { db, id } = await account();
    const cloud = plusSub(id, { metadata: { account_id: id } }, "price_monthly");
    expect((await deliver(db, stripe({ sub_1: cloud }).fetcher, paid(invoice({}, { price: "price_monthly" })))).status).toBe(200);
    expect(await grantAllowance(db, { accountId: id, grantKey: "in_seed", micros: SETTING, periodEndUnixMs: LINE_END * 1000 }, NOW)).toBe(true);
    const ended = plusSub(id, { metadata: { account_id: id }, status: "canceled", ended_at: NOW / 1000 }, "price_plus_monthly");
    expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"))).status).toBe(200);
    expect(await allowance(db, id)).toMatchObject({ micros: 0, plan: "free", tier: null });
    expect((await markers(db)).length).toBe(1);
  }
});

it("a_legacy_line_without_parent_still_grants", async () => {
  const { db, id } = await account();
  const inv = invoice({}, {});
  inv.lines.data = [{ id: "il_1", type: "subscription", proration: false, period: { start: START, end: LINE_END }, price: { id: "price_plus_monthly" }, subscription_item: "si_1" } as never];
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(inv))).status).toBe(200);
  expect((await allowance(db, id)).expires).toBe(LINE_END * 1000);
});

it("an_invoice_with_only_proration_lines_on_the_plus_price_grants_nothing_and_logs", async () => {
  const { db, id } = await account();
  const inv = invoice({ billing_reason: "subscription_cycle" });
  inv.lines.data = [prorationLine() as never];
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const evt = paid(inv);
  // Logged and left unrecorded for Stripe to retry (a paid allowance is never dropped silently).
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, evt)).status).toBe(500);
  expect(await recorded(db, evt.id)).toBe(false);
  expect(await grants(db)).toBe(0);
  expect(err).toHaveBeenCalledWith("billing: paid plus invoice without a plus line period", "in_1");
});

it("a_paid_grant_for_an_ended_period_is_logged_not_retried", async () => {
  const { db, id } = await account();
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const evt = paid(invoice());
  // Delivered after the line's period has ended (a very late invoice.paid).
  const late = (LINE_END + 60) * 1000;
  const sub = plusSub(id, {}, "price_plus_monthly", LINE_END + 30 * DAY);
  expect((await deliver(db, stripe({ sub_1: sub }).fetcher, evt, late)).status).toBe(200);
  expect(await recorded(db, evt.id)).toBe(true);
  expect(await grants(db)).toBe(0);
  expect(err).toHaveBeenCalledWith("billing: paid plus invoice for a period already ended, no grant", "in_1");
});

it("silent_no_grant_cases_are_logged", async () => {
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  // Paid from the credit balance, with no priced subtotal on the payload (a priced one now grants).
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice({ amount_paid: 0 })))).status).toBe(200);
  expect(log).toHaveBeenCalledWith("billing: plus invoice paid with no amount, no grant", "in_1");
  // A subscription status that maps to no plan.
  const { db: db2, id: id2 } = await account();
  expect((await deliver(db2, stripe({ sub_1: plusSub(id2, { status: "incomplete" }) }).fetcher, paid(invoice()))).status).toBe(200);
  expect(log).toHaveBeenCalledWith("billing: paid plus invoice, subscription status gives no plan, no grant", "sub_1", "incomplete");
  // A Plus subscription without account_id metadata.
  const { db: db3 } = await account();
  expect((await deliver(db3, stripe({ sub_1: plusSub("x", { metadata: { tier: "plus" } }) }).fetcher, paid(invoice()))).status).toBe(200);
  expect(log).toHaveBeenCalledWith("billing: paid plus invoice, subscription has no account_id metadata, no grant", "sub_1");
  for (const d of [db, db2, db3]) expect(await grants(d)).toBe(0);
});

it("past_due_keeps_the_allowance", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  const pastDue = plusSub(id, { status: "past_due" });
  expect((await deliver(db, stripe({ sub_1: pastDue }).fetcher, event("invoice.payment_failed", { id: "in_2", object: "invoice", parent: { subscription_details: { subscription: "sub_1" } } }))).status).toBe(200);
  expect((await deliver(db, stripe({ sub_1: pastDue }).fetcher, subEvent("customer.subscription.updated"))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  expect(await markers(db)).toEqual([]);
});

it("a_duplicate_where_the_stored_subscription_is_newer_keeps_the_older_and_grants_from_it", async () => {
  const { db, id } = await account();
  // The account is on sub_2 (newer), whose first invoice has not been paid yet.
  await accounts.applyPlan(db, id, "sub_2", { plan: "cloud", tier: "plus", untilUnixMs: (LINE_END + DAY) * 1000, interval: "month", renews: true });
  const older = plusSub(id, { created: START - 1000 });
  const newer = plusSub(id, { id: "sub_2", created: START, items: { data: [{ id: "si_2", current_period_end: LINE_END + DAY, price: { id: "price_plus_monthly", recurring: { interval: "month" } } }] } });
  const { fetcher, calls } = stripe({ sub_1: older, sub_2: newer });
  vi.spyOn(console, "error").mockImplementation(() => {});
  // The older sub_1's invoice is paid: sub_2 is cancelled, sub_1 is kept and grants.
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect(calls).toContain("DELETE https://api.stripe.com/v1/subscriptions/sub_2");
  expect(calls).not.toContain("DELETE https://api.stripe.com/v1/subscriptions/sub_1");
  expect((await accounts.findAccountById(db, id))?.stripeSubscriptionId).toBe("sub_1");
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  expect(await grants(db)).toBe(1);
});

it("a_duplicate_where_the_newer_stored_one_already_granted_never_doubles_the_allowance", async () => {
  const { db, id } = await account();
  const older = plusSub(id, { created: START - 1000 });
  const newer = plusSub(id, { id: "sub_2", created: START, items: { data: [{ id: "si_2", current_period_end: LINE_END + DAY, price: { id: "price_plus_monthly", recurring: { interval: "month" } } }] } });
  // sub_2 lands first and grants (its line ends a day later).
  expect((await deliver(db, stripe({ sub_2: newer }).fetcher, paid(invoice({ id: "in_2" }, { sub: "sub_2", item: "si_2", end: LINE_END + DAY })))).status).toBe(200);
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  vi.spyOn(console, "error").mockImplementation(() => {});
  expect((await deliver(db, stripe({ sub_1: older, sub_2: newer }).fetcher, paid(invoice()))).status).toBe(200);
  expect((await accounts.findAccountById(db, id))?.stripeSubscriptionId).toBe("sub_1");
  // The kept sub_1's grant is recorded but cannot move the expiry backwards: no top-up.
  expect(await allowance(db, id)).toEqual({ micros: 1_000_000, expires: (LINE_END + DAY) * 1000, plan: "cloud", tier: "plus" });
});

it("every_zero_path_leaves_one_marker_and_the_non_zero_paths_leave_none", async () => {
  // Immediate cancel.
  {
    const { db, id } = await account();
    expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
    const ended = plusSub(id, { status: "canceled", ended_at: NOW / 1000 });
    expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"))).status).toBe(200);
    expect(await markers(db)).toEqual([{ grant_key: `zeroed:${id}:${NOW}`, account_id: id, micros: 0, period_end_unix_ms: NOW + 1, created_at_unix_ms: NOW, subscription_id: "sub_1" }]);
  }
  // Full refund; a partial refund before it and a cancel at period end leave none.
  {
    const { db, id } = await account();
    const { fetcher } = stripe({ sub_1: plusSub(id), in_1: invoice() }, { pi_inv: "in_1" });
    expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
    expect((await deliver(db, fetcher, event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 500, payment_intent: "pi_inv" }))).status).toBe(200);
    expect((await deliver(db, stripe({ sub_1: plusSub(id, { cancel_at_period_end: true }) }).fetcher, subEvent("customer.subscription.updated"))).status).toBe(200);
    expect(await markers(db)).toEqual([]);
    expect((await deliver(db, fetcher, event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_inv" }))).status).toBe(200);
    expect(await markers(db)).toEqual([{ grant_key: `zeroed:${id}:${NOW}`, account_id: id, micros: 0, period_end_unix_ms: LINE_END * 1000 + 1, created_at_unix_ms: NOW, subscription_id: "sub_1" }]);
    // The same refund redelivered under a new event id in the same ms: no unique-key error, still one marker.
    expect((await deliver(db, fetcher, event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_inv" }))).status).toBe(200);
    expect((await markers(db)).length).toBe(1);
  }
});

// ---- the daily cron: months 2 to 12 of a yearly Plus subscription ----

const S_MS = START * 1000; // the yearly fixture's year starts here (2026-11-19 08:59 UTC)
const Y_END_MS = YEAR_END * 1000; // and is paid through here (2027-11-19 08:59 UTC)
const HOUR = 3_600_000;
/** When month n of the yearly fixture starts (month 1 starts at S_MS). */
const monthStart = (n: number) => addMonthsUtc(S_MS, n - 1);
const allGrants = async (db: SqlExecutor) =>
  db.execute("SELECT grant_key, account_id, micros, period_end_unix_ms, created_at_unix_ms FROM ai_allowance_grant ORDER BY rowid", []);

/** A yearly Plus account whose first invoice (in_1) has granted month 1. */
async function yearlyAccount() {
  const { db, id } = await account();
  const s = stripe({ sub_1: yearlySub(id), in_1: yearlyInvoice() }, { pi_inv: "in_1" });
  expect((await deliver(db, s.fetcher, paid(yearlyInvoice()))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: MONTH_1_END, plan: "cloud", tier: "plus" });
  return { db, id, fetcher: s.fetcher };
}
const fullRefund = () => event("charge.refunded", { id: "ch_inv", object: "charge", amount: 1500, amount_refunded: 1500, payment_intent: "pi_inv" });

/** The cron finds nothing to do: returns 0, writes no row, moves no bucket. */
async function expectNothing(db: SqlExecutor, id: string, at: number) {
  const before = { rows: await allGrants(db), a: await allowance(db, id) };
  expect(await grantDueYearlyMonths(db, at)).toBe(0);
  expect(await allGrants(db)).toEqual(before.rows);
  expect(await allowance(db, id)).toEqual(before.a);
}

it("month_two_is_granted_once_by_the_cron", async () => {
  const { db, id } = await yearlyAccount();
  // The last ms of month 1: nothing.
  await expectNothing(db, id, monthStart(2) - 1);
  // The first ms of month 2: one grant, keyed on the first invoice, ending with month 2.
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  expect(await grantDueYearlyMonths(db, monthStart(2))).toBe(1);
  expect(monthStart(2)).toBe(MONTH_1_END);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: MONTH_2_END, plan: "cloud", tier: "plus" });
  expect((await allGrants(db)).map((g) => [g.grant_key, g.account_id, g.micros, g.period_end_unix_ms])).toEqual([
    ["in_1", id, SETTING, MONTH_1_END],
    ["in_1:m2", id, SETTING, MONTH_2_END],
  ]);
  await expectNothing(db, id, monthStart(2) + DAY * 1000);
});

it("months_two_to_twelve_only", async () => {
  const { db, id } = await yearlyAccount();
  // Month 1 is the invoice's: the cron never grants it.
  for (const at of [NOW, NOW + 10 * DAY * 1000, monthStart(2) - 1]) await expectNothing(db, id, at);
  for (let n = 2; n <= 12; n++) {
    expect(await grantDueYearlyMonths(db, monthStart(n) + HOUR)).toBe(1);
    const end = addMonthsUtc(S_MS, n);
    expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: end });
    expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: `in_1:m${n}`, period_end_unix_ms: end });
  }
  // Month 12 ends exactly when the year is paid through; after that, nothing.
  expect((await allowance(db, id)).expires).toBe(Y_END_MS);
  for (const at of [Y_END_MS - 1, Y_END_MS, Y_END_MS + DAY * 1000]) await expectNothing(db, id, at);
  expect((await allGrants(db)).map((g) => g.grant_key)).toEqual(["in_1", ...Array.from({ length: 11 }, (_, i) => `in_1:m${i + 2}`)]);
});

it("a_second_cron_run_in_the_same_month_grants_nothing", async () => {
  const { db, id } = await yearlyAccount();
  expect(await grantDueYearlyMonths(db, monthStart(3) + HOUR)).toBe(1);
  // Spent down, the allowance is not refilled by a later run in the same month.
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  for (const at of [monthStart(3) + HOUR, monthStart(3) + 2 * HOUR, monthStart(3) + 20 * DAY * 1000, monthStart(4) - 1]) await expectNothing(db, id, at);
  expect((await allowance(db, id)).micros).toBe(1_000_000);
});

it("a_cancelled_or_refunded_yearly_account_gets_no_more_grants", async () => {
  // A full refund in month 2: the plan stays yearly Plus, only the marker stops the cron.
  {
    const { db, id, fetcher } = await yearlyAccount();
    expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(1);
    expect((await deliver(db, fetcher, fullRefund(), monthStart(2) + 2 * HOUR)).status).toBe(200);
    expect(await allowance(db, id)).toMatchObject({ micros: 0, plan: "cloud", tier: "plus" });
    for (let n = 3; n <= 12; n++) await expectNothing(db, id, monthStart(n) + HOUR);
  }
  // A full refund still in month 1 stops month 2 onwards.
  {
    const { db, id, fetcher } = await yearlyAccount();
    expect((await deliver(db, fetcher, fullRefund(), NOW + HOUR)).status).toBe(200);
    for (const n of [2, 3, 12]) await expectNothing(db, id, monthStart(n) + HOUR);
  }
  // An immediate cancel: the plan is free.
  {
    const { db, id } = await yearlyAccount();
    expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(1);
    const ended = yearlySub(id, { status: "canceled", ended_at: (monthStart(2) + 2 * HOUR) / 1000 });
    expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"), monthStart(2) + 2 * HOUR)).status).toBe(200);
    expect(await allowance(db, id)).toMatchObject({ micros: 0, plan: "free", tier: null });
    for (const n of [3, 4, 12]) await expectNothing(db, id, monthStart(n) + HOUR);
  }
});

it("a_monthly_subscriber_is_never_granted_by_the_cron", async () => {
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: LINE_END * 1000, plan: "cloud", tier: "plus" });
  await db.execute("UPDATE account SET ai_allowance_micros = 1000000 WHERE id = ?", [id]);
  for (const at of [NOW, NOW + 10 * DAY * 1000, LINE_END * 1000 - 1]) await expectNothing(db, id, at);
  // The renewal's subscription update lands before its invoice is paid:
  // plan_until is already the next period's end, the latest grant still
  // the last one. No allowance before the renewal is paid.
  const next = LINE_END + 31 * DAY;
  expect((await deliver(db, stripe({ sub_1: plusSub(id, {}, "price_plus_monthly", next) }).fetcher, subEvent("customer.subscription.updated"), LINE_END * 1000 + 60_000)).status).toBe(200);
  expect((await accounts.findAccountById(db, id))?.planUntilUnixMs).toBe(next * 1000);
  for (const at of [LINE_END * 1000 + 120_000, LINE_END * 1000 + HOUR]) await expectNothing(db, id, at);
});

it("the_cron_never_grants_month_one_even_when_the_base_grant_ends_early", async () => {
  // A monthly-to-yearly switch: the yearly year starts at S_MS, the switch
  // invoice granted nothing, and the last monthly grant ends 10 days in.
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_monthly", micros: SETTING, periodEndUnixMs: S_MS + 10 * DAY * 1000 }, S_MS - DAY * 1000)).toBe(true);
  for (const at of [S_MS + HOUR, S_MS + 12 * DAY * 1000, monthStart(2) - 1]) await expectNothing(db, id, at);
  // Month 2 onwards hangs off the monthly invoice.
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(1);
  expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_monthly:m2", period_end_unix_ms: MONTH_2_END });
  // The same with the monthly grant's own (monthly) line stored: a monthly
  // line never defines the year, so the year still comes from plan_until.
  const { db: db2, id: id2 } = await account();
  await accounts.applyPlan(db2, id2, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
  const monthly = { lineStartUnixMs: S_MS - 20 * DAY * 1000, lineEndUnixMs: S_MS + 10 * DAY * 1000 };
  expect(await grantAllowance(db2, { accountId: id2, grantKey: "in_monthly", micros: SETTING, periodEndUnixMs: monthly.lineEndUnixMs, ...monthly }, S_MS - DAY * 1000)).toBe(true);
  for (const at of [S_MS + HOUR, S_MS + 12 * DAY * 1000, monthStart(2) - 1]) await expectNothing(db2, id2, at);
  expect(await grantDueYearlyMonths(db2, monthStart(2) + HOUR)).toBe(1);
  expect((await allGrants(db2)).at(-1)).toMatchObject({ grant_key: "in_monthly:m2", period_end_unix_ms: MONTH_2_END });
});

it("the_month_end_never_passes_the_paid_through_date", async () => {
  // The usual year: month 12 ends exactly at plan_until, every month before it earlier.
  {
    const { db, id } = await yearlyAccount();
    for (let n = 2; n <= 12; n++) {
      expect(await grantDueYearlyMonths(db, monthStart(n))).toBe(1);
      const expires = (await allowance(db, id)).expires!;
      expect(expires).toBeLessThanOrEqual(Y_END_MS);
      if (n < 12) expect(expires).toBeLessThan(Y_END_MS);
    }
    expect((await allowance(db, id)).expires).toBe(Y_END_MS);
  }
  // A year paid through 29 Feb 2028 (anchor day 29): it began 28 Feb 2027.
  // Month 12 runs from 28 Jan 2028 to the year end itself (not 28 Feb), so
  // the last day is covered, and there is never a 13th month.
  {
    const { db, id } = await account();
    const start = Date.UTC(2027, 1, 28, 9) / 1000;
    const end = Date.UTC(2028, 1, 29, 9) / 1000;
    const sub = yearlySub(id, {}, end);
    const inv = invoice({ billing_reason: "subscription_cycle" }, { price: "price_plus_yearly", start, end });
    expect((await deliver(db, stripe({ sub_1: sub }).fetcher, paid(inv), (start + 60) * 1000)).status).toBe(200);
    expect(await grantDueYearlyMonths(db, Date.UTC(2028, 0, 28, 10))).toBe(1);
    expect((await allowance(db, id)).expires).toBe(end * 1000);
    expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_1:m12", period_end_unix_ms: end * 1000 });
    for (const at of [Date.UTC(2028, 1, 28, 9), Date.UTC(2028, 1, 28, 10), end * 1000 - 1]) await expectNothing(db, id, at);
    expect((await allGrants(db)).some((g) => String(g.grant_key).endsWith(":m13"))).toBe(false);
  }
});

it("the_cron_does_not_backfill_missed_months", async () => {
  const { db, id } = await yearlyAccount();
  // No run in months 2 to 4: month 5's run grants month 5 only.
  expect(await grantDueYearlyMonths(db, monthStart(5) + HOUR)).toBe(1);
  expect((await allGrants(db)).map((g) => g.grant_key)).toEqual(["in_1", "in_1:m5"]);
  expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: addMonthsUtc(S_MS, 5) });
  await expectNothing(db, id, monthStart(5) + 2 * HOUR);
});

it("a_marker_from_a_previous_year_does_not_block_the_new_year", async () => {
  for (const zeroMonth of [2, 12]) {
    const { db, id, fetcher } = await yearlyAccount();
    expect(await grantDueYearlyMonths(db, monthStart(zeroMonth) + HOUR)).toBe(1);
    // Refunded in year 1 (the plan stays on), then renewed for year 2.
    expect((await deliver(db, fetcher, fullRefund(), monthStart(zeroMonth) + 2 * HOUR)).status).toBe(200);
    const [marker] = await markers(db);
    expect((await markers(db)).length).toBe(1);
    const y2End = Date.UTC(2028, 10, 19, 8, 59) / 1000;
    if (zeroMonth === 12) {
      // A month-12 zero's marker ends after year 1's G1 AND inside the
      // cron's YEAR_BACK_MS read window for year 2, so the cron really sees it:
      // only the comparison with year 2's G1 keeps it from blocking.
      expect(Number(marker.period_end_unix_ms)).toBe(Y_END_MS + 1);
      expect(Number(marker.period_end_unix_ms)).toBeGreaterThan(y2End * 1000 - YEAR_BACK_MS);
      // The window must stay a full leap year: a shorter one could drop such a marker, silently.
      expect(YEAR_BACK_MS).toBeGreaterThanOrEqual(366 * DAY * 1000);
    }
    const renewal = invoice({ id: "in_2", billing_reason: "subscription_cycle", period_start: START, period_end: YEAR_END }, { price: "price_plus_yearly", start: YEAR_END, end: y2End });
    const y2 = stripe({ sub_1: yearlySub(id, {}, y2End), in_2: renewal }, { pi_inv2: "in_2" });
    expect((await deliver(db, y2.fetcher, paid(renewal), Y_END_MS + 60_000)).status).toBe(200);
    expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: addMonthsUtc(Y_END_MS, 1), plan: "cloud", tier: "plus" });
    // Year 2's month 2 is granted, keyed on year 2's invoice.
    expect(await grantDueYearlyMonths(db, addMonthsUtc(Y_END_MS, 1) + HOUR)).toBe(1);
    expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_2:m2", period_end_unix_ms: addMonthsUtc(Y_END_MS, 2) });
    expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: addMonthsUtc(Y_END_MS, 2) });
    // A year-2 refund (its marker at or after year 2's G1) does block.
    const refund2 = event("charge.refunded", { id: "ch_inv2", object: "charge", amount: 14400, amount_refunded: 14400, payment_intent: "pi_inv2" });
    expect((await deliver(db, y2.fetcher, refund2, addMonthsUtc(Y_END_MS, 1) + 2 * HOUR)).status).toBe(200);
    expect((await markers(db)).length).toBe(2);
    for (const n of [3, 4]) await expectNothing(db, id, addMonthsUtc(Y_END_MS, n - 1) + HOUR);
  }
});

it("a_refused_late_grant_row_after_a_marker_does_not_unblock_the_cron", async () => {
  const { db, id, fetcher } = await yearlyAccount();
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(1);
  expect((await deliver(db, fetcher, fullRefund(), monthStart(2) + 2 * HOUR)).status).toBe(200);
  // A late grant for a period inside the refunded window is refused but keeps its row, written after the marker.
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_late", micros: SETTING, periodEndUnixMs: MONTH_2_END }, monthStart(2) + 3 * HOUR)).toBe(true);
  expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_late" });
  expect((await allowance(db, id)).micros).toBe(0);
  for (const n of [3, 4]) await expectNothing(db, id, monthStart(n) + HOUR);
});

it("a_repeat_zero_in_the_same_millisecond_still_blocks_the_cron", async () => {
  const { db, id } = await yearlyAccount();
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(1);
  const at = monthStart(2) + 2 * HOUR;
  await zeroAllowance(db, id, at, { subscriptionId: "sub_1", mode: "refund" });
  await zeroAllowance(db, id, at, { subscriptionId: "sub_1", mode: "refund" });
  // One marker; the expiry has moved 1 ms past it.
  const [m] = await markers(db);
  expect((await markers(db)).length).toBe(1);
  expect((await allowance(db, id)).expires).toBe(Number(m.period_end_unix_ms) + 1);
  for (const n of [3, 4]) await expectNothing(db, id, monthStart(n) + HOUR);
});

it("an_account_with_no_first_invoice_grant_gets_nothing", async () => {
  // Yearly Plus on the account, but the first invoice never granted.
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
  for (const n of [2, 3, 12]) await expectNothing(db, id, monthStart(n) + HOUR);
  // Only month keys or last year's invoice grant: still no base grant for this year.
  await grantAllowance(db, { accountId: id, grantKey: "in_old:m12", micros: SETTING, periodEndUnixMs: S_MS + HOUR }, S_MS - HOUR);
  await grantAllowance(db, { accountId: id, grantKey: "in_old", micros: SETTING, periodEndUnixMs: S_MS }, S_MS - 2 * HOUR);
  for (const n of [2, 3, 12]) await expectNothing(db, id, monthStart(n) + HOUR);
});

it("a_deleting_account_gets_nothing", async () => {
  const { db, id } = await yearlyAccount();
  await db.execute("UPDATE account SET deleting_since_unix_ms = ? WHERE id = ?", [NOW + HOUR, id]);
  for (const n of [2, 3]) await expectNothing(db, id, monthStart(n) + HOUR);
});

it("a_cancel_at_period_end_yearly_account_keeps_monthly_grants_until_plan_until", async () => {
  const { db, id } = await yearlyAccount();
  const cancelling = yearlySub(id, { cancel_at_period_end: true });
  expect((await deliver(db, stripe({ sub_1: cancelling }).fetcher, subEvent("customer.subscription.updated"), NOW + HOUR)).status).toBe(200);
  expect((await accounts.findAccountById(db, id))?.planRenews).toBe(false);
  expect((await accounts.findAccountById(db, id))?.planUntilUnixMs).toBe(Y_END_MS);
  for (let n = 2; n <= 12; n++) expect(await grantDueYearlyMonths(db, monthStart(n) + HOUR)).toBe(1);
  expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: Y_END_MS });
  await expectNothing(db, id, Y_END_MS);
});

it("the_cron_grants_at_most_the_per_run_limit_and_the_rest_next_run", async () => {
  const db = realSqlExecutor();
  open.push(db);
  const ids: string[] = [];
  for (let i = 0; i <= DEFAULT_YEARLY_GRANTS_PER_RUN; i++) {
    const a = await accounts.getOrCreateAccount(db, `u${i}@example.com`, `g-${i}`, 1, "db", "https://db.test");
    await accounts.applyPlan(db, a.id, `sub_${i}`, { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
    expect(await grantAllowance(db, { accountId: a.id, grantKey: `in_${i}`, micros: SETTING, periodEndUnixMs: MONTH_1_END }, NOW)).toBe(true);
    ids.push(a.id);
  }
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(DEFAULT_YEARLY_GRANTS_PER_RUN);
  expect(warn).toHaveBeenCalledWith("plus yearly grants: per-run limit reached,", 1, "due accounts left for the next run");
  // Granted in account id order; the last id waits for the next run, which grants it alone.
  const last = [...ids].sort().at(-1)!;
  const granted = async () => (await db.execute("SELECT account_id FROM ai_allowance_grant WHERE grant_key LIKE '%:m2' ORDER BY account_id", [])).map((r) => String(r.account_id));
  expect(await granted()).toEqual([...ids].sort().slice(0, DEFAULT_YEARLY_GRANTS_PER_RUN));
  expect(await grantDueYearlyMonths(db, monthStart(2) + 2 * HOUR)).toBe(1);
  expect((await granted()).at(-1)).toBe(last);
  expect(await grantDueYearlyMonths(db, monthStart(2) + 3 * HOUR)).toBe(0);
  expect(warn).toHaveBeenCalledTimes(1);
});

it("the_base_grant_is_the_latest_period_end_not_the_latest_written", async () => {
  const { db, id } = await yearlyAccount();
  // In month 1, a late invoice grant for a period ending before month 1 does
  // is written after in_1 (refused by the forward guard, row kept).
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_late", micros: SETTING, periodEndUnixMs: MONTH_1_END - HOUR }, NOW + HOUR)).toBe(true);
  expect((await allowance(db, id)).expires).toBe(MONTH_1_END);
  // The month keys still hang off in_1, whose period ends latest.
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(1);
  expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_1:m2", period_end_unix_ms: MONTH_2_END });
  await expectNothing(db, id, monthStart(2) + 2 * HOUR);
});

it("lapsed_or_deleting_accounts_never_use_up_the_per_run_limit", async () => {
  const db = realSqlExecutor();
  open.push(db);
  const ids: string[] = [];
  for (let i = 0; i <= 2 * DEFAULT_YEARLY_GRANTS_PER_RUN; i++) {
    ids.push((await accounts.getOrCreateAccount(db, `u${i}@example.com`, `g-${i}`, 1, "db", "https://db.test")).id);
  }
  ids.sort();
  const run = monthStart(2) + HOUR;
  const due = ids.at(-1)!;
  for (const [i, id] of ids.entries()) {
    if (i < DEFAULT_YEARLY_GRANTS_PER_RUN) {
      // Lapsed a ms before the run: still yearly Plus on the row, with this (now past) year's base grant only.
      const until = run - 1;
      const g1End = addMonthsUtc(addMonthsUtc(until, -12), 1);
      await accounts.applyPlan(db, id, `sub_${i}`, { plan: "cloud", tier: "plus", untilUnixMs: until, interval: "year", renews: true });
      expect(await grantAllowance(db, { accountId: id, grantKey: `in_${i}`, micros: SETTING, periodEndUnixMs: g1End }, g1End - HOUR)).toBe(true);
    } else {
      // Due this run; all but the last are being deleted.
      await accounts.applyPlan(db, id, `sub_${i}`, { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
      expect(await grantAllowance(db, { accountId: id, grantKey: `in_${i}`, micros: SETTING, periodEndUnixMs: MONTH_1_END }, NOW)).toBe(true);
      if (id !== due) await db.execute("UPDATE account SET deleting_since_unix_ms = ? WHERE id = ?", [NOW + HOUR, id]);
    }
  }
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  expect(await grantDueYearlyMonths(db, run)).toBe(1);
  expect((await db.execute("SELECT account_id FROM ai_allowance_grant WHERE grant_key LIKE '%:m%'", [])).map((r) => String(r.account_id))).toEqual([due]);
  expect(warn).not.toHaveBeenCalled();
});

it("a_yearly_invoice_paid_late_grants_the_current_month_under_the_invoice_key", async () => {
  const { db, id } = await account();
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  // Paid in month 3 of its line (say, after long dunning).
  expect((await deliver(db, stripe({ sub_1: yearlySub(id) }).fetcher, paid(yearlyInvoice()), monthStart(3) + HOUR)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: addMonthsUtc(S_MS, 3), plan: "cloud", tier: "plus" });
  expect((await allGrants(db)).map((g) => [g.grant_key, g.period_end_unix_ms])).toEqual([["in_1", addMonthsUtc(S_MS, 3)]]);
  expect(log).toHaveBeenCalledWith("billing: yearly plus invoice paid late, granting month", 3, "in_1");
  expect(err).not.toHaveBeenCalled();
  // The cron does not grant month 3 again, then carries on from month 4, keyed on the invoice.
  await expectNothing(db, id, monthStart(3) + 2 * HOUR);
  expect(await grantDueYearlyMonths(db, monthStart(4) + HOUR)).toBe(1);
  expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_1:m4", period_end_unix_ms: addMonthsUtc(S_MS, 4) });
  // Paid in month 12: that month ends at the line's end.
  const { db: db2, id: id2 } = await account();
  expect((await deliver(db2, stripe({ sub_1: yearlySub(id2) }).fetcher, paid(yearlyInvoice()), monthStart(12) + HOUR)).status).toBe(200);
  expect(await allowance(db2, id2)).toMatchObject({ micros: SETTING, expires: Y_END_MS });
  // Paid on time it is still month 1.
  const { db: db3, id: id3 } = await account();
  expect((await deliver(db3, stripe({ sub_1: yearlySub(id3) }).fetcher, paid(yearlyInvoice()), MONTH_1_END - 1000)).status).toBe(200);
  expect(await allowance(db3, id3)).toMatchObject({ micros: SETTING, expires: MONTH_1_END });
});

it("a_yearly_invoice_paid_after_the_year_is_over_grants_nothing_and_logs", async () => {
  const { db, id } = await account();
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const evt = paid(yearlyInvoice());
  expect((await deliver(db, stripe({ sub_1: yearlySub(id) }).fetcher, evt, Y_END_MS + HOUR)).status).toBe(200);
  expect(await recorded(db, evt.id)).toBe(true);
  expect(await allGrants(db)).toEqual([]);
  expect((await allowance(db, id)).micros).toBe(0);
  expect(err).toHaveBeenCalledWith("billing: paid plus invoice for a period already ended, no grant", "in_1");
});

/** Yearly Plus accounts with a month-1 base grant, sorted by id. */
async function dueAccounts(db: SqlExecutor, count: number) {
  const ids: string[] = [];
  for (let i = 0; i < count; i++) {
    const a = await accounts.getOrCreateAccount(db, `u${i}@example.com`, `g-${i}`, 1, "db", "https://db.test");
    await accounts.applyPlan(db, a.id, `sub_${i}`, { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
    expect(await grantAllowance(db, { accountId: a.id, grantKey: `in_${i}`, micros: SETTING, periodEndUnixMs: MONTH_1_END }, NOW)).toBe(true);
    ids.push(a.id);
  }
  return ids.sort();
}

it("one_failing_account_does_not_stop_the_cron", async () => {
  const real = realSqlExecutor();
  open.push(real);
  const ids = await dueAccounts(real, 3);
  const bad = ids[1];
  const db: SqlExecutor = {
    execute: (sql, params) => real.execute(sql, params),
    batch: async (stmts) => {
      if (stmts.some((st) => st.params.includes(bad))) throw new Error("boom");
      return real.batch(stmts);
    },
  };
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(2);
  expect(err).toHaveBeenCalledWith("plus yearly grant failed, account", bad, "Error: boom");
  const granted = (await real.execute("SELECT account_id FROM ai_allowance_grant WHERE grant_key LIKE '%:m2' ORDER BY account_id", [])).map((r) => String(r.account_id));
  expect(granted).toEqual([ids[0], ids[2]]);
});

it("the_cron_reads_the_allowance_setting_once_per_run", async () => {
  const real = realSqlExecutor();
  open.push(real);
  await dueAccounts(real, 3);
  await real.execute("INSERT INTO server_setting (key, value) VALUES ('plus_allowance_micros', '7500000')", []);
  let reads = 0;
  const db: SqlExecutor = {
    execute: (sql, params) => {
      if (sql.includes("'plus_allowance_micros'")) reads++;
      return real.execute(sql, params);
    },
    batch: (stmts) => real.batch(stmts),
  };
  // Nothing due (month 1): no read at all.
  expect(await grantDueYearlyMonths(db, NOW + HOUR)).toBe(0);
  expect(reads).toBe(0);
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(3);
  expect(reads).toBe(1);
  const m2 = await real.execute("SELECT micros FROM ai_allowance_grant WHERE grant_key LIKE '%:m2'", []);
  expect(m2.map((r) => Number(r.micros))).toEqual([7_500_000, 7_500_000, 7_500_000]);
});

it("the_cron_counts_only_grants_that_refill_the_allowance", async () => {
  const { db: real, id } = await yearlyAccount();
  // The allowance moves past month 2's end between the cron's SELECT and its
  // grant (say, another grant landing): the row is written, nothing refills.
  const db: SqlExecutor = {
    execute: async (sql, params) => {
      const rows = await real.execute(sql, params);
      if (sql.startsWith("SELECT a.id AS account_id")) await real.execute("UPDATE account SET ai_allowance_micros = 1000000, ai_allowance_expires_ms = ? WHERE id = ?", [Y_END_MS, id]);
      return rows;
    },
    batch: (stmts) => real.batch(stmts),
  };
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(0);
  expect((await allGrants(real)).at(-1)).toMatchObject({ grant_key: "in_1:m2", period_end_unix_ms: MONTH_2_END });
  expect(await allowance(real, id)).toMatchObject({ micros: 1_000_000, expires: Y_END_MS });
});

it("an_account_with_a_live_allowance_window_is_not_even_read", async () => {
  const { db: real, id } = await yearlyAccount();
  // Month 2 is due, but the allowance window has not ended (it runs past month 2's start).
  await real.execute("UPDATE account SET ai_allowance_expires_ms = ? WHERE id = ?", [monthStart(2) + DAY * 1000, id]);
  let read: string[] = [];
  const db: SqlExecutor = {
    execute: async (sql, params) => {
      const rows = await real.execute(sql, params);
      if (sql.startsWith("SELECT a.id AS account_id")) read = rows.map((r) => String(r.account_id));
      return rows;
    },
    batch: (stmts) => real.batch(stmts),
  };
  await expectNothing(db, id, monthStart(2) + HOUR);
  expect(read).toEqual([]);
  // At the expiry exactly, it is read and granted.
  expect(await grantDueYearlyMonths(db, monthStart(2) + DAY * 1000)).toBe(1);
  expect(read.length).toBeGreaterThan(0);
  expect(new Set(read)).toEqual(new Set([id]));
});

it("a_base_grant_outlasting_month_two_start_delays_the_month_two_grant", async () => {
  // A monthly-to-yearly switch: the last monthly window (anchored on the
  // 31st, say) ends 3 days after the yearly month 2 starts.
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
  const monthlyEnd = monthStart(2) + 3 * DAY * 1000;
  const line = { lineStartUnixMs: monthlyEnd - 31 * DAY * 1000, lineEndUnixMs: monthlyEnd };
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_monthly", micros: SETTING, periodEndUnixMs: monthlyEnd, ...line }, S_MS)).toBe(true);
  // The monthly balance is kept until its window ends; month 2 comes then, up to 3 days late, ending as usual.
  for (const at of [monthStart(2) + HOUR, monthlyEnd - 1]) await expectNothing(db, id, at);
  expect(await grantDueYearlyMonths(db, monthlyEnd)).toBe(1);
  expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_monthly:m2", period_end_unix_ms: MONTH_2_END });
});

it("a_paid_plus_invoice_whose_line_period_is_not_a_range_is_logged_not_retried", async () => {
  for (const [start, end] of [[START, START], [LINE_END, START]]) {
    const { db, id } = await account();
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const evt = paid(invoice({}, { start, end }));
    expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, evt)).status).toBe(200);
    expect(await recorded(db, evt.id)).toBe(true);
    expect(await allGrants(db)).toEqual([]);
    expect(err).toHaveBeenCalledWith("billing: paid plus invoice line period is not a range, no grant", "in_1");
    err.mockRestore();
  }
  // A valid Plus line after such a line still grants.
  const { db, id } = await account();
  const inv = invoice();
  inv.lines.data.unshift({ ...inv.lines.data[0], id: "il_bad", period: { start: START, end: START } } as never);
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(inv))).status).toBe(200);
  expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: LINE_END * 1000 });
});

it("the_per_run_cap_is_a_server_setting_defaulting_to_10", async () => {
  const real = realSqlExecutor();
  open.push(real);
  const ids = await dueAccounts(real, 14);
  let capReads = 0;
  const db: SqlExecutor = {
    execute: (sql, params) => {
      if (sql.includes("'plus_yearly_grants_per_run'")) capReads++;
      return real.execute(sql, params);
    },
    batch: (stmts) => real.batch(stmts),
  };
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const granted = async () => (await real.execute("SELECT account_id FROM ai_allowance_grant WHERE grant_key LIKE '%:m2' ORDER BY account_id", [])).length;
  // Nothing due: the cap is not even read.
  expect(await grantDueYearlyMonths(db, NOW + HOUR)).toBe(0);
  expect(capReads).toBe(0);
  // Missing: 10, read once per run.
  expect(DEFAULT_YEARLY_GRANTS_PER_RUN).toBe(10);
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(10);
  expect(capReads).toBe(1);
  expect(warn).toHaveBeenCalledWith("plus yearly grants: per-run limit reached,", 4, "due accounts left for the next run");
  // Set to 3: three a run.
  await real.execute("INSERT INTO server_setting (key, value) VALUES ('plus_yearly_grants_per_run', '3')", []);
  expect(await grantDueYearlyMonths(db, monthStart(2) + 2 * HOUR)).toBe(3);
  expect(await granted()).toBe(13);
  // Unusable values fall back to 10 with one warning each.
  for (const bad of ["0", "201", "-1", "2.5", "lots"]) {
    expect(await readYearlyGrantsPerRun(real)).toBe(3);
    await real.execute("UPDATE server_setting SET value = ? WHERE key = 'plus_yearly_grants_per_run'", [bad]);
    warn.mockClear();
    expect(await readYearlyGrantsPerRun(real)).toBe(10);
    expect(warn).toHaveBeenCalledWith(`server_setting plus_yearly_grants_per_run ${JSON.stringify(bad)} is not a whole number in 1..200; using 10`);
    await real.execute("UPDATE server_setting SET value = '3' WHERE key = 'plus_yearly_grants_per_run'", []);
  }
  await real.execute("UPDATE server_setting SET value = '200' WHERE key = 'plus_yearly_grants_per_run'", []);
  expect(await readYearlyGrantsPerRun(real)).toBe(200);
  expect(ids.length).toBe(14);
});

// ---- stored invoice line bounds (migration 0007) ----

const lineOf = async (db: SqlExecutor, key: string) =>
  (await db.execute("SELECT line_start_unix_ms AS s, line_end_unix_ms AS e FROM ai_allowance_grant WHERE grant_key = ?", [key]))[0];

it("grants_store_the_line_start_and_end", async () => {
  // Monthly: the invoice line's period.
  const { db, id } = await account();
  expect((await deliver(db, stripe({ sub_1: plusSub(id) }).fetcher, paid(invoice()))).status).toBe(200);
  expect(await lineOf(db, "in_1")).toEqual({ s: START * 1000, e: LINE_END * 1000 });
  // Yearly: the year line, on the invoice grant and on every month grant.
  const { db: db2 } = await yearlyAccount();
  expect(await lineOf(db2, "in_1")).toEqual({ s: S_MS, e: Y_END_MS });
  expect(await grantDueYearlyMonths(db2, monthStart(2) + HOUR)).toBe(1);
  expect(await lineOf(db2, "in_1:m2")).toEqual({ s: S_MS, e: Y_END_MS });
  // A yearly invoice paid late (month 3) stores the line too.
  const { db: db3, id: id3 } = await account();
  vi.spyOn(console, "log").mockImplementation(() => {});
  expect((await deliver(db3, stripe({ sub_1: yearlySub(id3) }).fetcher, paid(yearlyInvoice()), monthStart(3) + HOUR)).status).toBe(200);
  expect(await lineOf(db3, "in_1")).toEqual({ s: S_MS, e: Y_END_MS });
  // Bounds are both or neither, start before end.
  for (const bad of [{ lineStartUnixMs: 1 }, { lineEndUnixMs: 2 }, { lineStartUnixMs: 2, lineEndUnixMs: 2 }, { lineStartUnixMs: 1.5, lineEndUnixMs: 3 }]) {
    await expect(grantAllowance(db3, { accountId: id3, grantKey: "in_x", micros: SETTING, periodEndUnixMs: Y_END_MS, ...bad }, NOW)).rejects.toThrow(RangeError);
  }
});

it("a_markers_line_bounds_are_null", async () => {
  const { db, fetcher } = await yearlyAccount();
  expect((await deliver(db, fetcher, fullRefund(), NOW + HOUR)).status).toBe(200);
  const [m] = await markers(db);
  expect(await lineOf(db, String(m.grant_key))).toEqual({ s: null, e: null });
});

it("old_grant_rows_without_line_bounds_still_work", async () => {
  // Rows written before 0007: no line bounds, so the year comes from plan_until.
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_old", micros: SETTING, periodEndUnixMs: MONTH_1_END }, NOW)).toBe(true);
  expect(await lineOf(db, "in_old")).toEqual({ s: null, e: null });
  await expectNothing(db, id, monthStart(2) - 1);
  expect(await grantDueYearlyMonths(db, monthStart(2))).toBe(1);
  expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: "in_old:m2", period_end_unix_ms: MONTH_2_END });
  // A derived year is not a stored line: the month grant keeps NULL bounds.
  expect(await lineOf(db, "in_old:m2")).toEqual({ s: null, e: null });
  expect(await grantDueYearlyMonths(db, monthStart(12) + HOUR)).toBe(1);
  expect(await allowance(db, id)).toMatchObject({ expires: Y_END_MS });
});

it("a_cancel_at_before_the_period_end_does_not_shift_the_month_windows", async () => {
  const { db, id } = await yearlyAccount();
  // cancel_at 200 days into the year (set by API or Dashboard): plan_until moves, the paid line does not.
  const cancelAt = START + 200 * DAY;
  expect((await deliver(db, stripe({ sub_1: yearlySub(id, { cancel_at: cancelAt }) }).fetcher, subEvent("customer.subscription.updated"), NOW + HOUR)).status).toBe(200);
  expect((await accounts.findAccountById(db, id))?.planUntilUnixMs).toBe(cancelAt * 1000);
  // Month 1 of the LINE: nothing (a year derived from plan_until would already be in its month 7).
  for (const at of [NOW + 2 * HOUR, S_MS + 20 * DAY * 1000, monthStart(2) - 1]) await expectNothing(db, id, at);
  // Months follow the line's windows.
  for (let n = 2; n <= 7; n++) {
    expect(await grantDueYearlyMonths(db, monthStart(n))).toBe(1);
    expect((await allGrants(db)).at(-1)).toMatchObject({ grant_key: `in_1:m${n}`, period_end_unix_ms: addMonthsUtc(S_MS, n) });
  }
  // Month 7 contains cancel_at; its grant ends with the month (the cancel's own webhook zeroes the allowance).
  expect(monthStart(7)).toBeLessThan(cancelAt * 1000);
  expect(monthStart(8)).toBeGreaterThan(cancelAt * 1000);
  // Once plan_until has passed, nothing more.
  for (const at of [cancelAt * 1000, monthStart(8) + HOUR]) await expectNothing(db, id, at);
});

// ---- zeros scoped to their subscription (review I1), refund scope (I2), refund tombstone (m1) ----

// The reviewer's I1 scenario. Monthly Plus A runs Jan 29 23:00 to Feb 28
// 23:00 2027 (Stripe clamps February); it is cancelled on Jan 30 at 10:00,
// and yearly Plus B is bought at 10:05, whose month 1 (clamped too) ends
// Feb 28 10:05, before A's window would have.
const A_START = Date.UTC(2027, 0, 29, 23) / 1000;
const A_END = Date.UTC(2027, 1, 28, 23) / 1000;
const CANCEL_MS = Date.UTC(2027, 0, 30, 10);
const B_START = Date.UTC(2027, 0, 30, 10, 5) / 1000;
const B_END = Date.UTC(2028, 0, 30, 10, 5) / 1000;
const B_MS = B_START * 1000;
const B_M1_END = Date.UTC(2027, 1, 28, 10, 5);
const B_PAID_MS = B_MS + 30_000;
/** When month n of B's year starts. */
const bMonth = (n: number) => addMonthsUtc(B_MS, n - 1);

const subA = (id: string, over: Record<string, unknown> = {}) =>
  plusSub(id, { id: "sub_A", created: A_START, items: { data: [{ id: "si_A", current_period_end: A_END, price: { id: "price_plus_monthly", recurring: { interval: "month" } } }] }, ...over });
const cancelledA = (id: string, at = CANCEL_MS) => subA(id, { status: "canceled", ended_at: at / 1000 });
const subB = (id: string) =>
  plusSub(id, { id: "sub_B", created: B_START, items: { data: [{ id: "si_B", current_period_end: B_END, price: { id: "price_plus_yearly", recurring: { interval: "year" } } }] } });
const invoiceA = (over: Record<string, unknown> = {}) => invoice({ id: "in_A", ...over }, { sub: "sub_A", item: "si_A", start: A_START, end: A_END });
const invoiceB = () => invoice({ id: "in_B" }, { sub: "sub_B", item: "si_B", price: "price_plus_yearly", start: B_START, end: B_END });
const refundOf = (pi: string, amount = 1500) => event("charge.refunded", { id: `ch_${pi}`, object: "charge", amount, amount_refunded: amount, payment_intent: pi });
const rowOf = async (db: SqlExecutor, key: string) =>
  (await db.execute("SELECT grant_key, micros, period_end_unix_ms, line_start_unix_ms, line_end_unix_ms, subscription_id FROM ai_allowance_grant WHERE grant_key = ?", [key]))[0];

/** An account on monthly Plus A, granted A's window at A's start. */
async function monthlyA() {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_A: subA(id), in_A: invoiceA() }, { pi_A: "in_A" });
  expect((await deliver(db, fetcher, paid(invoiceA()), A_START * 1000 + 60_000)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: A_END * 1000, plan: "cloud", tier: "plus" });
  expect(await rowOf(db, "in_A")).toMatchObject({ subscription_id: "sub_A" });
  return { db, id, fetcher };
}

/** B's month 1 has landed and the cron grants months 2, 3 and 12 for B, under B's key, with B's subscription id and line. */
async function expectBGetsItsYear(db: SqlExecutor, id: string) {
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: B_M1_END, plan: "cloud", tier: "plus" });
  expect(await rowOf(db, "in_B")).toEqual({ grant_key: "in_B", micros: SETTING, period_end_unix_ms: B_M1_END, line_start_unix_ms: B_MS, line_end_unix_ms: B_END * 1000, subscription_id: "sub_B" });
  for (const n of [2, 3, 12]) {
    expect(await grantDueYearlyMonths(db, bMonth(n) + HOUR), `month ${n}`).toBe(1);
    const end = n === 12 ? B_END * 1000 : addMonthsUtc(B_MS, n);
    expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: end });
    expect(await rowOf(db, `in_B:m${n}`)).toEqual({ grant_key: `in_B:m${n}`, micros: SETTING, period_end_unix_ms: end, line_start_unix_ms: B_MS, line_end_unix_ms: B_END * 1000, subscription_id: "sub_B" });
  }
}

it("a_new_subscription_bought_after_an_immediate_cancel_gets_its_month_one_and_its_year", async () => {
  const { db, id } = await monthlyA();
  // A is cancelled now: allowance 0, expiry the cancel time + 1 ms (not past A's Feb 28 23:00 end).
  expect((await deliver(db, stripe({ sub_A: cancelledA(id) }).fetcher, subEvent("customer.subscription.deleted", "sub_A"), CANCEL_MS)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: 0, expires: CANCEL_MS + 1, plan: "free", tier: null });
  expect(await markers(db)).toEqual([{ grant_key: `zeroed:${id}:${CANCEL_MS}`, account_id: id, micros: 0, period_end_unix_ms: CANCEL_MS + 1, created_at_unix_ms: CANCEL_MS, subscription_id: "sub_A" }]);
  // Yearly B is bought five minutes later: its month 1 lands although it ends before A's window would have.
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  const s = stripe({ sub_A: cancelledA(id), sub_B: subB(id) });
  expect((await deliver(db, s.fetcher, paid(invoiceB()), B_PAID_MS)).status).toBe(200);
  expect(err).not.toHaveBeenCalled();
  // An old invoice of A arriving late grants nothing (A has ended; the account is on B).
  const lateA = invoiceA({ id: "in_A_late", billing_reason: "subscription_cycle" });
  expect((await deliver(db, s.fetcher, paid(lateA), Date.UTC(2027, 1, 1))).status).toBe(200);
  expect(await rowOf(db, "in_A_late")).toBeUndefined();
  expect((await accounts.findAccountById(db, id))?.stripeSubscriptionId).toBe("sub_B");
  await expectBGetsItsYear(db, id);
});

it("a_same_subscription_late_invoice_after_a_cancel_zero_grants_nothing", async () => {
  const { db, id } = await monthlyA();
  const s = stripe({ sub_A: cancelledA(id) });
  expect((await deliver(db, s.fetcher, subEvent("customer.subscription.deleted", "sub_A"), CANCEL_MS)).status).toBe(200);
  // A's invoice for its still-open window, redelivered under a new event id and as another invoice id: nothing.
  // (Each re-applies the ended plan, which zeroes again in "cancel" mode: still 0, expiry just past that event.)
  for (const inv of [invoiceA(), invoiceA({ id: "in_A_late", billing_reason: "subscription_cycle" })]) {
    expect((await deliver(db, s.fetcher, paid(inv), CANCEL_MS + HOUR)).status).toBe(200);
  }
  expect(await allowance(db, id)).toEqual({ micros: 0, expires: CANCEL_MS + HOUR + 1, plan: "free", tier: null });
  expect(await rowOf(db, "in_A_late")).toBeUndefined();
  expect(await grants(db)).toBe(1); // in_A only
});

it("a_refund_zero_then_a_cancel_then_a_new_subscription_gets_its_year", async () => {
  const { db, id, fetcher } = await monthlyA();
  // A full refund of A's current invoice (A still active): "refund" zero, expiry past A's window end.
  expect((await deliver(db, fetcher, refundOf("pi_A"), CANCEL_MS)).status).toBe(200);
  expect(await allowance(db, id)).toMatchObject({ micros: 0, expires: A_END * 1000 + 1, plan: "cloud", tier: "plus" });
  // Then A is cancelled, and B is bought.
  expect((await deliver(db, stripe({ sub_A: cancelledA(id, CANCEL_MS + 60_000) }).fetcher, subEvent("customer.subscription.deleted", "sub_A"), CANCEL_MS + 60_000)).status).toBe(200);
  expect((await deliver(db, stripe({ sub_A: cancelledA(id, CANCEL_MS + 60_000), sub_B: subB(id) }).fetcher, paid(invoiceB()), B_PAID_MS)).status).toBe(200);
  // A's refund marker ends AFTER B's base grant: only its subscription (A) keeps it from blocking B's months.
  const refundMarker = (await markers(db))[0];
  expect(refundMarker).toMatchObject({ subscription_id: "sub_A", period_end_unix_ms: A_END * 1000 + 1 });
  expect(Number(refundMarker.period_end_unix_ms)).toBeGreaterThanOrEqual(B_M1_END);
  await expectBGetsItsYear(db, id);
});

it("a_new_subscription_paid_before_the_old_ones_cancel_is_processed_lands_over_a_refund_zero", async () => {
  // A is refunded and cancelled in Stripe, but B's invoice.paid is processed before A's deletion event:
  // the window is still A's refund zero (expiry A's end + 1 ms), later than B's month 1.
  const { db, id, fetcher } = await monthlyA();
  expect((await deliver(db, fetcher, refundOf("pi_A"), CANCEL_MS)).status).toBe(200);
  const s = stripe({ sub_A: cancelledA(id), sub_B: subB(id) });
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  expect((await deliver(db, s.fetcher, paid(invoiceB()), B_PAID_MS)).status).toBe(200);
  expect(err).not.toHaveBeenCalled();
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: B_M1_END, plan: "cloud", tier: "plus" });
  // A's deletion now is stale (the account is on B): nothing is zeroed.
  expect((await deliver(db, s.fetcher, subEvent("customer.subscription.deleted", "sub_A"), B_PAID_MS + 60_000)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: B_M1_END, plan: "cloud", tier: "plus" });
  await expectBGetsItsYear(db, id);
});

it("a_refund_of_last_months_invoice_keeps_this_months_allowance_and_a_refund_of_this_months_zeroes", async () => {
  const { db, id } = await account();
  const next = LINE_END + 30 * DAY;
  const in2 = invoice({ id: "in_2", billing_reason: "subscription_cycle", period_start: START, period_end: LINE_END }, { start: LINE_END, end: next });
  const { fetcher } = stripe({ sub_1: plusSub(id, {}, "price_plus_monthly", next), in_1: invoice(), in_2: in2 }, { pi_1: "in_1", pi_2: "in_2" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect((await deliver(db, fetcher, paid(in2), LINE_END * 1000 + 60_000)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: next * 1000, plan: "cloud", tier: "plus" });
  // Last month's invoice (its line ended) is fully refunded: this month's paid allowance stays, no marker.
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  expect((await deliver(db, fetcher, refundOf("pi_1"), LINE_END * 1000 + 2 * HOUR)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: next * 1000, plan: "cloud", tier: "plus" });
  expect(await markers(db)).toEqual([]);
  expect(log).toHaveBeenCalledWith("billing: plus refund of an invoice not behind the current window, allowance kept", "in_1", "sub_1");
  // This month's invoice fully refunded: zeroed.
  expect((await deliver(db, fetcher, refundOf("pi_2"), LINE_END * 1000 + 3 * HOUR)).status).toBe(200);
  expect(await allowance(db, id)).toMatchObject({ micros: 0, expires: next * 1000 + 1 });
  expect(await markers(db)).toEqual([expect.objectContaining({ subscription_id: "sub_1", period_end_unix_ms: next * 1000 + 1 })]);
});

it("a_refund_of_a_yearly_invoice_within_its_year_zeroes_and_blocks_the_cron_for_that_subscription_only", async () => {
  const { db, id, fetcher } = await yearlyAccount();
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(1);
  // In month 3 (month 3 not granted yet), the yearly invoice is fully refunded: its line runs to the year end, so it zeroes.
  expect((await deliver(db, fetcher, fullRefund(), monthStart(3) + 2 * HOUR)).status).toBe(200);
  expect(await allowance(db, id)).toMatchObject({ micros: 0, plan: "cloud", tier: "plus" });
  expect(await markers(db)).toEqual([expect.objectContaining({ subscription_id: "sub_1" })]);
  for (const n of [3, 4, 12]) await expectNothing(db, id, monthStart(n) + HOUR);
  // sub_1 is cancelled, and a new yearly sub_2 is bought in month 4: sub_2 gets its own months.
  const at = monthStart(4) + 3 * HOUR;
  const ended = yearlySub(id, { status: "canceled", ended_at: at / 1000 });
  expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"), at)).status).toBe(200);
  const s2Start = at / 1000 + 60;
  const s2End = addMonthsUtc(s2Start * 1000, 12) / 1000;
  const sub2 = yearlySub(id, { id: "sub_2", created: s2Start, items: { data: [{ id: "si_2", current_period_end: s2End, price: { id: "price_plus_yearly", recurring: { interval: "year" } } }] } });
  const inv2 = invoice({ id: "in_s2" }, { sub: "sub_2", item: "si_2", price: "price_plus_yearly", start: s2Start, end: s2End });
  expect((await deliver(db, stripe({ sub_1: ended, sub_2: sub2 }).fetcher, paid(inv2), (s2Start + 30) * 1000)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: addMonthsUtc(s2Start * 1000, 1), plan: "cloud", tier: "plus" });
  expect(await grantDueYearlyMonths(db, addMonthsUtc(s2Start * 1000, 1) + HOUR)).toBe(1);
  expect(await rowOf(db, "in_s2:m2")).toMatchObject({ subscription_id: "sub_2", period_end_unix_ms: addMonthsUtc(s2Start * 1000, 2) });
});

it("a_refund_that_arrives_before_its_invoice_paid_leaves_a_tombstone_and_the_invoice_grants_nothing", async () => {
  for (const yearly of [false, true]) {
    const { db, id } = await account();
    const sub = yearly ? yearlySub(id) : plusSub(id);
    const inv = yearly ? yearlyInvoice() : invoice();
    const { fetcher } = stripe({ sub_1: sub, in_1: inv }, { pi_inv: "in_1" });
    const completed = event("checkout.session.completed", { id: "cs_1", object: "checkout.session", mode: "subscription", subscription: "sub_1", client_reference_id: id });
    expect((await deliver(db, fetcher, completed)).status).toBe(200);
    // The invoice's invoice.paid is still being retried when a full refund lands.
    expect((await deliver(db, fetcher, fullRefund(), NOW + 60_000)).status).toBe(200);
    expect(await rowOf(db, "in_1")).toEqual({ grant_key: "in_1", micros: 0, period_end_unix_ms: 0, line_start_unix_ms: null, line_end_unix_ms: null, subscription_id: "sub_1" });
    expect(await markers(db)).toEqual([expect.objectContaining({ subscription_id: "sub_1" })]);
    // Now invoice.paid lands: a repeat key, no allowance, and logged.
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    expect((await deliver(db, fetcher, paid(inv), NOW + 120_000)).status).toBe(200);
    expect((await allowance(db, id)).micros).toBe(0);
    expect(err).toHaveBeenCalledWith("billing: paid plus invoice was refunded before it was paid, no grant", "in_1", "sub_1", id);
    expect(await rowOf(db, "in_1")).toMatchObject({ micros: 0 });
    // The refund redelivered: the tombstone is not a current grant, so nothing more is zeroed or written.
    vi.spyOn(console, "log").mockImplementation(() => {});
    expect((await deliver(db, fetcher, fullRefund(), NOW + 180_000)).status).toBe(200);
    expect((await markers(db)).length).toBe(1);
    // A tombstone is never a yearly base grant.
    if (yearly) for (const n of [2, 3]) await expectNothing(db, id, monthStart(n) + HOUR);
    err.mockRestore();
  }
});

it("a_refund_of_an_older_invoice_that_never_granted_keeps_the_current_allowance", async () => {
  // in_1 was paid only after its line ended (no grant row), in_2 granted on time; then in_1 is refunded.
  const { db, id } = await account();
  const next = LINE_END + 30 * DAY;
  const in2 = invoice({ id: "in_2", billing_reason: "subscription_cycle", period_start: START, period_end: LINE_END }, { start: LINE_END, end: next });
  const { fetcher } = stripe({ sub_1: plusSub(id, {}, "price_plus_monthly", next), in_1: invoice(), in_2: in2 }, { pi_1: "in_1" });
  vi.spyOn(console, "error").mockImplementation(() => {});
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  expect((await deliver(db, fetcher, paid(in2), LINE_END * 1000 + 60_000)).status).toBe(200);
  expect((await deliver(db, fetcher, paid(invoice()), LINE_END * 1000 + 120_000)).status).toBe(200);
  expect(await rowOf(db, "in_1")).toBeUndefined();
  expect((await deliver(db, fetcher, refundOf("pi_1"), LINE_END * 1000 + 2 * HOUR)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: SETTING, expires: next * 1000, plan: "cloud", tier: "plus" });
  expect(await markers(db)).toEqual([]);
  expect(await rowOf(db, "in_1")).toBeUndefined();
  expect(log).toHaveBeenCalledWith("billing: plus refund of an invoice not behind the current window, allowance kept", "in_1", "sub_1");
});

it("a_paid_plus_invoice_refused_by_the_expiry_guard_is_logged", async () => {
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id), in_1: invoice() }, { pi_inv: "in_1" });
  expect((await deliver(db, fetcher, paid(invoice()))).status).toBe(200);
  expect((await deliver(db, fetcher, fullRefund(), NOW + 60_000)).status).toBe(200);
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  // Another paid invoice of the same subscription for the refunded window: written, refused, logged by ids.
  expect((await deliver(db, fetcher, paid(invoice({ id: "in_late" })), NOW + 120_000)).status).toBe(200);
  expect((await allowance(db, id)).micros).toBe(0);
  expect(err).toHaveBeenCalledWith("billing: paid plus invoice grant refused by the expiry guard, allowance not refilled", "in_late", "sub_1", id);
  // A plain redelivery of a granted invoice is not an error.
  err.mockClear();
  expect((await deliver(db, fetcher, paid(invoice()), NOW + 180_000)).status).toBe(200);
  expect(err).not.toHaveBeenCalled();
});

it("rows_without_a_subscription_id_keep_the_old_any_marker_blocks_rule", async () => {
  // Rows written before migration 0007's subscription_id column (NULL).
  const legacy = { subscriptionId: null, mode: "refund" } as const;
  // A legacy base grant: any marker blocks it, even one of another subscription.
  {
    const { db, id } = await account();
    await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
    expect(await grantAllowance(db, { accountId: id, grantKey: "in_1", micros: SETTING, periodEndUnixMs: MONTH_1_END, lineStartUnixMs: S_MS, lineEndUnixMs: Y_END_MS }, NOW)).toBe(true);
    await zeroAllowance(db, id, NOW + HOUR, { subscriptionId: "sub_other", mode: "refund" });
    for (const n of [2, 3]) await expectNothing(db, id, monthStart(n) + HOUR);
  }
  // A legacy marker blocks a base grant that has a subscription id.
  {
    const { db, id } = await yearlyAccount();
    await zeroAllowance(db, id, NOW + HOUR, legacy);
    for (const n of [2, 3]) await expectNothing(db, id, monthStart(n) + HOUR);
  }
  // A legacy marker never lets another subscription's grant past the forward guard.
  {
    const { db, id } = await monthlyA();
    await zeroAllowance(db, id, CANCEL_MS, legacy);
    expect(await grantAllowance(db, { accountId: id, grantKey: "in_B", micros: SETTING, periodEndUnixMs: B_M1_END, subscriptionId: "sub_B" }, B_PAID_MS)).toBe(true);
    expect(await allowance(db, id)).toMatchObject({ micros: 0, expires: A_END * 1000 + 1 });
  }
});

it("a_paid_plus_invoice_whose_plus_line_is_past_the_first_page_reads_the_lines_once", async () => {
  // The payload's first page has only an invoice item; Stripe says there are more lines.
  const firstPage = { data: [{ id: "il_x", type: "invoiceitem", period: { start: START, end: LINE_END }, price: { id: "price_plus_monthly" } }], has_more: true };
  const plusLine = invoice().lines.data[0];
  {
    const { db, id } = await account();
    const { fetcher, calls } = stripe({ sub_1: plusSub(id), lines: { object: "list", data: [firstPage.data[0], plusLine], has_more: false } });
    expect((await deliver(db, fetcher, paid(invoice({ lines: firstPage })))).status).toBe(200);
    expect(calls.filter((c) => c.includes("/v1/invoices/"))).toEqual(["GET https://api.stripe.com/v1/invoices/in_1/lines?limit=100"]);
    expect(await allowance(db, id)).toMatchObject({ micros: SETTING, expires: LINE_END * 1000 });
  }
  // Still no Plus line in what Stripe returns: the "no readable Plus line" case, retried.
  {
    const { db } = await account();
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const { fetcher } = stripe({ sub_1: plusSub("x"), lines: { object: "list", data: [firstPage.data[0]], has_more: true } });
    await db.execute("UPDATE account SET id = 'x'", []);
    const evt = paid(invoice({ lines: firstPage }));
    expect((await deliver(db, fetcher, evt)).status).toBe(500);
    expect(await recorded(db, evt.id)).toBe(false);
    expect(err).toHaveBeenCalledWith("billing: paid plus invoice without a plus line period", "in_1");
    expect(await grants(db)).toBe(0);
  }
  // The lines read fails: retried.
  {
    const { db, id } = await account();
    const { fetcher } = stripe({ sub_1: plusSub(id) });
    const evt = paid(invoice({ lines: firstPage }));
    expect((await deliver(db, fetcher, evt)).status).toBe(500);
    expect(await recorded(db, evt.id)).toBe(false);
    expect(await grants(db)).toBe(0);
  }
});

it("the_hourly_cron_without_migration_0007_logs_one_clear_line", async () => {
  const { db, id } = await yearlyAccount();
  void id;
  for (const c of ["subscription_id", "line_start_unix_ms", "line_end_unix_ms"]) await db.execute(`ALTER TABLE ai_allowance_grant DROP COLUMN ${c}`, []);
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  expect(await grantDueYearlyMonths(db, monthStart(2) + HOUR)).toBe(0);
  expect(err.mock.calls).toEqual([["plus grants need migration 0007"]]);
});

it("a_stale_active_write_after_a_month_one_cancel_gets_no_cron_months", async () => {
  // The accepted m7 race: a yearly subscription is cancelled in month 1 ("cancel" zero, expiry now + 1, before
  // G1's end), then a stale "active" read puts the plan back. Its own cancel marker, written after G1, still blocks.
  const { db, id } = await yearlyAccount();
  const at = NOW + HOUR;
  const ended = yearlySub(id, { status: "canceled", ended_at: at / 1000 });
  expect((await deliver(db, stripe({ sub_1: ended }).fetcher, subEvent("customer.subscription.deleted"), at)).status).toBe(200);
  expect(await allowance(db, id)).toEqual({ micros: 0, expires: at + 1, plan: "free", tier: null });
  expect(await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true })).toBe(true);
  for (const n of [2, 3, 12]) await expectNothing(db, id, monthStart(n) + HOUR);
});

it("a_refund_before_invoice_paid_zeroes_in_refund_mode", async () => {
  // A still-live window of the same subscription (from an earlier grant): the tombstone path's zero moves the
  // expiry past it (MAX(old, now) + 1), not to now + 1, so no late grant of that subscription ending inside it lands.
  const { db, id } = await account();
  const { fetcher } = stripe({ sub_1: plusSub(id), in_1: invoice() }, { pi_inv: "in_1" });
  await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: LINE_END * 1000, interval: "month", renews: true });
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_prev", micros: SETTING, periodEndUnixMs: LINE_END * 1000, subscriptionId: "sub_1" }, NOW)).toBe(true);
  // Refunded with no row for in_1 yet (and in_prev's row is another invoice): zero + tombstone.
  expect((await deliver(db, fetcher, fullRefund(), NOW + 60_000)).status).toBe(200);
  expect(await rowOf(db, "in_1")).toMatchObject({ micros: 0, period_end_unix_ms: 0 });
  expect(await allowance(db, id)).toMatchObject({ micros: 0, expires: LINE_END * 1000 + 1 });
});

it("the_refund_lookup_matches_month_rows_of_a_yearly_invoice_without_line_bounds", async () => {
  // Rows without line bounds (as before 0007): in_1's own row has ended (month 1), its month 2 row has not.
  const { db, id } = await account();
  await accounts.applyPlan(db, id, "sub_1", { plan: "cloud", tier: "plus", untilUnixMs: Y_END_MS, interval: "year", renews: true });
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_1", micros: SETTING, periodEndUnixMs: MONTH_1_END, subscriptionId: "sub_1" }, NOW)).toBe(true);
  expect(await grantAllowance(db, { accountId: id, grantKey: "in_1:m2", micros: SETTING, periodEndUnixMs: MONTH_2_END, subscriptionId: "sub_1" }, monthStart(2))).toBe(true);
  const { fetcher } = stripe({ sub_1: yearlySub(id), in_1: yearlyInvoice() }, { pi_inv: "in_1" });
  expect((await deliver(db, fetcher, fullRefund(), monthStart(2) + HOUR)).status).toBe(200);
  expect(await allowance(db, id)).toMatchObject({ micros: 0, expires: MONTH_2_END + 1 });
  expect(await markers(db)).toEqual([expect.objectContaining({ subscription_id: "sub_1" })]);
});
