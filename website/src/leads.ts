export const PLATFORMS = ["mac", "windows"] as const;
export type Platform = (typeof PLATFORMS)[number];

export const ROLES = [
  "executive",
  "manager",
  "individual-contributor",
  "engineer",
  "sales",
  "consultant",
  "founder",
  "other",
] as const;

export const TEAM_SIZES = ["1", "2-10", "11-50", "51-200", "201-1000", "1000+"] as const;

export interface LeadInput {
  email: string;
  name: string;
  company: string | null;
  role: string | null;
  teamSize: string | null;
  platform: Platform;
  marketingOptIn: boolean;
  utmSource: string | null;
  utmMedium: string | null;
  utmCampaign: string | null;
  referrer: string | null;
  landingPage: string | null;
  turnstileToken: string | null;
  /** Honeypot: a field hidden from people. Anything in it means a bot filled the form. */
  honeypot: string;
}

export type ParseResult = { ok: true; lead: LeadInput } | { ok: false; errors: Record<string, string> };

// Deliberately permissive: one @, a dot in the domain, no whitespace. Real validation is
// the person receiving our email; a stricter regex only rejects valid addresses.
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function text(raw: unknown, max: number): string | null {
  if (typeof raw !== "string") return null;
  const trimmed = raw.trim();
  return trimmed.length === 0 ? null : trimmed.slice(0, max);
}

function oneOf<T extends string>(raw: unknown, allowed: readonly T[]): T | null {
  return typeof raw === "string" && (allowed as readonly string[]).includes(raw) ? (raw as T) : null;
}

function truthy(raw: unknown): boolean {
  return raw === true || raw === "true" || raw === "on" || raw === "1";
}

/** Validates a registration body (already decoded from JSON or a form post into a plain
 * object). Unknown values for the select fields are dropped rather than rejected — they
 * are nice-to-have segmentation, not a reason to lose a lead. */
export function parseLead(body: Record<string, unknown>): ParseResult {
  const errors: Record<string, string> = {};

  const email = text(body.email, 254)?.toLowerCase() ?? "";
  if (!EMAIL_RE.test(email)) errors.email = "Enter a valid email address.";

  const name = text(body.name, 100) ?? "";
  if (name.length === 0) errors.name = "Enter your name.";

  const platform = oneOf(body.platform, PLATFORMS);
  if (!platform) errors.platform = "Choose Mac or Windows.";

  if (Object.keys(errors).length > 0 || !platform) return { ok: false, errors };

  return {
    ok: true,
    lead: {
      email,
      name,
      company: text(body.company, 200),
      role: oneOf(body.role, ROLES),
      teamSize: oneOf(body.teamSize, TEAM_SIZES),
      platform,
      marketingOptIn: truthy(body.marketingOptIn),
      utmSource: text(body.utm_source, 200),
      utmMedium: text(body.utm_medium, 200),
      utmCampaign: text(body.utm_campaign, 200),
      referrer: text(body.referrer, 500),
      landingPage: text(body.landing_page, 500),
      turnstileToken: text(body["cf-turnstile-response"], 4096),
      // `kestrel_hp` is the field's name on pages cached from before the rename.
      honeypot: text(body.whimbrel_hp ?? body.kestrel_hp, 200) ?? "",
    },
  };
}

/** Inserts the lead, or — when the email is already registered — refreshes the row
 * (latest platform/profile wins, opt-in is sticky once given, first-touch attribution is
 * kept) and bumps its registration count. Returns the lead's stable id either way, and
 * whether this was the email's first registration. */
export async function upsertLead(
  db: D1Database,
  lead: LeadInput,
  country: string | null,
  now: string,
): Promise<{ id: string; isNew: boolean }> {
  const row = await db
    .prepare(
      `INSERT INTO leads (id, email, name, company, role, team_size, platform, marketing_opt_in,
         utm_source, utm_medium, utm_campaign, referrer, landing_page, country, created_at, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
       ON CONFLICT(email) DO UPDATE SET
         name = excluded.name,
         company = COALESCE(excluded.company, leads.company),
         role = COALESCE(excluded.role, leads.role),
         team_size = COALESCE(excluded.team_size, leads.team_size),
         platform = excluded.platform,
         marketing_opt_in = MAX(leads.marketing_opt_in, excluded.marketing_opt_in),
         country = COALESCE(excluded.country, leads.country),
         registrations = leads.registrations + 1,
         updated_at = excluded.updated_at
       RETURNING id, registrations`,
    )
    .bind(
      crypto.randomUUID(),
      lead.email,
      lead.name,
      lead.company,
      lead.role,
      lead.teamSize,
      lead.platform,
      lead.marketingOptIn ? 1 : 0,
      lead.utmSource,
      lead.utmMedium,
      lead.utmCampaign,
      lead.referrer,
      lead.landingPage,
      country,
      now,
    )
    .first<{ id: string; registrations: number }>();
  if (!row) throw new Error("lead upsert returned no row");
  return { id: row.id, isNew: row.registrations === 1 };
}

export async function leadExists(db: D1Database, id: string): Promise<boolean> {
  const row = await db.prepare("SELECT 1 AS found FROM leads WHERE id = ?1").bind(id).first();
  return row !== null;
}

export async function recordDownload(
  db: D1Database,
  leadId: string | null,
  platform: Platform,
  version: string | null,
  country: string | null,
  now: string,
): Promise<void> {
  await db
    .prepare("INSERT INTO downloads (lead_id, platform, version, country, created_at) VALUES (?1, ?2, ?3, ?4, ?5)")
    .bind(leadId, platform, version, country, now)
    .run();
}
