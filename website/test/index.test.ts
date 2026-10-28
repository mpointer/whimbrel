import { beforeEach, describe, expect, it } from "vitest";
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { handle, LEAD_COOKIE, type Env } from "../src/app";

const UPDATES = "https://updates.whimbrelapp.com";

interface Call {
  url: string;
  init?: RequestInit;
}

/** A stand-in for global fetch: serves the two release manifests and records every call
 * (Turnstile, the lead webhook) so tests can assert on outbound traffic. */
function fakeFetch(overrides: Record<string, () => Response> = {}) {
  const calls: Call[] = [];
  const routes: Record<string, () => Response> = {
    [`${UPDATES}/version.json`]: () => Response.json({ version: "0.9.21", url: `${UPDATES}/Whimbrel-0.9.21.zip`, sha256: "x" }),
    [`${UPDATES}/windows/latest.json`]: () =>
      Response.json({ version: "0.9.21", platforms: { "windows-x86_64": { signature: "s", url: `${UPDATES}/windows/Whimbrel_0.9.21_x64-setup.exe` } } }),
    ...overrides,
  };
  const fn = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    calls.push({ url, init });
    const route = routes[url];
    return route ? route() : new Response("not found", { status: 404 });
  }) as typeof fetch;
  return { fn, calls };
}

function testEnv(extra: Partial<Env> = {}): Env {
  return { ASSETS: env.ASSETS, DB: env.DB, UPDATES_URL: UPDATES, ...extra };
}

async function call(request: Request, opts: { env?: Partial<Env>; fetch?: ReturnType<typeof fakeFetch> } = {}) {
  const ctx = createExecutionContext();
  const f = opts.fetch ?? fakeFetch();
  const response = await handle(request, testEnv(opts.env), ctx, f.fn);
  await waitOnExecutionContext(ctx);
  return response;
}

function registerJson(body: Record<string, unknown>) {
  return new Request("https://whimbrel.test/api/register", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

const valid = { name: "Ada Lovelace", email: "Ada@Example.com", platform: "mac", company: "Analytical", role: "engineer", teamSize: "2-10" };

function cookieFrom(response: Response): string {
  const header = response.headers.get("set-cookie") ?? "";
  const m = header.match(new RegExp(`${LEAD_COOKIE}=([^;]+)`));
  if (!m) throw new Error(`no lead cookie in: ${header}`);
  return `${LEAD_COOKIE}=${m[1]}`;
}

beforeEach(async () => {
  await env.DB.exec("DELETE FROM downloads; DELETE FROM leads;");
});

describe("POST /api/register", () => {
  it("stores a valid lead, normalizes the email, and sets the lead cookie", async () => {
    const res = await call(registerJson({ ...valid, marketingOptIn: true, utm_source: "newsletter", landing_page: "/features/meeting-notes/" }));
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, next: "/download/thanks/?platform=mac" });
    const cookie = res.headers.get("set-cookie")!;
    expect(cookie).toContain("HttpOnly");
    expect(cookie).toContain("Secure");
    expect(cookie).toContain("SameSite=Lax");

    const row = await env.DB.prepare("SELECT * FROM leads").first<Record<string, unknown>>();
    expect(row).toMatchObject({
      email: "ada@example.com",
      name: "Ada Lovelace",
      company: "Analytical",
      role: "engineer",
      team_size: "2-10",
      platform: "mac",
      marketing_opt_in: 1,
      utm_source: "newsletter",
      landing_page: "/features/meeting-notes/",
      registrations: 1,
    });
  });

  it("rejects missing or invalid fields with per-field errors and stores nothing", async () => {
    const res = await call(registerJson({ name: " ", email: "not-an-email", platform: "linux" }));
    expect(res.status).toBe(400);
    const body = (await res.json()) as { ok: boolean; errors: Record<string, string> };
    expect(body.ok).toBe(false);
    expect(Object.keys(body.errors).sort()).toEqual(["email", "name", "platform"]);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM leads").first("n")).toBe(0);
  });

  it("drops unknown select values instead of rejecting the lead", async () => {
    const res = await call(registerJson({ ...valid, role: "astronaut", teamSize: "lots" }));
    expect(res.status).toBe(200);
    const row = await env.DB.prepare("SELECT role, team_size FROM leads").first();
    expect(row).toEqual({ role: null, team_size: null });
  });

  it("re-registering the same email updates one row, keeps the id and first-touch attribution, and never revokes opt-in", async () => {
    const first = await call(registerJson({ ...valid, marketingOptIn: true, utm_source: "google" }));
    const second = await call(registerJson({ ...valid, email: "ada@example.com", platform: "windows", company: "", utm_source: "twitter" }));
    expect(cookieFrom(first)).toBe(cookieFrom(second));

    const rows = await env.DB.prepare("SELECT * FROM leads").all<Record<string, unknown>>();
    expect(rows.results).toHaveLength(1);
    expect(rows.results[0]).toMatchObject({
      platform: "windows",
      company: "Analytical",
      marketing_opt_in: 1,
      utm_source: "google",
      registrations: 2,
    });
  });

  it("answers a filled honeypot like a success but stores nothing", async () => {
    const res = await call(registerJson({ ...valid, whimbrel_hp: "http://spam.example" }));
    expect(res.status).toBe(200);
    expect(res.headers.get("set-cookie")).toBeNull();
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM leads").first("n")).toBe(0);
  });

  it("accepts a plain form post and redirects to the thank-you page", async () => {
    const form = new URLSearchParams({ ...valid, platform: "windows" });
    const res = await call(
      new Request("https://whimbrel.test/api/register", {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: form.toString(),
      }),
    );
    expect(res.status).toBe(303);
    expect(res.headers.get("location")).toBe("/download/thanks/?platform=windows");
    expect(res.headers.get("set-cookie")).toContain(LEAD_COOKIE);
  });

  it("redirects an invalid form post back to the form with an error flag", async () => {
    const res = await call(
      new Request("https://whimbrel.test/api/register", {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: "name=&email=x",
      }),
    );
    expect(res.status).toBe(303);
    expect(res.headers.get("location")).toBe("/download/?error=1#register");
  });

  it("requires a valid Turnstile token when TURNSTILE_SECRET is set", async () => {
    const turnstile = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
    const failing = fakeFetch({ [turnstile]: () => Response.json({ success: false }) });
    const denied = await call(registerJson({ ...valid, "cf-turnstile-response": "bad" }), { env: { TURNSTILE_SECRET: "s" }, fetch: failing });
    expect(denied.status).toBe(403);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM leads").first("n")).toBe(0);

    const passing = fakeFetch({ [turnstile]: () => Response.json({ success: true }) });
    const allowed = await call(registerJson({ ...valid, "cf-turnstile-response": "good" }), { env: { TURNSTILE_SECRET: "s" }, fetch: passing });
    expect(allowed.status).toBe(200);
    expect(passing.calls.map((c) => c.url)).toContain(turnstile);
  });

  it("posts the lead to LEAD_WEBHOOK_URL without the bot-check fields", async () => {
    const hook = "https://hooks.example.com/lead";
    const f = fakeFetch({ [hook]: () => new Response("ok") });
    await call(registerJson({ ...valid, "cf-turnstile-response": "t" }), { env: { LEAD_WEBHOOK_URL: hook }, fetch: f });
    const sent = f.calls.find((c) => c.url === hook);
    expect(sent).toBeDefined();
    const payload = JSON.parse(String(sent!.init!.body));
    expect(payload).toMatchObject({ event: "lead.registered", email: "ada@example.com", platform: "mac" });
    expect(payload).not.toHaveProperty("turnstileToken");
    expect(payload).not.toHaveProperty("honeypot");
  });

  describe("email", () => {
    const RESEND = "https://api.resend.com/emails";
    const emailEnv = {
      RESEND_API_KEY: "re_test",
      EMAIL_FROM: "Whimbrel <whimbrel@futuremeanswered.com>",
      EMAIL_REPLY_TO: "mike@futuremeanswered.com",
      LEAD_ALERT_EMAIL: "mike@futuremeanswered.com",
    };
    const sentEmails = (f: ReturnType<typeof fakeFetch>) =>
      f.calls.filter((c) => c.url === RESEND).map((c) => JSON.parse(String(c.init!.body)) as Record<string, unknown>);

    it("sends a new lead a welcome email with personal download links, and alerts the team", async () => {
      const f = fakeFetch({ [RESEND]: () => Response.json({ id: "e1" }) });
      const res = await call(registerJson(valid), { env: emailEnv, fetch: f });
      const leadId = cookieFrom(res).split("=")[1];
      const [welcome, alert] = sentEmails(f);

      expect(welcome).toMatchObject({ to: ["ada@example.com"], reply_to: "mike@futuremeanswered.com", subject: "Your Whimbrel download links" });
      expect(welcome.text).toContain(`https://whimbrel.test/api/download/mac?lead=${leadId}`);
      expect(welcome.text).toContain(`https://whimbrel.test/api/download/windows?lead=${leadId}`);
      expect(welcome.html).toContain("Hi Ada,");

      expect(alert).toMatchObject({ to: ["mike@futuremeanswered.com"], reply_to: "ada@example.com" });
      expect(alert.subject).toBe("New Whimbrel lead: Ada Lovelace (Analytical), Mac");
      expect(f.calls.find((c) => c.url === RESEND)!.init!.headers).toMatchObject({ authorization: "Bearer re_test" });
    });

    it("alerts on a returning lead but does not send a second welcome email", async () => {
      await call(registerJson(valid), { env: emailEnv, fetch: fakeFetch({ [RESEND]: () => Response.json({}) }) });
      const f = fakeFetch({ [RESEND]: () => Response.json({}) });
      await call(registerJson({ ...valid, platform: "windows" }), { env: emailEnv, fetch: f });
      const sent = sentEmails(f);
      expect(sent).toHaveLength(1);
      expect(sent[0].subject).toBe("Returning Whimbrel lead: Ada Lovelace (Analytical), Windows");
    });

    it("escapes registrant text in HTML email", async () => {
      const f = fakeFetch({ [RESEND]: () => Response.json({}) });
      await call(registerJson({ ...valid, name: "<b>Eve</b> X", company: "<script>" }), { env: emailEnv, fetch: f });
      for (const email of sentEmails(f)) {
        expect(email.html).not.toContain("<b>Eve</b>");
        expect(email.html).not.toContain("<script>");
      }
    });

    it("sends nothing without RESEND_API_KEY, and a Resend failure never fails the registration", async () => {
      const off = fakeFetch();
      await call(registerJson(valid), { env: { ...emailEnv, RESEND_API_KEY: undefined }, fetch: off });
      expect(off.calls.some((c) => c.url === RESEND)).toBe(false);

      const failing = fakeFetch({ [RESEND]: () => new Response("down", { status: 500 }) });
      const res = await call(registerJson({ ...valid, email: "grace@example.com" }), { env: emailEnv, fetch: failing });
      expect(res.status).toBe(200);
    });

    it("sends nothing for a honeypot submission", async () => {
      const f = fakeFetch({ [RESEND]: () => Response.json({}) });
      await call(registerJson({ ...valid, whimbrel_hp: "x" }), { env: emailEnv, fetch: f });
      expect(sentEmails(f)).toHaveLength(0);
    });
  });

  it("rejects other methods", async () => {
    const res = await call(new Request("https://whimbrel.test/api/register"));
    expect(res.status).toBe(405);
  });
});

describe("GET /api/download/:platform", () => {
  it("sends unregistered visitors to the form for that platform", async () => {
    const res = await call(new Request("https://whimbrel.test/api/download/windows"));
    expect(res.status).toBe(303);
    expect(res.headers.get("location")).toBe("/download/?platform=windows#register");
  });

  it("treats a cookie for an unknown lead as unregistered", async () => {
    const res = await call(
      new Request("https://whimbrel.test/api/download/mac", { headers: { cookie: `${LEAD_COOKIE}=00000000-0000-4000-8000-000000000000` } }),
    );
    expect(res.headers.get("location")).toBe("/download/?platform=mac#register");
  });

  it("redirects a registered visitor to the current installer and records the download", async () => {
    const cookie = cookieFrom(await call(registerJson(valid)));
    const mac = await call(new Request("https://whimbrel.test/api/download/mac", { headers: { cookie } }));
    expect(mac.status).toBe(302);
    expect(mac.headers.get("location")).toBe(`${UPDATES}/Whimbrel-0.9.21.zip`);

    const win = await call(new Request("https://whimbrel.test/api/download/windows", { headers: { cookie: `other=1; ${cookie}` } }));
    expect(win.headers.get("location")).toBe(`${UPDATES}/windows/Whimbrel_0.9.21_x64-setup.exe`);

    const downloads = await env.DB.prepare("SELECT platform, version, lead_id IS NOT NULL AS attributed FROM downloads ORDER BY id").all();
    expect(downloads.results).toEqual([
      { platform: "mac", version: "0.9.21", attributed: 1 },
      { platform: "windows", version: "0.9.21", attributed: 1 },
    ]);
  });

  it("accepts the email link's ?lead= in place of the cookie, and sets the cookie", async () => {
    const leadId = cookieFrom(await call(registerJson(valid))).split("=")[1];
    const res = await call(new Request(`https://whimbrel.test/api/download/windows?lead=${leadId}`));
    expect(res.status).toBe(302);
    expect(res.headers.get("location")).toBe(`${UPDATES}/windows/Whimbrel_0.9.21_x64-setup.exe`);
    expect(cookieFrom(res)).toBe(`${LEAD_COOKIE}=${leadId}`);
  });

  it("treats an unknown or malformed ?lead= as unregistered", async () => {
    for (const lead of ["00000000-0000-4000-8000-000000000000", "not-a-uuid"]) {
      const res = await call(new Request(`https://whimbrel.test/api/download/mac?lead=${lead}`));
      expect(res.headers.get("location")).toBe("/download/?platform=mac#register");
      expect(res.headers.get("set-cookie")).toBeNull();
    }
  });

  it("never redirects to a host other than the updates Worker", async () => {
    const cookie = cookieFrom(await call(registerJson(valid)));
    const evil = fakeFetch({ [`${UPDATES}/version.json`]: () => Response.json({ version: "6.6.6", url: "https://evil.example/Whimbrel.zip" }) });
    const res = await call(new Request("https://whimbrel.test/api/download/mac", { headers: { cookie } }), { fetch: evil });
    expect(res.headers.get("location")).toBe("/download/unavailable/?platform=mac");
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM downloads").first("n")).toBe(0);
  });

  it("accepts an installer on the updates Worker's workers.dev address when UPDATES_HOSTS lists it", async () => {
    const cookie = cookieFrom(await call(registerJson(valid)));
    const custom = "https://kestrel-updates-worker.mpointer.workers.dev/Whimbrel-1.0.0.zip";
    const f = () => fakeFetch({ [`${UPDATES}/version.json`]: () => Response.json({ version: "1.0.0", url: custom }) });
    const allowed = await call(new Request("https://whimbrel.test/api/download/mac", { headers: { cookie } }), {
      env: { UPDATES_HOSTS: "kestrel-updates-worker.mpointer.workers.dev" },
      fetch: f(),
    });
    expect(allowed.headers.get("location")).toBe(custom);
    const refused = await call(new Request("https://whimbrel.test/api/download/mac", { headers: { cookie } }), { fetch: f() });
    expect(refused.headers.get("location")).toBe("/download/unavailable/?platform=mac");
  });

  it("shows the unavailable page when nothing is published", async () => {
    const cookie = cookieFrom(await call(registerJson(valid)));
    const empty = fakeFetch({ [`${UPDATES}/windows/latest.json`]: () => new Response("Not Found", { status: 404 }) });
    const res = await call(new Request("https://whimbrel.test/api/download/windows", { headers: { cookie } }), { fetch: empty });
    expect(res.headers.get("location")).toBe("/download/unavailable/?platform=windows");
  });
});

describe("canonical host", () => {
  const hosts = { CANONICAL_HOST: "whimbrelapp.com", REDIRECT_HOSTS: "www.whimbrelapp.com, kestrel-website.mpointer.workers.dev" };

  it("301s a listed host to the same path and query on the canonical host", async () => {
    const res = await call(new Request("https://www.whimbrelapp.com/pricing/?utm_source=x#faq"), { env: hosts });
    expect(res.status).toBe(301);
    expect(res.headers.get("location")).toBe("https://whimbrelapp.com/pricing/?utm_source=x");
    const old = await call(new Request("https://kestrel-website.mpointer.workers.dev/download/mac/"), { env: hosts });
    expect(old.headers.get("location")).toBe("https://whimbrelapp.com/download/mac/");
  });

  it("uses 308 for a form post so the method survives", async () => {
    const res = await call(new Request("https://www.whimbrelapp.com/api/register", { method: "POST", body: "x" }), { env: hosts });
    expect(res.status).toBe(308);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM leads").first("n")).toBe(0);
  });

  it("serves the canonical host, unlisted hosts, and everything when CANONICAL_HOST is unset", async () => {
    for (const [url, extra] of [
      ["https://whimbrelapp.com/", hosts],
      ["https://kestrel-website.mpointer.workers.dev/", { CANONICAL_HOST: "whimbrelapp.com", REDIRECT_HOSTS: "www.whimbrelapp.com" }],
      ["https://www.whimbrelapp.com/", {}],
    ] as const) {
      const res = await call(new Request(url), { env: extra });
      expect(res.status, url).toBe(200);
    }
  });
});

describe("other routes", () => {
  it("GET /api/releases reports versions without download URLs", async () => {
    const res = await call(new Request("https://whimbrel.test/api/releases"));
    expect(await res.json()).toEqual({ mac: { version: "0.9.21" }, windows: { version: "0.9.21" } });
  });

  it("unknown /api/ paths are JSON 404s", async () => {
    const res = await call(new Request("https://whimbrel.test/api/nope"));
    expect(res.status).toBe(404);
  });

  it("serves the built home page from the asset store", async () => {
    const res = await call(new Request("https://whimbrel.test/"));
    expect(res.status).toBe(200);
    const html = await res.text();
    expect(html).toContain("<h1>The AI assistant that");
    expect(html).toContain('"@type":"FAQPage"');
  });
});
