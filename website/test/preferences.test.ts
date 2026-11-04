// Email preferences, the two kinds of mail, the backfill and the next-day follow-up
// (docs/superpowers/specs/2026-11-04-lead-email-preferences-design.md).
import { beforeEach, describe, expect, it } from "vitest";
import { createExecutionContext, env, waitOnExecutionContext } from "cloudflare:test";
import { handle, type Env } from "../src/app";
import { send, type Email } from "../src/email";
import { runScheduled } from "../src/followup";
import { leadByEmail, signToken, syncContact, TOKEN_DAYS } from "../src/preferences";

const RESEND = "https://api.resend.com/emails";
const SECRET = "test-token-secret";
const ORIGIN = "https://whimbrel.test";
const mailEnv = {
  RESEND_API_KEY: "re_test",
  EMAIL_FROM: "Whimbrel <whimbrel@futuremeanswered.com>",
  EMAIL_REPLY_TO: "mike@futuremeanswered.com",
  EMAIL_TOKEN_SECRET: SECRET,
};

function recorder(resendOk = true) {
  const sent: Record<string, unknown>[] = [];
  const fn = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input instanceof Request ? input.url : input);
    if (url === RESEND) {
      sent.push(JSON.parse(String(init!.body)));
      return resendOk ? Response.json({ id: "e" }) : new Response("nope", { status: 500 });
    }
    return new Response("not found", { status: 404 });
  }) as typeof fetch;
  return { fn, sent };
}

async function call(request: Request, extra: Partial<Env> = {}, fetcher = recorder().fn) {
  const ctx = createExecutionContext();
  const res = await handle(request, { ASSETS: env.ASSETS, DB: env.DB, UPDATES_URL: "https://updates.test", ...extra }, ctx, fetcher);
  await waitOnExecutionContext(ctx);
  return res;
}

async function register(body: Record<string, unknown>, extra: Partial<Env> = {}, fetcher = recorder().fn) {
  return call(
    new Request(`${ORIGIN}/api/register`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
    extra,
    fetcher,
  );
}

const ada = { name: "Ada Lovelace", email: "ada@example.com", platform: "mac" };
const grace = { name: "Grace Hopper", email: "grace@example.com", platform: "windows" };
const lead = async (email: string) => (await leadByEmail(env.DB, email))!;
const events = async (leadId: string) =>
  (await env.DB.prepare("SELECT value, source FROM email_preference_events WHERE lead_id = ?1 ORDER BY id").bind(leadId).all()).results;
const prefsPost = (fields: Record<string, string>) =>
  call(new Request(`${ORIGIN}/email/preferences/`, { method: "POST", body: new URLSearchParams(fields) }), mailEnv);
async function optInViaLink(email: string) {
  const l = await lead(email);
  await prefsPost({ t: await signToken(SECRET, l.id, Date.now()), value: "opted_in" });
}
const mail = (to: string): Email => ({ from: mailEnv.EMAIL_FROM, to, subject: "News", text: "Hello", html: "<!doctype html><html><body><div>Hello</div></body></html>" });

beforeEach(async () => {
  await env.DB.exec("DELETE FROM downloads; DELETE FROM email_preference_events; DELETE FROM leads;");
});

describe("signup (double opt-in)", () => {
  it("stores where and in which language the form was used; everyone starts opted_out", async () => {
    await register({ ...ada, signup_page: "/es/download/", signup_language: "es" });
    await register({ ...grace, marketingOptIn: true });
    const a = await lead("ada@example.com");
    expect(a).toMatchObject({ product_emails: "opted_out", product_emails_source: "signup_form", signup_page: "/es/download/", signup_language: "es" });
    expect((await lead("grace@example.com")).product_emails).toBe("opted_out");
    expect(await events(a.id)).toEqual([{ value: "opted_out", source: "signup_form" }]);
  });

  it("a ticked box mails a confirmation link, at most once a day, and confirming turns product mail on", async () => {
    const r = recorder();
    await register({ ...grace, marketingOptIn: true, signup_language: "fr" }, mailEnv, r.fn);
    await register({ ...grace, marketingOptIn: true }, mailEnv, r.fn); // again within the day: no second mail
    const confirms = r.sent.filter((e) => String(e.subject).startsWith("Confirmez"));
    expect(confirms).toHaveLength(1);
    const link = String(confirms[0].text).match(/https:\/\/whimbrel\.test\/email\/preferences\/\?t=[^\s]+&confirm=1/)![0];
    const g = await lead("grace@example.com");
    expect(g.product_emails).toBe("opted_out");
    expect(await (await call(new Request(link), mailEnv)).text()).toContain("Confirmez ci-dessous");
    const t = decodeURIComponent(new URL(link).searchParams.get("t")!);
    await prefsPost({ t, value: "opted_in" });
    expect((await lead(g.email)).product_emails).toBe("opted_in");
    expect((await events(g.id)).at(-1)).toEqual({ value: "opted_in", source: "preferences_page" });
  });

  it("a re-registration never changes the answer, ticked or not", async () => {
    await register(ada);
    await optInViaLink("ada@example.com");
    await register(ada);
    expect((await lead("ada@example.com")).product_emails).toBe("opted_in");
  });
});

describe("a choice made through a link", () => {
  it("is never overridden by someone submitting the form with that address", async () => {
    await register(ada);
    await optInViaLink("ada@example.com");
    const a = await lead("ada@example.com");
    await prefsPost({ t: await signToken(SECRET, a.id, Date.now()), value: "opted_out" });
    await register({ ...ada, marketingOptIn: true }, mailEnv); // anyone can submit the form with this address
    expect((await lead(a.email)).product_emails).toBe("opted_out");
  });
});

describe("two kinds of mail", () => {
  it("an opted_out lead gets transactional mail and is left out of product mail", async () => {
    await register(ada);
    const a = await lead("ada@example.com");
    const r = recorder();
    const opts = { leadId: a.id, origin: ORIGIN, nowMs: Date.now() };
    expect(await send({ DB: env.DB, ...mailEnv }, "transactional", mail(a.email), opts, r.fn)).toEqual({ sent: true });
    expect(await send({ DB: env.DB, ...mailEnv }, "product", mail(a.email), opts, r.fn)).toEqual({ sent: false, refused: "opted_out" });
    expect(await send({ DB: env.DB, ...mailEnv }, "product", mail("stranger@example.com"), { origin: ORIGIN, nowMs: Date.now() }, r.fn)).toEqual({ sent: false, refused: "not_a_lead" });
    expect(r.sent).toHaveLength(1);
    expect(r.sent[0].text).toContain(`${ORIGIN}/email/preferences/?t=`);
    expect(r.sent[0].headers).toBeUndefined();
  });

  it("an opted_in lead gets product mail with one-click unsubscribe", async () => {
    await register(ada);
    await optInViaLink("ada@example.com");
    const a = await lead("ada@example.com");
    const r = recorder();
    expect(await send({ DB: env.DB, ...mailEnv }, "product", mail(a.email), { origin: ORIGIN, nowMs: Date.now() }, r.fn)).toEqual({ sent: true });
    const headers = r.sent[0].headers as Record<string, string>;
    expect(headers["List-Unsubscribe"]).toMatch(new RegExp(`^<${ORIGIN}/api/email/unsubscribe\\?t=.+>$`));
    expect(headers["List-Unsubscribe-Post"]).toBe("List-Unsubscribe=One-Click");
    expect(String(r.sent[0].html)).toContain("Manage email preferences");
  });

  it("the download links mail carries the preferences footer and changes nothing", async () => {
    const r = recorder();
    await register(ada, mailEnv, r.fn);
    const welcome = r.sent.find((e) => (e.to as string[])[0] === "ada@example.com")!;
    expect(welcome.subject).toBe("Your Whimbrel download links");
    expect(String(welcome.html)).toContain("/email/preferences/?t=");
    expect((await lead("ada@example.com")).product_emails).toBe("opted_out");
  });
});

describe("the preferences page", () => {
  const post = (fields: Record<string, string>) =>
    call(new Request(`${ORIGIN}/email/preferences/`, { method: "POST", body: new URLSearchParams(fields) }), mailEnv);

  it("shows the answer and switches both ways, logging each change", async () => {
    await register(ada);
    const a = await lead("ada@example.com");
    const t = await signToken(SECRET, a.id, Date.now());
    const page = await (await call(new Request(`${ORIGIN}/email/preferences/?t=${encodeURIComponent(t)}`), mailEnv)).text();
    expect(page).toContain("Product emails are off");
    expect(await (await post({ t, value: "opted_in" })).text()).toContain("Product emails are on");
    expect((await lead(a.email)).product_emails).toBe("opted_in");
    await post({ t, value: "opted_out" });
    expect((await lead(a.email)).product_emails).toBe("opted_out");
    expect(await events(a.id)).toEqual([
      { value: "opted_out", source: "signup_form" },
      { value: "opted_in", source: "preferences_page" },
      { value: "opted_out", source: "preferences_page" },
    ]);
  });

  it("one person's link can't change another's setting, and altered or expired links are refused", async () => {
    await register(ada);
    await register(grace);
    const a = await lead("ada@example.com");
    const g = await lead("grace@example.com");
    const t = await signToken(SECRET, a.id, Date.now());
    const forged = t.replace(a.id, g.id);
    expect(await (await post({ t: forged, value: "opted_in" })).text()).toContain("expired");
    const expired = await signToken(SECRET, a.id, Date.now() - (TOKEN_DAYS + 1) * 86400_000);
    expect(await (await post({ t: expired, value: "opted_in" })).text()).toContain("expired");
    const otherSecret = await signToken("another-secret", g.id, Date.now());
    expect(await (await post({ t: otherSecret, value: "opted_in" })).text()).toContain("expired");
    expect((await lead(a.email)).product_emails).toBe("opted_out");
    expect((await lead(g.email)).product_emails).toBe("opted_out");
  });

  it("one-click unsubscribe turns product mail off", async () => {
    await register(ada);
    await optInViaLink("ada@example.com");
    const a = await lead("ada@example.com");
    const t = await signToken(SECRET, a.id, Date.now());
    const res = await call(new Request(`${ORIGIN}/api/email/unsubscribe?t=${encodeURIComponent(t)}`, { method: "POST", body: "List-Unsubscribe=One-Click" }), mailEnv);
    expect(res.status).toBe(200);
    expect((await lead(a.email)).product_emails).toBe("opted_out");
    expect((await events(a.id)).at(-1)).toEqual({ value: "opted_out", source: "one_click_unsubscribe" });
  });
});

describe("the backfill", () => {
  it("keeps every existing answer, marks existing leads followed up, and logs one event each", async () => {
    const insert = (id: string, email: string, optIn: number, landing: string | null) =>
      env.DB.prepare(
        `INSERT INTO leads (id, email, name, platform, marketing_opt_in, landing_page, created_at, updated_at, product_emails, product_emails_source)
         VALUES (?1, ?2, 'X', 'mac', ?3, ?4, '2026-10-01T00:00:00.000Z', '2026-10-01T00:00:00.000Z', 'opted_out', NULL)`,
      )
        .bind(id, email, optIn, landing)
        .run();
    await insert("11111111-1111-4111-8111-111111111111", "yes@example.com", 1, "/fr/features/routines/");
    await insert("22222222-2222-4222-8222-222222222222", "no@example.com", 0, "/es/");
    await insert("33333333-3333-4333-8333-333333333333", "plain@example.com", 0, null);
    const backfill = env.TEST_MIGRATIONS.find((m) => m.name.startsWith("0002"))!.queries.map((q) => q.replace(/^\s*--.*$/gm, "").trim()).filter((q) => /^(UPDATE|INSERT)/i.test(q));
    expect(backfill).toHaveLength(2);
    for (const q of backfill) await env.DB.prepare(q).run();
    for (const q of backfill) await env.DB.prepare(q).run(); // idempotent

    const rows = (await env.DB.prepare("SELECT email, product_emails, signup_language, followup_sent_at FROM leads ORDER BY email").all()).results;
    expect(rows.map((r) => [r.email, r.product_emails, r.signup_language])).toEqual([
      ["no@example.com", "opted_out", "es"],
      ["plain@example.com", "opted_out", "en"],
      ["yes@example.com", "opted_in", "fr"],
    ]);
    expect(rows.every((r) => r.followup_sent_at)).toBe(true);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM email_preference_events WHERE source = 'backfill'").first("n")).toBe(3);
  });
});

describe("the next-day follow-up", () => {
  const at = (h: number) => new Date(Date.parse("2026-11-05T12:00:00.000Z") - h * 3600_000).toISOString();
  const now = Date.parse("2026-11-05T12:00:00.000Z");

  it("goes once, in the signup language, only 20 to 72 hours after signup, and retries a failed send", async () => {
    await register({ ...ada, signup_language: "es" });
    await register(grace);
    await register({ name: "New Person", email: "new@example.com", platform: "mac" });
    await env.DB.prepare("UPDATE leads SET created_at = ?2 WHERE email = ?1").bind("ada@example.com", at(24)).run();
    await env.DB.prepare("UPDATE leads SET created_at = ?2 WHERE email = ?1").bind("grace@example.com", at(100)).run();
    await env.DB.prepare("UPDATE leads SET created_at = ?2 WHERE email = ?1").bind("new@example.com", at(2)).run();

    const failing = recorder(false);
    expect(await runScheduled({ DB: env.DB, ...mailEnv }, now, failing.fn)).toMatchObject({ followups: 0 });
    expect((await lead("ada@example.com")).followup_sent_at).toBeNull();

    const r = recorder();
    expect(await runScheduled({ DB: env.DB, ...mailEnv }, now, r.fn)).toMatchObject({ followups: 1 });
    expect(r.sent).toHaveLength(1);
    expect(r.sent[0]).toMatchObject({ to: ["ada@example.com"], subject: "Whimbrel: gracias por probarlo", reply_to: "mike@futuremeanswered.com" });
    expect(String(r.sent[0].text)).toContain("No recibirás correos de producto");
    expect(String(r.sent[0].html)).toContain("Gestionar preferencias de correo");
    expect(r.sent[0].headers).toBeUndefined();

    const again = recorder();
    await runScheduled({ DB: env.DB, ...mailEnv }, now + 3600_000, again.fn);
    expect(again.sent).toHaveLength(0);
  });
});

describe("the Resend contact", () => {
  it("sends the topic list as a bare array, and creates the contact when Resend has none", async () => {
    const calls: { url: string; method: string; body: unknown }[] = [];
    const fn = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      calls.push({ url, method: init!.method!, body: JSON.parse(String(init!.body)) });
      return url.endsWith("/topics") ? new Response("{}", { status: 404 }) : Response.json({ id: "c1" });
    }) as typeof fetch;
    expect(await syncContact("re_test", "topic-1", { email: "ada@example.com", name: "Ada Lovelace", product_emails: "opted_in" }, fn)).toBe(true);
    expect(calls[0]).toEqual({ url: "https://api.resend.com/contacts/ada%40example.com/topics", method: "PATCH", body: [{ id: "topic-1", subscription: "opt_in" }] });
    expect(calls[1]).toEqual({ url: "https://api.resend.com/contacts", method: "POST", body: { email: "ada@example.com", first_name: "Ada", topics: [{ id: "topic-1", subscription: "opt_in" }] } });
  });
});
