import { handleAdminLeads } from "./admin";
import { leadExists, parseLead, PLATFORMS, recordDownload, upsertLead, type Contact, type LeadInput, type Platform } from "./leads";
import { handleBeta, handleReview } from "./beta";
import { resolveRelease } from "./downloads";
import { leadAlertEmail, send, welcomeEmail, type Email } from "./email";
import { preferencesPage } from "./preferences-page";
import { language, leadByEmail, leadById, logEvent, markSynced, setPreference, signToken, syncContact, verifyToken, type ProductEmails } from "./preferences";

export interface Env {
  ASSETS: Fetcher;
  DB: D1Database;
  UPDATES_URL: string;
  /** Other hostnames of the updates Worker that release manifests may name, comma-separated. */
  UPDATES_HOSTS?: string;
  /** The site's one public hostname (whimbrelapp.com). */
  CANONICAL_HOST?: string;
  /** Hostnames that 301 to CANONICAL_HOST, comma-separated (www., and the workers.dev
   * address once the custom domain is confirmed live). */
  REDIRECT_HOSTS?: string;
  TURNSTILE_SECRET?: string;
  LEAD_WEBHOOK_URL?: string;
  /** Resend API key (secret). Without it no email is sent. */
  RESEND_API_KEY?: string;
  /** Sender for all email, on a Resend-verified domain, e.g. "Whimbrel <whimbrel@futuremeanswered.com>". */
  EMAIL_FROM?: string;
  /** Where replies to the welcome email go. */
  EMAIL_REPLY_TO?: string;
  /** Who gets an alert for every registration. */
  LEAD_ALERT_EMAIL?: string;
  /** Signs the "Manage email preferences" links (secret). Without it mail carries no
   * footer link and product mail is refused (it could not carry one-click unsubscribe). */
  EMAIL_TOKEN_SECRET?: string;
  /** The Resend topic ("Product updates") that mirrors each lead's answer for broadcasts. */
  RESEND_PRODUCT_TOPIC_ID?: string;
  /** /admin/leads: the Cloudflare Access team domain, its application's AUD tag, and who may
   * see it (src/admin.ts). The page is 404 until the first two are set. */
  ADMIN_ACCESS_TEAM?: string;
  ADMIN_ACCESS_AUD?: string;
  ADMIN_EMAILS?: string;
  ADMIN_EMAILS_EXTRA?: string;
  /** Signs the Approve and Decline links in the mobile beta alert (secret). Without it,
   * sign-ups are still saved but the alert has no review links. */
  BETA_REVIEW_SECRET?: string;
  /** The TestFlight public link for the iPhone beta. Empty: the install email says it follows. */
  TESTFLIGHT_PUBLIC_URL?: string;
  /** Where Android testers download the APK. */
  ANDROID_RELEASE_URL?: string;
}

/** Cookie that remembers a registered visitor so the download links work without asking
 * again. It holds only the lead's random id. This is a soft gate: the installers are
 * public on the updates Worker (the auto-updaters need them to be), so the gate exists to
 * capture the lead and attribute the download, not to protect the file. */
export const LEAD_COOKIE = "whimbrel_lead";
const LEAD_COOKIE_MAX_AGE = 60 * 60 * 24 * 365;
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

function readLeadCookie(request: Request): string | null {
  const header = request.headers.get("cookie") ?? "";
  for (const part of header.split(";")) {
    const [key, ...rest] = part.trim().split("=");
    if (key === LEAD_COOKIE) {
      const value = rest.join("=");
      return UUID_RE.test(value) ? value : null;
    }
  }
  return null;
}

function leadCookie(id: string): string {
  return `${LEAD_COOKIE}=${id}; Path=/; Max-Age=${LEAD_COOKIE_MAX_AGE}; HttpOnly; Secure; SameSite=Lax`;
}

export function country(request: Request): string | null {
  const cf = (request as Request & { cf?: { country?: unknown } }).cf;
  return typeof cf?.country === "string" ? cf.country : null;
}

function list(value: string | undefined): string[] {
  return (value ?? "").split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
}

/** Sends a request for one of REDIRECT_HOSTS to the same path on CANONICAL_HOST: 301 for
 * reads, 308 for anything else so a form post from a cached page keeps its method. */
function canonicalRedirect(request: Request, env: Env): Response | null {
  if (!env.CANONICAL_HOST) return null;
  const url = new URL(request.url);
  if (url.hostname === env.CANONICAL_HOST || !list(env.REDIRECT_HOSTS).includes(url.hostname)) return null;
  url.protocol = "https:";
  url.hostname = env.CANONICAL_HOST;
  url.port = "";
  url.hash = ""; // browsers carry the fragment across the redirect themselves
  const status = request.method === "GET" || request.method === "HEAD" ? 301 : 308;
  return new Response(null, { status, headers: { location: url.toString() } });
}

export function redirect(location: string, headers: Record<string, string> = {}): Response {
  return new Response(null, { status: 303, headers: { location, "cache-control": "no-store", ...headers } });
}

export async function readBody(request: Request): Promise<{ body: Record<string, unknown>; isJson: boolean } | null> {
  const type = request.headers.get("content-type") ?? "";
  try {
    if (type.includes("application/json")) {
      const json = await request.json();
      return json && typeof json === "object" ? { body: json as Record<string, unknown>, isJson: true } : null;
    }
    if (type.includes("application/x-www-form-urlencoded") || type.includes("multipart/form-data")) {
      const form = await request.formData();
      const body: Record<string, unknown> = {};
      for (const [k, v] of form.entries()) if (typeof v === "string") body[k] = v;
      return { body, isJson: false };
    }
  } catch {
    return null;
  }
  return null;
}

async function verifyTurnstile(secret: string, token: string | null, ip: string | null, fetcher: typeof fetch): Promise<boolean> {
  if (!token) return false;
  const form = new FormData();
  form.append("secret", secret);
  form.append("response", token);
  if (ip) form.append("remoteip", ip);
  try {
    const res = await fetcher("https://challenges.cloudflare.com/turnstile/v0/siteverify", { method: "POST", body: form });
    const outcome = (await res.json()) as { success?: boolean };
    return outcome.success === true;
  } catch {
    return false;
  }
}

function webhookPayload(id: string, lead: LeadInput, countryCode: string | null, at: string) {
  const { turnstileToken: _t, honeypot: _h, ...profile } = lead;
  return { event: "lead.registered", leadId: id, country: countryCode, at, ...profile };
}

/** The spam guard every sign-up form shares: "bot" when the hidden honeypot field is
 * filled (answer as a success, store nothing), "refused" when Turnstile is on and fails. */
export async function spamCheck(request: Request, env: Env, contact: Contact, fetcher: typeof fetch): Promise<"ok" | "bot" | "refused"> {
  if (contact.honeypot) return "bot";
  if (env.TURNSTILE_SECRET) {
    const ok = await verifyTurnstile(env.TURNSTILE_SECRET, contact.turnstileToken, request.headers.get("cf-connecting-ip"), fetcher);
    if (!ok) return "refused";
  }
  return "ok";
}

/** Upserts the lead and starts its product-email answer. Every sign-up form goes through here. */
export async function saveLead(
  request: Request,
  env: Env,
  ctx: ExecutionContext,
  lead: Contact & { platform: Platform | null },
  fetcher: typeof fetch,
): Promise<{ id: string; isNew: boolean; now: string; countryCode: string | null }> {
  const now = new Date().toISOString();
  const countryCode = country(request);
  const { id, isNew } = await upsertLead(env.DB, lead, countryCode, now);

  // The product-email answer (design §1, double opt-in): every lead starts opted_out. The
  // form needs no proof of owning the address, so a ticked box only mails a confirmation
  // link; product mail starts when the owner of the address confirms through it.
  const where = { page: lead.signupPage, language: lead.signupLanguage };
  if (isNew) await logEvent(env.DB, id, "opted_out", "signup_form", where, now);
  ctx.waitUntil(syncLead(env, id, fetcher));
  if (lead.marketingOptIn) ctx.waitUntil(requestConfirmation(env, id, new URL(request.url).origin, Date.parse(now), fetcher));
  return { id, isNew, now, countryCode };
}

/** POST /api/register — accepts JSON (the page's script) or a plain form post (no-JS
 * fallback). JSON callers get JSON back; form posts are redirected to the thank-you page
 * or back to the form with an error flag. */
export async function handleRegister(request: Request, env: Env, ctx: ExecutionContext, fetcher: typeof fetch = fetch): Promise<Response> {
  const parsedBody = await readBody(request);
  if (!parsedBody) return Response.json({ ok: false, errors: { form: "Unreadable request." } }, { status: 400 });
  const { body, isJson } = parsedBody;

  const fail = (status: number, errors: Record<string, string>) =>
    isJson ? Response.json({ ok: false, errors }, { status }) : redirect("/download/?error=1#register");

  const result = parseLead(body);
  if (!result.ok) return fail(400, result.errors);
  const lead = result.lead;
  const thanks = `/download/thanks/?platform=${lead.platform}`;

  // A bot filled the hidden field: answer exactly as a success would, store nothing.
  const spam = await spamCheck(request, env, lead, fetcher);
  if (spam === "bot") return isJson ? Response.json({ ok: true, next: thanks }) : redirect(thanks);
  if (spam === "refused") return fail(403, { form: "Verification failed. Refresh the page and try again." });

  const { id, isNew, now, countryCode } = await saveLead(request, env, ctx, lead, fetcher);

  // Email never blocks or fails the registration: it runs after the response is sent.
  if (env.EMAIL_FROM) {
    const from = env.EMAIL_FROM;
    const origin = new URL(request.url).origin;
    const nowMs = Date.parse(now);
    if (isNew) ctx.waitUntil(send(env, "transactional", welcomeEmail(origin, id, lead, from, env.EMAIL_REPLY_TO), { leadId: id, origin, nowMs }, fetcher));
    if (env.LEAD_ALERT_EMAIL) ctx.waitUntil(send(env, "transactional", leadAlertEmail(lead, isNew, countryCode, from, env.LEAD_ALERT_EMAIL), { origin, nowMs }, fetcher));
  }

  if (env.LEAD_WEBHOOK_URL) {
    const url = env.LEAD_WEBHOOK_URL;
    ctx.waitUntil(
      fetcher(url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(webhookPayload(id, lead, countryCode, now)),
      }).catch(() => undefined),
    );
  }

  const cookie = { "set-cookie": leadCookie(id) };
  return isJson
    ? Response.json({ ok: true, next: thanks }, { headers: { ...cookie, "cache-control": "no-store" } })
    : redirect(thanks, cookie);
}

/** GET /api/download/{mac|windows} — registered visitors are redirected to the current
 * installer and the download is recorded; everyone else is sent to the form first. The
 * welcome email's links carry the lead id as `?lead=` instead of the cookie; a valid one
 * also sets the cookie, so the site remembers that browser too. */
export async function handleDownload(request: Request, env: Env, platform: Platform, fetcher: typeof fetch = fetch): Promise<Response> {
  const fromLink = new URL(request.url).searchParams.get("lead");
  const linkId = fromLink && UUID_RE.test(fromLink) ? fromLink : null;
  const leadId = linkId ?? readLeadCookie(request);
  if (!leadId || !(await leadExists(env.DB, leadId))) {
    return redirect(`/download/?platform=${platform}#register`);
  }
  const release = await resolveRelease(platform, env.UPDATES_URL, fetcher, list(env.UPDATES_HOSTS));
  if (!release) return redirect(`/download/unavailable/?platform=${platform}`);
  await recordDownload(env.DB, leadId, platform, release.version, country(request), new Date().toISOString());
  const headers: Record<string, string> = { location: release.url, "cache-control": "no-store" };
  if (linkId) headers["set-cookie"] = leadCookie(linkId);
  return new Response(null, { status: 302, headers });
}

/** GET /api/releases — current version numbers, for the "Version x.y.z" labels on the
 * download page. URLs are deliberately not included; downloads go through the gate. */
export async function handleReleases(env: Env, fetcher: typeof fetch = fetch): Promise<Response> {
  const [mac, windows] = await Promise.all(PLATFORMS.map((p) => resolveRelease(p, env.UPDATES_URL, fetcher, list(env.UPDATES_HOSTS))));
  return Response.json(
    { mac: mac ? { version: mac.version } : null, windows: windows ? { version: windows.version } : null },
    { headers: { "cache-control": "public, max-age=300" } },
  );
}

export async function handle(request: Request, env: Env, ctx: ExecutionContext, fetcher: typeof fetch = fetch): Promise<Response> {
  const moved = canonicalRedirect(request, env);
  if (moved) return moved;

  const { pathname } = new URL(request.url);

  if (pathname === "/api/beta") {
    if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
    return handleBeta(request, env, ctx, fetcher);
  }

  if (pathname === "/beta/review" || pathname === "/beta/review/") {
    if (request.method !== "GET" && request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET, POST" } });
    return handleReview(request, env, fetcher);
  }

  if (pathname === "/admin/leads" || pathname === "/admin/leads/" || pathname === "/admin/leads.csv") {
    return handleAdminLeads(request, env, fetcher);
  }

  if (pathname === "/api/register") {
    if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
    return handleRegister(request, env, ctx, fetcher);
  }

  const download = pathname.match(/^\/api\/download\/(mac|windows)\/?$/);
  if (download) {
    if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
    return handleDownload(request, env, download[1] as Platform, fetcher);
  }

  if (pathname === "/api/releases" && request.method === "GET") return handleReleases(env, fetcher);

  if (pathname === "/email/preferences/" || pathname === "/email/preferences") {
    if (request.method === "GET") return handlePreferencesGet(request, env);
    if (request.method === "POST") return handlePreferencesPost(request, env, ctx, fetcher);
    return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET, POST" } });
  }

  if (pathname === "/api/email/unsubscribe") {
    // RFC 8058 one-click: mail clients POST. A person who opens the link gets the page.
    if (request.method === "POST") return handleOneClickUnsubscribe(request, env, ctx, fetcher);
    const t = new URL(request.url).searchParams.get("t") ?? "";
    return redirect(`/email/preferences/?t=${encodeURIComponent(t)}`);
  }

  if (pathname.startsWith("/api/")) return Response.json({ error: "not found" }, { status: 404 });
  return env.ASSETS.fetch(request);
}

// ---- email preferences (design §3) --------------------------------------------------------

const CONFIRM_EVERY_MS = 24 * 3600_000;

/** Double opt-in: mails the lead a link to confirm product emails, at most once a day, so
 * the form can't be used to flood an address. Nothing changes until they confirm. */
async function requestConfirmation(env: Env, leadId: string, origin: string, nowMs: number, fetcher: typeof fetch): Promise<void> {
  if (!env.EMAIL_FROM || !env.EMAIL_TOKEN_SECRET) return;
  const lead = await leadById(env.DB, leadId);
  if (!lead || lead.product_emails === "opted_in") return;
  const last = lead.product_emails_confirm_sent_at ? Date.parse(lead.product_emails_confirm_sent_at) : 0;
  if (nowMs - last < CONFIRM_EVERY_MS) return;
  const token = await signToken(env.EMAIL_TOKEN_SECRET, lead.id, nowMs);
  const link = `${origin}/email/preferences/?t=${encodeURIComponent(token)}&confirm=1`;
  const result = await send(env, "transactional", confirmEmail(lead.email, language(lead.signup_language), link, env.EMAIL_FROM, env.EMAIL_REPLY_TO), { leadId: lead.id, origin, nowMs }, fetcher);
  if (result.sent) await env.DB.prepare("UPDATE leads SET product_emails_confirm_sent_at = ?2 WHERE id = ?1").bind(lead.id, new Date(nowMs).toISOString()).run();
}

function confirmEmail(to: string, lang: "en" | "es" | "fr", link: string, from: string, replyTo: string | undefined): Email {
  const c = {
    en: {
      subject: "Confirm Whimbrel product news",
      body: "You asked for Whimbrel release notes and product news when you registered. To make sure it was you, please confirm.",
      button: "Yes, send me product news",
      ignore: "If it wasn't you, ignore this email and you won't get any.",
    },
    es: {
      subject: "Confirma las novedades de Whimbrel",
      body: "Pediste recibir las notas de versión y las novedades de Whimbrel al registrarte. Para asegurarnos de que fuiste tú, confírmalo.",
      button: "Sí, envíenme las novedades",
      ignore: "Si no fuiste tú, ignora este correo y no recibirás ninguna.",
    },
    fr: {
      subject: "Confirmez les nouvelles de Whimbrel",
      body: "Vous avez demandé à recevoir les notes de version et les nouvelles de Whimbrel lors de votre inscription. Pour vérifier que c'était bien vous, veuillez confirmer.",
      button: "Oui, envoyez-moi les nouvelles",
      ignore: "Si ce n'était pas vous, ignorez ce courriel et vous n'en recevrez aucune.",
    },
  }[lang];
  const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const html = `<!doctype html><html><body style="margin:0;background:#F7F2E7;font-family:-apple-system,'Segoe UI',sans-serif;color:#241E19">
<div style="max-width:560px;margin:0 auto;padding:32px 20px">
<p style="font-size:16px;line-height:1.5">${esc(c.body)}</p>
<p style="margin:24px 0"><a href="${esc(link)}" style="display:inline-block;padding:12px 20px;border-radius:10px;text-decoration:none;font-weight:600;background:#A8502E;color:#ffffff">${esc(c.button)}</a></p>
<p style="font-size:14px;line-height:1.5;color:#6B5F4F">${esc(c.ignore)}</p>
</div></body></html>`;
  return { from, to, replyTo, subject: c.subject, text: `${c.body}\n\n${c.button}: ${link}\n\n${c.ignore}`, html };
}

/** Mirrors one lead's answer to its Resend contact now; the cron retries a failure. */
async function syncLead(env: Env, leadId: string, fetcher: typeof fetch): Promise<void> {
  if (!env.RESEND_API_KEY || !env.RESEND_PRODUCT_TOPIC_ID) return;
  const lead = await leadById(env.DB, leadId);
  if (lead && (await syncContact(env.RESEND_API_KEY, env.RESEND_PRODUCT_TOPIC_ID, lead, fetcher))) {
    await markSynced(env.DB, leadId, new Date().toISOString());
  }
}

async function leadFromToken(env: Env, token: string | null) {
  if (!env.EMAIL_TOKEN_SECRET) return null;
  const id = await verifyToken(env.EMAIL_TOKEN_SECRET, token, Date.now());
  return id ? leadById(env.DB, id) : null;
}

async function handlePreferencesGet(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const token = url.searchParams.get("t");
  const lead = await leadFromToken(env, token);
  if (!lead || !token) return preferencesPage(language(url.searchParams.get("lang")), { kind: "expired" });
  return preferencesPage(language(lead.signup_language), { kind: "show", value: lead.product_emails, token, saved: false, confirm: url.searchParams.get("confirm") === "1" });
}

async function handlePreferencesPost(request: Request, env: Env, ctx: ExecutionContext, fetcher: typeof fetch): Promise<Response> {
  const form = await request.formData().catch(() => null);
  const token = form?.get("t");
  if (typeof token === "string" && token) {
    const lead = await leadFromToken(env, token);
    const value = form?.get("value");
    if (!lead || (value !== "opted_in" && value !== "opted_out")) return preferencesPage("en", { kind: "expired" });
    await setPreference(env.DB, lead.id, value as ProductEmails, "preferences_page", { page: "/email/preferences/", language: lead.signup_language }, new Date().toISOString());
    ctx.waitUntil(syncLead(env, lead.id, fetcher));
    return preferencesPage(language(lead.signup_language), { kind: "show", value: value as ProductEmails, token, saved: true, confirm: false });
  }
  // A new link, mailed to the address only if it is a lead. The answer is the same either
  // way, so the page doesn't reveal who has registered.
  // ponytail: no rate limit; add one if the form is abused (it can only mail a lead their own link).
  const lang = language(typeof form?.get("lang") === "string" ? (form?.get("lang") as string) : null);
  const email = form?.get("email");
  if (typeof email === "string" && env.EMAIL_FROM && env.EMAIL_TOKEN_SECRET) {
    const lead = await leadByEmail(env.DB, email);
    if (lead) {
      const origin = new URL(request.url).origin;
      ctx.waitUntil(send(env, "transactional", preferencesLinkEmail(lead.email, language(lead.signup_language), env.EMAIL_FROM, env.EMAIL_REPLY_TO), { leadId: lead.id, origin, nowMs: Date.now() }, fetcher));
    }
  }
  return preferencesPage(lang, { kind: "link_sent" });
}

/** The footer `send` adds carries the fresh link, so the body only says what it is for. */
function preferencesLinkEmail(to: string, lang: "en" | "es" | "fr", from: string, replyTo: string | undefined): Email {
  const copy = {
    en: ["Your Whimbrel email preferences link", "Here's a new link to change which emails you get from Whimbrel. It's below."],
    es: ["Tu enlace de preferencias de correo de Whimbrel", "Aquí tienes un enlace nuevo para cambiar qué correos recibes de Whimbrel. Está abajo."],
    fr: ["Votre lien de préférences de courriel Whimbrel", "Voici un nouveau lien pour choisir les courriels que vous recevez de Whimbrel. Il se trouve ci-dessous."],
  }[lang];
  const html = `<!doctype html><html><body style="margin:0;background:#F7F2E7;font-family:-apple-system,'Segoe UI',sans-serif;color:#241E19">
<div style="max-width:560px;margin:0 auto;padding:32px 20px">
<p style="font-size:16px;line-height:1.5">${copy[1]}</p>
</div></body></html>`;
  return { from, to, replyTo, subject: copy[0], text: copy[1], html };
}

async function handleOneClickUnsubscribe(request: Request, env: Env, ctx: ExecutionContext, fetcher: typeof fetch): Promise<Response> {
  const lead = await leadFromToken(env, new URL(request.url).searchParams.get("t"));
  if (!lead) return new Response("Invalid or expired link", { status: 400 });
  await setPreference(env.DB, lead.id, "opted_out", "one_click_unsubscribe", { page: null, language: lead.signup_language }, new Date().toISOString());
  ctx.waitUntil(syncLead(env, lead.id, fetcher));
  return new Response("Unsubscribed", { status: 200 });
}

