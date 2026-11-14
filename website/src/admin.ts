// GET /admin/leads (HTML) and /admin/leads.csv: every lead with what they signed up with, their
// downloads and their mobile beta request, for Mike only.
//
// Access: the route sits behind a Cloudflare Access application (Zero Trust → Access →
// Applications, path whimbrelapp.com/admin/*, policy: Mike's email). Access adds a signed JWT
// (`Cf-Access-Jwt-Assertion`); this Worker verifies it against the team's public keys, the
// application's audience tag and ADMIN_EMAILS, so the page stays closed even if the Access
// application is removed or misconfigured. Without ADMIN_ACCESS_TEAM and ADMIN_ACCESS_AUD set,
// the page answers 404.

export interface AdminEnv {
  DB: D1Database;
  /** The Access team domain, e.g. "whimbrel.cloudflareaccess.com". */
  ADMIN_ACCESS_TEAM?: string;
  /** The Access application's audience (AUD) tag. */
  ADMIN_ACCESS_AUD?: string;
  /** Who may see the page, comma-separated emails (checked after Access). */
  ADMIN_EMAILS?: string;
  /** More allowed emails, as a Worker secret, so personal addresses stay out of the repo. */
  ADMIN_EMAILS_EXTRA?: string;
  /** The sync control database (libsql://… URL) and a READ-ONLY Turso token for it, both Worker
   * secrets. With them the page shows whether each lead uses the app; without them it says how to
   * turn that on. */
  ADMIN_SYNC_DB_URL?: string;
  ADMIN_SYNC_DB_TOKEN?: string;
}

// ---- Access JWT ------------------------------------------------------------------------------

interface Jwk extends JsonWebKey {
  kid?: string;
}

function b64urlBytes(s: string): Uint8Array {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "=");
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
}

function b64urlJson(s: string): Record<string, unknown> | null {
  try {
    return JSON.parse(new TextDecoder().decode(b64urlBytes(s)));
  } catch {
    return null;
  }
}

/** The verified email from the Access token, or null. `now` is seconds since the epoch. */
export async function accessEmail(
  request: Request,
  env: AdminEnv,
  fetcher: typeof fetch,
  now: number = Math.floor(Date.now() / 1000),
): Promise<string | null> {
  // Each refusal logs one reason code (never the token or the email), for `wrangler tail`.
  const refuse = (reason: string): null => {
    console.warn(`admin/leads refused: ${reason}`);
    return null;
  };
  const team = env.ADMIN_ACCESS_TEAM?.trim(), aud = env.ADMIN_ACCESS_AUD?.trim();
  if (!team || !aud) return refuse("not_configured");
  const token = request.headers.get("cf-access-jwt-assertion");
  if (!token) return refuse("no_token_header");
  const parts = token.split(".");
  if (parts.length !== 3) return refuse("malformed_token");
  const header = b64urlJson(parts[0]), claims = b64urlJson(parts[1]);
  if (!header || !claims || header.alg !== "RS256") return refuse(`bad_header alg=${String(header?.alg)}`);

  const res = await fetcher(`https://${team}/cdn-cgi/access/certs`);
  if (!res.ok) return refuse(`certs_http_${res.status}`);
  const { keys } = (await res.json()) as { keys?: Jwk[] };
  const jwk = keys?.find((k) => k.kid === header.kid);
  if (!jwk) return refuse(`unknown_kid (${keys?.length ?? 0} keys)`);
  let verified = false;
  try {
    const { kid: _kid, ...material } = jwk;
    const key = await crypto.subtle.importKey("jwk", material, { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, false, ["verify"]);
    const signed = new TextEncoder().encode(`${parts[0]}.${parts[1]}`);
    verified = await crypto.subtle.verify("RSASSA-PKCS1-v1_5", key, b64urlBytes(parts[2]), signed);
  } catch (e) {
    return refuse(`key_error ${(e as Error).name}`);
  }
  if (!verified) return refuse("bad_signature");

  const audience = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
  if (!audience.includes(aud)) return refuse("wrong_audience");
  if (claims.iss !== `https://${team}`) return refuse(`wrong_issuer ${String(claims.iss)}`);
  if (typeof claims.exp !== "number" || claims.exp < now) return refuse("expired");
  const email = typeof claims.email === "string" ? claims.email.toLowerCase() : "";
  const allowed = `${env.ADMIN_EMAILS ?? ""},${env.ADMIN_EMAILS_EXTRA ?? ""}`.split(",").map((e) => e.trim().toLowerCase()).filter(Boolean);
  return email && allowed.includes(email) ? email : refuse("email_not_allowed");
}

// ---- data ------------------------------------------------------------------------------------

export interface LeadRow {
  email: string;
  name: string;
  company: string | null;
  role: string | null;
  team_size: string | null;
  platform: string;
  marketing_opt_in: number;
  source: string | null;
  country: string | null;
  registrations: number;
  created_at: string;
  downloads: number;
  last_download_at: string | null;
  last_download: string | null;
  beta: string | null;
  /** From the sync database (`addAppUse`): "free · mac, ios", or "no" when the email has no account. */
  app?: string;
  /** Latest time any of their devices, or the account, reached the sync server (UTC). */
  app_last_active?: string;
}

// ---- app use (sync database) -----------------------------------------------------------------

export interface AppUse {
  plan: string;
  platforms: string;
  lastActiveMs: number;
}

/** Per lowercase email, the sync account behind it: "off" when the secrets aren't set, "error" when
 * the database can't be read (logged, no secrets in the log). */
export async function appUse(env: AdminEnv, emails: string[], fetcher: typeof fetch): Promise<Map<string, AppUse> | "off" | "error"> {
  const raw = env.ADMIN_SYNC_DB_URL?.trim(), token = env.ADMIN_SYNC_DB_TOKEN?.trim();
  if (!raw || !token) return "off";
  const base = raw.replace(/^libsql:\/\//, "https://").replace(/\/$/, "");
  const out = new Map<string, AppUse>();
  const unique = [...new Set(emails.map((e) => e.toLowerCase()))];
  for (let i = 0; i < unique.length; i += 100) {
    const chunk = unique.slice(i, i + 100);
    const sql = `SELECT lower(a.email), a.plan,
        MAX(COALESCE(MAX(d.last_seen_unix_ms), 0), COALESCE(a.last_seen_unix_ms, 0)),
        COALESCE(group_concat(DISTINCT d.platform), '')
      FROM account a LEFT JOIN device d ON d.account_id = a.id AND d.revoked_at_unix_ms IS NULL
      WHERE lower(a.email) IN (${chunk.map(() => "?").join(",")}) GROUP BY a.id`;
    const res = await fetcher(`${base}/v2/pipeline`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify({
        requests: [{ type: "execute", stmt: { sql, args: chunk.map((value) => ({ type: "text", value })) } }, { type: "close" }],
      }),
    });
    if (!res.ok) {
      console.warn(`admin/leads: sync database http ${res.status}`);
      return "error";
    }
    type Cell = { type: string; value?: string };
    const body = (await res.json()) as { results?: { type: string; response?: { result?: { rows?: Cell[][] } } }[] };
    const first = body.results?.[0];
    if (first?.type !== "ok") {
      console.warn("admin/leads: sync database query failed");
      return "error";
    }
    for (const row of first.response?.result?.rows ?? []) {
      const [email, plan, last, platforms] = row.map((c) => c.value ?? "");
      out.set(email, { plan, platforms: platforms.split(",").filter(Boolean).sort().join(", "), lastActiveMs: Number(last) || 0 });
    }
  }
  return out;
}

/** Fills each row's app columns from `use`. */
export function addAppUse(rows: LeadRow[], use: Map<string, AppUse>): void {
  for (const row of rows) {
    const u = use.get(row.email.toLowerCase());
    row.app = u ? [u.plan, u.platforms].filter(Boolean).join(" · ") : "no";
    row.app_last_active = u?.lastActiveMs ? new Date(u.lastActiveMs).toISOString() : "";
  }
}

/** Leads signed up since `since` (ISO, or null for all), newest first. */
export async function listLeads(db: D1Database, since: string | null): Promise<LeadRow[]> {
  const { results } = await db
    .prepare(
      `SELECT l.email, l.name, l.company, l.role, l.team_size, l.platform, l.marketing_opt_in,
              COALESCE(l.utm_source, l.referrer) AS source, l.country, l.registrations, l.created_at,
              (SELECT COUNT(*) FROM downloads d WHERE d.lead_id = l.id) AS downloads,
              (SELECT MAX(d.created_at) FROM downloads d WHERE d.lead_id = l.id) AS last_download_at,
              (SELECT d.platform || COALESCE(' ' || d.version, '') FROM downloads d WHERE d.lead_id = l.id
                 ORDER BY d.created_at DESC LIMIT 1) AS last_download,
              (SELECT b.status || ' (' || b.phones || ')' FROM beta_signups b WHERE b.lead_id = l.id
                 ORDER BY b.created_at DESC LIMIT 1) AS beta
         FROM leads l
        WHERE ?1 IS NULL OR l.created_at >= ?1
        ORDER BY l.created_at DESC`,
    )
    .bind(since)
    .all<LeadRow>();
  return results ?? [];
}

/** The window from `?days=`: 7 (default), 30, or all. */
export function windowFrom(url: URL, now: Date): { days: number | null; since: string | null } {
  const raw = url.searchParams.get("days");
  const days = raw === "all" ? null : raw === "30" ? 30 : 7;
  return { days, since: days === null ? null : new Date(now.getTime() - days * 86_400_000).toISOString() };
}

// ---- output ----------------------------------------------------------------------------------

const COLUMNS: [keyof LeadRow, string][] = [
  ["created_at", "Signed up"], ["name", "Name"], ["email", "Email"], ["company", "Company"], ["role", "Role"],
  ["team_size", "Team"], ["platform", "Platform"], ["downloads", "Downloads"], ["last_download", "Last download"],
  ["last_download_at", "Downloaded at"], ["app", "Uses the app"], ["app_last_active", "Last active"], ["beta", "Mobile beta"], ["source", "Source"], ["country", "Country"],
  ["marketing_opt_in", "Product emails"], ["registrations", "Sign-ups"],
];

function esc(v: unknown): string {
  return String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

function cell(key: keyof LeadRow, row: LeadRow): string {
  const v = row[key];
  if (key === "marketing_opt_in") return v ? "yes" : "no";
  if ((key === "created_at" || key === "last_download_at" || key === "app_last_active") && typeof v === "string") return v.slice(0, 16).replace("T", " ");
  return v === null || v === undefined ? "" : String(v);
}

/** A CSV cell; a leading = + - @ is quoted with a ' so a spreadsheet never runs it as a formula. */
function csvCell(s: string): string {
  const safe = /^[=+\-@\t\r]/.test(s) ? `'${s}` : s;
  return /[",\n\r]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe;
}

export function leadsCsv(rows: LeadRow[]): string {
  const lines = [COLUMNS.map(([, h]) => h).join(",")];
  for (const row of rows) lines.push(COLUMNS.map(([k]) => csvCell(cell(k, row))).join(","));
  return lines.join("\r\n") + "\r\n";
}

export function leadsHtml(rows: LeadRow[], days: number | null, viewer: string, app: "ok" | "off" | "error" = "off"): string {
  const downloaded = rows.filter((r) => r.downloads > 0).length;
  const using = rows.filter((r) => r.app && r.app !== "no").length;
  const appLine = app === "ok"
    ? ` ${using} use the app (have a sync account).`
    : app === "error"
      ? ` <span class="muted">App use couldn't be read from the sync database just now.</span>`
      : ` <span class="muted">App use isn't shown: add the ADMIN_SYNC_DB_URL and ADMIN_SYNC_DB_TOKEN secrets.</span>`;
  const link = (d: string, label: string) =>
    `<a href="/admin/leads?days=${d}"${String(days ?? "all") === d ? ' aria-current="page"' : ""}>${label}</a>`;
  const head = COLUMNS.map(([, h]) => `<th>${esc(h)}</th>`).join("");
  const body = rows
    .map((r) => `<tr class="${r.downloads ? "" : "nodl"}">${COLUMNS.map(([k]) => `<td>${esc(cell(k, r))}</td>`).join("")}</tr>`)
    .join("");
  const span = days === null ? "all time" : `the last ${days} days`;
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex,nofollow"><title>Whimbrel leads</title>
<style>body{font:14px system-ui,sans-serif;margin:24px;color:#1d2433}nav a{margin-right:12px}nav a[aria-current]{font-weight:600;text-decoration:none;color:inherit}
.wrap{overflow-x:auto}table{border-collapse:collapse;margin-top:16px;white-space:nowrap}th,td{border-bottom:1px solid #e3e6ec;padding:6px 10px;text-align:left}
th{position:sticky;top:0;background:#f6f7f9}tr.nodl td{color:#7a8294}.muted{color:#7a8294}</style></head><body>
<h1>Leads</h1>
<p>${rows.length} signed up in ${span}; ${downloaded} downloaded.${appLine}</p>
<nav>${link("7", "Last 7 days")}${link("30", "Last 30 days")}${link("all", "All")} · <a href="/admin/leads.csv?days=${days ?? "all"}">Download CSV</a></nav>
<div class="wrap"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>
<p class="muted">Grey rows haven't downloaded. Signed in as ${esc(viewer)}. Times are UTC.</p>
</body></html>`;
}

// ---- handler ---------------------------------------------------------------------------------

const PRIVATE = { "cache-control": "no-store", "x-robots-tag": "noindex, nofollow", "referrer-policy": "no-referrer" };

/** `/admin/leads` and `/admin/leads.csv`. 404 (not 401) to anyone the Access token doesn't name. */
export async function handleAdminLeads(request: Request, env: AdminEnv, fetcher: typeof fetch, now: Date = new Date()): Promise<Response> {
  if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
  const viewer = await accessEmail(request, env, fetcher, Math.floor(now.getTime() / 1000));
  if (!viewer) return new Response("Not Found", { status: 404, headers: PRIVATE });
  const url = new URL(request.url);
  const { days, since } = windowFrom(url, now);
  const rows = await listLeads(env.DB, since);
  const use = await appUse(env, rows.map((r) => r.email), fetcher);
  if (use instanceof Map) addAppUse(rows, use);
  const appState = use instanceof Map ? "ok" : use;
  if (url.pathname.endsWith(".csv")) {
    return new Response(leadsCsv(rows), {
      headers: { ...PRIVATE, "content-type": "text/csv; charset=utf-8", "content-disposition": `attachment; filename="whimbrel-leads-${days ?? "all"}.csv"` },
    });
  }
  return new Response(leadsHtml(rows, days, viewer, appState), { headers: { ...PRIVATE, "content-type": "text/html; charset=utf-8" } });
}
