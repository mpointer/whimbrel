import type { Env } from "./index";

export function billingConfigured(env: Env): boolean {
  return !!(env.STRIPE_SECRET_KEY && env.STRIPE_WEBHOOK_SECRET && env.STRIPE_PRICE_MONTHLY && env.STRIPE_PRICE_YEARLY);
}

const TOLERANCE_SECONDS = 300;

function hexToBytes(hex: string): Uint8Array<ArrayBuffer> | null {
  if (hex.length === 0 || hex.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(hex)) return null;
  const out = new Uint8Array(new ArrayBuffer(hex.length / 2));
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** Checks a `Stripe-Signature` header (`t=…,v1=…[,v1=…]`) against the raw body.
 * HMAC-SHA256 over `${t}.${rawBody}`, compared in constant time by
 * `crypto.subtle.verify`; timestamps more than 300 s from now are rejected. */
export async function verifyStripeSignature(rawBody: string, header: string | null, secret: string, nowUnixMs: number): Promise<boolean> {
  if (!header || !secret) return false;
  let t: string | null = null;
  const sigs: string[] = [];
  for (const part of header.split(",")) {
    const i = part.indexOf("=");
    if (i < 0) continue;
    const k = part.slice(0, i).trim();
    const v = part.slice(i + 1).trim();
    if (k === "t") t = v;
    else if (k === "v1") sigs.push(v);
  }
  if (t === null || !/^\d+$/.test(t) || sigs.length === 0) return false;
  if (Math.abs(nowUnixMs / 1000 - Number(t)) > TOLERANCE_SECONDS) return false;
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey("raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["verify"]);
  const data = enc.encode(`${t}.${rawBody}`);
  let ok = false;
  for (const s of sigs) {
    const bytes = hexToBytes(s);
    if (bytes && (await crypto.subtle.verify("HMAC", key, bytes, data))) ok = true;
  }
  return ok;
}

/** Minimal Stripe REST call. Never throws; logs the status only. It sends
 * no Stripe-Version header, so every read comes back in the Stripe
 * account's default API version (the webhook's version pin covers event
 * payloads only). That default is 2025-10-29.clover, confirmed by Mike on
 * 2026-11-19; changing it in the Stripe Dashboard changes these reads. A redirect is
 * not followed (`redirect: "manual"`; Workers reject "error"), so the
 * Authorization header can't follow one to another host: a 3xx is just !ok. */
export async function stripeRequest(
  key: string,
  method: "GET" | "POST" | "DELETE",
  path: string,
  params: Record<string, string>,
  fetcher: typeof fetch,
): Promise<{ ok: boolean; status: number; body: any }> {
  const qs = new URLSearchParams(params).toString();
  const withQuery = method !== "POST";
  const url = `https://api.stripe.com/v1/${path}${withQuery && qs ? `?${qs}` : ""}`;
  const headers: Record<string, string> = { authorization: `Bearer ${key}` };
  if (!withQuery) headers["content-type"] = "application/x-www-form-urlencoded";
  try {
    const res = await fetcher(url, { method, headers, body: withQuery ? undefined : qs, redirect: "manual" });
    let body: any = null;
    try {
      body = await res.json();
    } catch {
      // non-JSON body
    }
    // Stripe's error code and the parameter it names, never its message (it can quote an email).
    if (!res.ok) console.log(`stripe ${method} ${path.split("/")[0]} -> ${res.status} ${body?.error?.code ?? ""} ${body?.error?.param ?? ""}`.trimEnd());
    return { ok: res.ok, status: res.status, body };
  } catch {
    console.log(`stripe ${method} ${path.split("/")[0]} -> network error`);
    return { ok: false, status: 0, body: null };
  }
}

export interface PlanState {
  plan: "free" | "cloud";
  /** Which paid product; null on the free branch (an ended plan has no tier). */
  tier: "cloud" | "plus" | null;
  untilUnixMs: number;
  interval: "month" | "year" | null;
  /** False when the subscription is set to cancel (at period end or a set date). */
  renews: boolean;
}

/** True when a subscription is Plus: its `metadata.tier` is "plus" (set at
 * checkout, so a renewal on a retired Plus price stays Plus) or its item's
 * price is one of `plusPriceIds`. Says nothing about whether it is live. */
export function isPlusSubscription(sub: any, plusPriceIds: string[]): boolean {
  const price = sub?.items?.data?.[0]?.price?.id;
  return sub?.metadata?.tier === "plus" || (typeof price === "string" && plusPriceIds.includes(price));
}

/** The period, in ms, of a paid invoice's Plus line: a subscription line
 * (never an invoice item, never a proration) on a Plus price or on one of
 * the subscription's items (a retired Plus price). Basil and later shapes
 * say so in `line.parent.type === "subscription_item_details"`; an older
 * shape without `parent` is accepted when it is not a proration and not an
 * `invoiceitem` line. Never the invoice's top-level period, which is the
 * previous period on a renewal and zero-length on the first invoice. null
 * when no such line has a whole period start and end; "invalid" when the
 * only such lines have whole bounds that are not a range (start at or after
 * end), which no retry of the same paid invoice can change. The result
 * also carries the line's pre-discount `amountCents`. */
export function plusLinePeriod(invoice: any, sub: any, plusPriceIds: string[]): { startMs: number; endMs: number; amountCents: number | null } | "invalid" | null {
  const lines: any[] = Array.isArray(invoice?.lines?.data) ? invoice.lines.data : [];
  const items = new Set<string>(
    (Array.isArray(sub?.items?.data) ? sub.items.data : []).map((i: any) => i?.id).filter((id: unknown): id is string => typeof id === "string"),
  );
  let invalid = false;
  for (const line of lines) {
    let price: unknown;
    let item: unknown;
    const parent = line?.parent;
    if (parent != null) {
      if (parent.type !== "subscription_item_details") continue;
      const details = parent.subscription_item_details;
      if (details?.proration === true) continue;
      price = line?.pricing?.price_details?.price;
      item = details?.subscription_item;
    } else {
      if (line?.proration === true || line?.type === "invoiceitem") continue;
      price = line?.price?.id;
      item = line?.subscription_item;
    }
    const ours = (typeof price === "string" && plusPriceIds.includes(price)) || (typeof item === "string" && items.has(item));
    const start = line?.period?.start;
    const end = line?.period?.end;
    if (ours && Number.isSafeInteger(start) && Number.isSafeInteger(end) && end > 0) {
      // `amountCents` is the line's amount before any discount or credit
      // (null when Stripe sent none): a $0 paid invoice grants only when it is positive.
      if (start < end) return { startMs: start * 1000, endMs: end * 1000, amountCents: Number.isSafeInteger(line?.amount) ? line.amount : null };
      invalid = true;
    }
  }
  return invalid ? "invalid" : null;
}

/** Maps a Stripe subscription to the plan we store. null = leave the account as is.
 * A paid subscription is Plus when its `metadata.tier` is "plus" (set at
 * checkout, so a renewal on a retired Plus price stays Plus) or its item's
 * price is one of `plusPriceIds`; otherwise it is Cloud. */
export function planFromSubscription(sub: any, nowUnixMs: number, plusPriceIds: string[] = []): PlanState | null {
  switch (sub?.status) {
    case "active":
    case "trialing":
    case "past_due": {
      const item = sub.items?.data?.[0];
      const end = item?.current_period_end ?? sub.current_period_end;
      if (typeof end !== "number") return null;
      const interval = item?.price?.recurring?.interval;
      // A cancelled subscription stays active until it ends: cancel_at (or the period end) says when.
      const cancelAt = typeof sub.cancel_at === "number" ? sub.cancel_at : null;
      const renews = cancelAt === null && sub.cancel_at_period_end !== true;
      const until = cancelAt !== null ? Math.min(cancelAt, end) : end;
      return { plan: "cloud", tier: isPlusSubscription(sub, plusPriceIds) ? "plus" : "cloud", untilUnixMs: until * 1000, interval: interval === "month" || interval === "year" ? interval : null, renews };
    }
    case "canceled":
    case "unpaid":
    case "incomplete_expired":
    case "paused":
      return { plan: "free", tier: null, untilUnixMs: (sub.ended_at ?? nowUnixMs / 1000) * 1000, interval: null, renews: false };
    default:
      return null;
  }
}

/** The subscription a webhook event is about, or null for events we ignore. */
export function subscriptionIdOf(event: any): string | null {
  const type: string = event?.type ?? "";
  const obj = event?.data?.object;
  if (type === "checkout.session.completed") return obj?.subscription ?? null;
  if (type.startsWith("customer.subscription.")) return obj?.id ?? null;
  if (type.startsWith("invoice.")) return obj?.parent?.subscription_details?.subscription ?? obj?.subscription ?? null;
  return null;
}

/** True for the Stripe events of a Whimbrel AI credit purchase that can be
 * told from the payload alone: a payment-mode Checkout Session (completed, or
 * its delayed payment succeeded), or a refunded
 * charge, carrying `metadata.kind = "ai_credit"`. A refunded charge without
 * that metadata may still be one; its PaymentIntent says. */
export function isAiCreditEvent(event: any): boolean {
  const obj = event?.data?.object;
  if (obj?.metadata?.kind !== "ai_credit") return false;
  if (event?.type === "checkout.session.completed" || event?.type === "checkout.session.async_payment_succeeded") return obj?.mode === "payment";
  return event?.type === "charge.refunded";
}
