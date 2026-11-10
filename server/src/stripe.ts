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

/** Minimal Stripe REST call. Never throws; logs the status only. A redirect is
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
  untilUnixMs: number;
  interval: "month" | "year" | null;
  /** False when the subscription is set to cancel (at period end or a set date). */
  renews: boolean;
}

/** Maps a Stripe subscription to the plan we store. null = leave the account as is. */
export function planFromSubscription(sub: any, nowUnixMs: number): PlanState | null {
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
      return { plan: "cloud", untilUnixMs: until * 1000, interval: interval === "month" || interval === "year" ? interval : null, renews };
    }
    case "canceled":
    case "unpaid":
    case "incomplete_expired":
    case "paused":
      return { plan: "free", untilUnixMs: (sub.ended_at ?? nowUnixMs / 1000) * 1000, interval: null, renews: false };
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
