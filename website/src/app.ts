import { leadExists, parseLead, PLATFORMS, recordDownload, upsertLead, type LeadInput, type Platform } from "./leads";
import { resolveRelease } from "./downloads";
import { leadAlertEmail, sendEmail, welcomeEmail } from "./email";

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

function country(request: Request): string | null {
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

function redirect(location: string, headers: Record<string, string> = {}): Response {
  return new Response(null, { status: 303, headers: { location, "cache-control": "no-store", ...headers } });
}

async function readBody(request: Request): Promise<{ body: Record<string, unknown>; isJson: boolean } | null> {
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
  if (lead.honeypot) return isJson ? Response.json({ ok: true, next: thanks }) : redirect(thanks);

  if (env.TURNSTILE_SECRET) {
    const ok = await verifyTurnstile(env.TURNSTILE_SECRET, lead.turnstileToken, request.headers.get("cf-connecting-ip"), fetcher);
    if (!ok) return fail(403, { form: "Verification failed. Refresh the page and try again." });
  }

  const now = new Date().toISOString();
  const countryCode = country(request);
  const { id, isNew } = await upsertLead(env.DB, lead, countryCode, now);

  // Email never blocks or fails the registration: it runs after the response is sent.
  if (env.RESEND_API_KEY && env.EMAIL_FROM) {
    const key = env.RESEND_API_KEY;
    const from = env.EMAIL_FROM;
    const origin = new URL(request.url).origin;
    if (isNew) ctx.waitUntil(sendEmail(key, welcomeEmail(origin, id, lead, from, env.EMAIL_REPLY_TO), fetcher));
    if (env.LEAD_ALERT_EMAIL) ctx.waitUntil(sendEmail(key, leadAlertEmail(lead, isNew, countryCode, from, env.LEAD_ALERT_EMAIL), fetcher));
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

  if (pathname.startsWith("/api/")) return Response.json({ error: "not found" }, { status: 404 });
  return env.ASSETS.fetch(request);
}
