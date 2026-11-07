import { language, type Language } from "./preferences";
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
  /** The path of the page the form was on, and that page's language. */
  signupPage: string | null;
  signupLanguage: Language;
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

export function oneOf<T extends string>(raw: unknown, allowed: readonly T[]): T | null {
  return typeof raw === "string" && (allowed as readonly string[]).includes(raw) ? (raw as T) : null;
}

function truthy(raw: unknown): boolean {
  return raw === true || raw === "true" || raw === "on" || raw === "1";
}

/** Who is signing up: the fields every form shares (registration and the mobile beta). */
export type Contact = Omit<LeadInput, "platform">;

/** Validates the shared fields of a sign-up body (already decoded from JSON or a form
 * post into a plain object). Unknown values for the select fields are dropped rather than
 * rejected: they are nice-to-have segmentation, not a reason to lose a lead. */
export function parseContact(body: Record<string, unknown>): { contact: Contact; errors: Record<string, string> } {
  const errors: Record<string, string> = {};

  const email = text(body.email, 254)?.toLowerCase() ?? "";
  if (!EMAIL_RE.test(email)) errors.email = "Enter a valid email address.";

  const name = text(body.name, 100) ?? "";
  if (name.length === 0) errors.name = "Enter your name.";

  return {
    errors,
    contact: {
      email,
      name,
      company: text(body.company, 200),
      role: oneOf(body.role, ROLES),
      teamSize: oneOf(body.teamSize, TEAM_SIZES),
      marketingOptIn: truthy(body.marketingOptIn),
      utmSource: text(body.utm_source, 200),
      utmMedium: text(body.utm_medium, 200),
      utmCampaign: text(body.utm_campaign, 200),
      referrer: text(body.referrer, 500),
      landingPage: text(body.landing_page, 500),
      signupPage: text(body.signup_page, 500),
      signupLanguage: language(text(body.signup_language, 10)),
      turnstileToken: text(body["cf-turnstile-response"], 4096),
      // `kestrel_hp` is the field's name on pages cached from before the rename.
      honeypot: text(body.whimbrel_hp ?? body.kestrel_hp, 200) ?? "",
    },
  };
}

/** Validates a registration body: the shared fields plus the platform to download. */
export function parseLead(body: Record<string, unknown>): ParseResult {
  const { contact, errors } = parseContact(body);
  const platform = oneOf(body.platform, PLATFORMS);
  if (!platform) errors.platform = "Choose Mac or Windows.";
  if (Object.keys(errors).length > 0 || !platform) return { ok: false, errors };
  return { ok: true, lead: { ...contact, platform } };
}

/** Inserts the lead, or — when the email is already registered — refreshes the row
 * (latest platform/profile wins, opt-in is sticky once given, first-touch attribution is
 * kept) and bumps its registration count. Returns the lead's stable id either way, and
 * whether this was the email's first registration. A null platform (a mobile beta sign-up
 * with no computer) keeps the lead's platform, or stores 'none' for a new lead. */
export async function upsertLead(
  db: D1Database,
  lead: Contact & { platform: Platform | null },
  country: string | null,
  now: string,
): Promise<{ id: string; isNew: boolean }> {
  const row = await db
    .prepare(
      `INSERT INTO leads (id, email, name, company, role, team_size, platform, marketing_opt_in,
         utm_source, utm_medium, utm_campaign, referrer, landing_page, country, created_at, updated_at,
         product_emails, product_emails_set_at, product_emails_source, signup_page, signup_language)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, COALESCE(?7, 'none'), ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15,
         'opted_out', ?15, 'signup_form', ?16, ?17)
       ON CONFLICT(email) DO UPDATE SET
         name = excluded.name,
         company = COALESCE(excluded.company, leads.company),
         role = COALESCE(excluded.role, leads.role),
         team_size = COALESCE(excluded.team_size, leads.team_size),
         platform = COALESCE(?7, leads.platform),
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
      lead.signupPage,
      lead.signupLanguage,
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
