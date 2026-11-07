import { beforeEach, describe, expect, it } from "vitest";
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { handle, type Env } from "../src/app";

const RESEND = "https://api.resend.com/emails";
const TURNSTILE = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
const MIKE = "mike@futuremeanswered.com";
const SECRET = "test-review-secret";

/** Records every outbound call; Resend and Turnstile answer as configured. */
function fakeFetch(overrides: Record<string, () => Response> = {}) {
  const calls: { url: string; init?: RequestInit }[] = [];
  const routes: Record<string, () => Response> = { [RESEND]: () => Response.json({ id: "e1" }), ...overrides };
  const fn = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    calls.push({ url, init });
    const route = routes[url];
    return route ? route() : new Response("not found", { status: 404 });
  }) as typeof fetch;
  const emails = () => calls.filter((c) => c.url === RESEND).map((c) => JSON.parse(String(c.init!.body)) as { to: string[]; subject: string; text: string; html: string });
  return { fn, calls, emails };
}

const baseEnv = { RESEND_API_KEY: "re_test", EMAIL_FROM: "Whimbrel <whimbrel@futuremeanswered.com>", EMAIL_REPLY_TO: MIKE, LEAD_ALERT_EMAIL: MIKE, BETA_REVIEW_SECRET: SECRET };

async function call(request: Request, opts: { env?: Partial<Env>; fetch?: ReturnType<typeof fakeFetch> } = {}) {
  const ctx = createExecutionContext();
  const f = opts.fetch ?? fakeFetch();
  const response = await handle(request, { ASSETS: env.ASSETS, DB: env.DB, UPDATES_URL: "https://updates.whimbrelapp.com", ...baseEnv, ...opts.env }, ctx, f.fn);
  await waitOnExecutionContext(ctx);
  return response;
}

const signup = (body: Record<string, unknown>) =>
  new Request("https://whimbrel.test/api/beta", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

const valid = { name: "Ada Lovelace", email: "Ada@Example.com", phones: "android", computer: "mac" };

/** Signs up and returns the Approve and Decline links from Mike's alert. */
async function signUp(body: Record<string, unknown> = valid, extra: Partial<Env> = {}) {
  const f = fakeFetch();
  const res = await call(signup(body), { fetch: f, env: extra });
  expect(res.status).toBe(200);
  const alert = f.emails().find((e) => e.to[0] === MIKE)!;
  const link = (label: string) => new URL(alert.text.match(new RegExp(`${label}: (\\S+)`))![1]);
  return { f, approve: link("Approve"), decline: link("Decline") };
}

/** Posts the review page's form, as its confirm button does. */
function review(link: URL, extra: Record<string, string> = {}) {
  const form = new URLSearchParams({ ...Object.fromEntries(link.searchParams), ...extra });
  return new Request("https://whimbrel.test/beta/review", { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: form.toString() });
}

const row = () => env.DB.prepare("SELECT * FROM beta_signups").first<Record<string, unknown>>();

beforeEach(async () => {
  await env.DB.exec("DELETE FROM beta_signups; DELETE FROM downloads; DELETE FROM email_preference_events; DELETE FROM leads;");
});

describe("POST /api/beta", () => {
  it("saves the request and the lead, emails the tester once and alerts Mike with review links", async () => {
    const { f } = await signUp({ ...valid, marketingOptIn: true });
    expect(await row()).toMatchObject({ email: "ada@example.com", phones: "android", computer: "mac", status: "pending", install_email_sent_at: null });
    const lead = await env.DB.prepare("SELECT * FROM leads").first<Record<string, unknown>>();
    expect(lead).toMatchObject({ email: "ada@example.com", platform: "mac", marketing_opt_in: 1, product_emails: "opted_out" });
    expect(lead!.followup_sent_at).not.toBeNull(); // no desktop "your download links" note

    const tester = f.emails().filter((e) => e.to[0] === "ada@example.com");
    expect(tester.map((e) => e.subject)).toEqual(["We got your Whimbrel beta request"]);
    const alert = f.emails().find((e) => e.to[0] === MIKE)!;
    expect(alert.subject).toBe("Beta request: Ada Lovelace, Android");
    expect(alert.text).toContain("Computer: Mac");
  });

  it("rejects missing or invalid fields and stores nothing", async () => {
    const res = await call(signup({ name: " ", email: "nope", phones: "pager", computer: "linux" }));
    expect(res.status).toBe(400);
    const body = (await res.json()) as { errors: Record<string, string> };
    expect(Object.keys(body.errors).sort()).toEqual(["computer", "email", "name", "phones"]);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM beta_signups").first("n")).toBe(0);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM leads").first("n")).toBe(0);
  });

  it("answers a filled honeypot like a success but stores and sends nothing", async () => {
    const f = fakeFetch();
    const res = await call(signup({ ...valid, whimbrel_hp: "x" }), { fetch: f });
    expect(await res.json()).toEqual({ ok: true, next: "/mobile-beta/thanks/" });
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM leads").first("n")).toBe(0);
    expect(f.emails()).toHaveLength(0);
  });

  it("requires a valid Turnstile token when TURNSTILE_SECRET is set", async () => {
    const failing = fakeFetch({ [TURNSTILE]: () => Response.json({ success: false }) });
    const res = await call(signup({ ...valid, "cf-turnstile-response": "bad" }), { env: { TURNSTILE_SECRET: "s" }, fetch: failing });
    expect(res.status).toBe(403);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM beta_signups").first("n")).toBe(0);
  });

  it("a repeat sign-up updates the pending row and doesn't email the tester again", async () => {
    await signUp();
    const first = await row();
    const { f } = await signUp({ ...valid, phones: "both", computer: "none" });
    const rows = await env.DB.prepare("SELECT * FROM beta_signups").all<Record<string, unknown>>();
    expect(rows.results).toHaveLength(1);
    expect(rows.results[0]).toMatchObject({ id: first!.id, phones: "both", computer: "none", status: "pending" });
    expect(f.emails().map((e) => e.to[0])).toEqual([MIKE]);
    expect(f.emails()[0].subject).toBe("Updated beta request: Ada Lovelace, iPhone and Android");
    // "none" keeps the lead's real desktop platform.
    expect(await env.DB.prepare("SELECT platform FROM leads").first("platform")).toBe("mac");
  });

  it("saves the request without BETA_REVIEW_SECRET and says the links are unavailable", async () => {
    const f = fakeFetch();
    await call(signup(valid), { fetch: f, env: { BETA_REVIEW_SECRET: undefined } });
    expect(await row()).toMatchObject({ status: "pending" });
    const alert = f.emails().find((e) => e.to[0] === MIKE)!;
    expect(alert.text).toContain("Review links are unavailable");
    expect(alert.text).not.toContain("/beta/review");
  });
});

describe("/beta/review", () => {
  it("refuses a bad or missing signature, and a signature for the other action", async () => {
    const { approve, decline } = await signUp();
    const tampered = new URL(approve);
    tampered.searchParams.set("sig", "AAAA" + approve.searchParams.get("sig")!.slice(4));
    const missing = new URL(approve);
    missing.searchParams.delete("sig");
    const swapped = new URL(approve);
    swapped.searchParams.set("sig", decline.searchParams.get("sig")!);
    for (const link of [tampered, missing, swapped]) {
      expect((await call(new Request(link))).status).toBe(403);
      expect((await call(review(link))).status).toBe(403);
    }
    expect((await call(new Request(approve), { env: { BETA_REVIEW_SECRET: undefined } })).status).toBe(403);
    expect(await row()).toMatchObject({ status: "pending", install_email_sent_at: null });
  });

  it("GET only shows the request and a confirm button", async () => {
    const { approve, decline } = await signUp();
    const f = fakeFetch();
    for (const link of [approve, decline]) {
      const res = await call(new Request(link), { fetch: f });
      expect(res.status).toBe(200);
      const html = await res.text();
      expect(html).toContain("ada@example.com");
      expect(html).toContain('method="post"');
    }
    expect(f.emails()).toHaveLength(0);
    expect(await row()).toMatchObject({ status: "pending", decided_at: null, install_email_sent_at: null });
  });

  it("approve sends one install email with only the Android section, and approving again sends nothing", async () => {
    const { approve } = await signUp();
    const f = fakeFetch();
    expect((await call(review(approve), { fetch: f })).status).toBe(200);
    const sent = f.emails();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ to: ["ada@example.com"], subject: "Your Whimbrel phone beta is ready" });
    expect(sent[0].text).toContain("https://github.com/mpointer/whimbrel/releases/latest");
    expect(sent[0].text).toContain("ANDROID");
    expect(sent[0].text).not.toContain("IPHONE");
    expect(sent[0].text).not.toContain("TestFlight");
    expect(await row()).toMatchObject({ status: "approved" });
    expect((await row())!.install_email_sent_at).not.toBeNull();

    const again = fakeFetch();
    await call(review(approve), { fetch: again });
    expect(again.emails()).toHaveLength(0);
  });

  it("approve for both phones sends both sections with the TestFlight link", async () => {
    const { approve } = await signUp({ ...valid, phones: "both" });
    const f = fakeFetch();
    await call(review(approve), { fetch: f, env: { TESTFLIGHT_PUBLIC_URL: "https://testflight.apple.com/join/abc", ANDROID_RELEASE_URL: "https://example.com/apk" } });
    const [email] = f.emails();
    expect(f.emails()).toHaveLength(1);
    expect(email.text).toContain("IPHONE");
    expect(email.text).toContain("https://testflight.apple.com/join/abc");
    expect(email.text).toContain("ANDROID");
    expect(email.text).toContain("https://example.com/apk");
  });

  it("approve for iPhone without TESTFLIGHT_PUBLIC_URL says the link follows and leaves the request unsent", async () => {
    const { approve } = await signUp({ ...valid, phones: "iphone" });
    const f = fakeFetch();
    await call(review(approve), { fetch: f, env: { TESTFLIGHT_PUBLIC_URL: "" } });
    const [email] = f.emails();
    expect(email.text).toContain("Your TestFlight link comes in a follow-up email.");
    expect(email.text).not.toContain("ANDROID");
    expect(await row()).toMatchObject({ status: "approved", install_email_sent_at: null });
  });

  it("decline marks the request declined and sends nothing", async () => {
    const { decline } = await signUp();
    const f = fakeFetch();
    await call(review(decline), { fetch: f });
    expect(f.emails()).toHaveLength(0);
    expect(await row()).toMatchObject({ status: "declined" });
    expect((await row())!.decided_at).not.toBeNull();
  });

  it('"Decline and tell them" sends one short email, and only once', async () => {
    const { decline } = await signUp();
    const f = fakeFetch();
    await call(review(decline, { tell: "1" }), { fetch: f });
    await call(review(decline, { tell: "1" }), { fetch: f });
    expect(f.emails()).toHaveLength(1);
    expect(f.emails()[0]).toMatchObject({ to: ["ada@example.com"], subject: "About your Whimbrel beta request" });
    expect(await row()).toMatchObject({ status: "declined" });
  });
});

describe("pages", () => {
  it("serves the sign-up page in English, Spanish and French, linked from home and download", async () => {
    for (const path of ["/mobile-beta/", "/es/mobile-beta/", "/fr/mobile-beta/"]) {
      const html = await (await call(new Request(`https://whimbrel.test${path}`))).text();
      expect(html, path).toContain('id="beta-form"');
      expect(html, path).toContain('name="phones"');
    }
    for (const path of ["/", "/download/", "/es/", "/es/download/", "/fr/", "/fr/download/"]) {
      const html = await (await call(new Request(`https://whimbrel.test${path}`))).text();
      expect(html, path).toMatch(/href="(\/es|\/fr)?\/mobile-beta\/"/);
    }
  });
});
