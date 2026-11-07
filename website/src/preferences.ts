// Product-email preferences: the stored answer per lead, the change log, the signed links
// that change it without a login, and the Resend contact that mirrors it for broadcasts
// (docs/superpowers/specs/2026-11-04-lead-email-preferences-design.md).

export type ProductEmails = "opted_in" | "opted_out";
export type PreferenceSource = "signup_form" | "preferences_page" | "one_click_unsubscribe";
export const LANGUAGES = ["en", "es", "fr"] as const;
export type Language = (typeof LANGUAGES)[number];

export interface LeadRow {
  id: string;
  email: string;
  name: string;
  platform: string;
  product_emails: ProductEmails;
  product_emails_set_at: string | null;
  product_emails_source: string | null;
  product_emails_confirm_sent_at: string | null;
  signup_language: string | null;
  created_at: string;
  followup_sent_at: string | null;
  resend_synced_at: string | null;
}

export function language(raw: string | null | undefined): Language {
  return (LANGUAGES as readonly string[]).includes(raw ?? "") ? (raw as Language) : "en";
}

export async function leadById(db: D1Database, id: string): Promise<LeadRow | null> {
  return db.prepare("SELECT * FROM leads WHERE id = ?1").bind(id).first<LeadRow>();
}

export async function leadByEmail(db: D1Database, email: string): Promise<LeadRow | null> {
  return db.prepare("SELECT * FROM leads WHERE email = ?1").bind(email.trim().toLowerCase()).first<LeadRow>();
}

/** Sets the answer and logs it; no change, no log. Returns whether the answer changed.
 * (`marketing_opt_in` stays the raw "ticked the box at signup" record.) */
export async function setPreference(
  db: D1Database,
  leadId: string,
  value: ProductEmails,
  source: PreferenceSource,
  where: { page?: string | null; language?: string | null },
  now: string,
): Promise<boolean> {
  const changed = await db
    .prepare(
      `UPDATE leads SET product_emails = ?2, product_emails_set_at = ?3, product_emails_source = ?4
       WHERE id = ?1 AND product_emails <> ?2`,
    )
    .bind(leadId, value, now, source)
    .run();
  if (!changed.meta.changes) return false;
  await logEvent(db, leadId, value, source, where, now);
  return true;
}

export async function logEvent(
  db: D1Database,
  leadId: string,
  value: ProductEmails,
  source: string,
  where: { page?: string | null; language?: string | null },
  now: string,
): Promise<void> {
  await db
    .prepare("INSERT INTO email_preference_events (lead_id, value, source, page, language, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
    .bind(leadId, value, source, where.page ?? null, where.language ?? null, now)
    .run();
}

// ---- signed links ----------------------------------------------------------------------
// `<lead id>.<expiry, unix seconds>.<HMAC-SHA256 of the two, base64url>`: names one lead,
// expires, and can't be altered without the secret.

export const TOKEN_DAYS = 180;

export const b64url = (bytes: ArrayBuffer) =>
  btoa(String.fromCharCode(...new Uint8Array(bytes))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

export function fromB64url(s: string): Uint8Array | null {
  try {
    const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/") + "===".slice((s.length + 3) % 4));
    return Uint8Array.from(bin, (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}

export const hmacKey = (secret: string) =>
  crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);

export async function signToken(secret: string, leadId: string, nowMs: number): Promise<string> {
  const exp = Math.floor(nowMs / 1000) + TOKEN_DAYS * 86400;
  const payload = `${leadId}.${exp}`;
  const sig = await crypto.subtle.sign("HMAC", await hmacKey(secret), new TextEncoder().encode(payload));
  return `${payload}.${b64url(sig)}`;
}

/** The lead id a token names, or null when it is malformed, altered or expired. */
export async function verifyToken(secret: string, token: string | null, nowMs: number): Promise<string | null> {
  const m = token?.match(/^([0-9a-f-]{36})\.(\d{1,12})\.([A-Za-z0-9_-]+)$/);
  if (!m) return null;
  const [, leadId, exp, sig] = m;
  if (Number(exp) * 1000 < nowMs) return null;
  const bytes = fromB64url(sig);
  if (!bytes) return null;
  const ok = await crypto.subtle.verify("HMAC", await hmacKey(secret), bytes, new TextEncoder().encode(`${leadId}.${exp}`));
  return ok ? leadId : null;
}

// ---- Resend contact ---------------------------------------------------------------------
// One contact per lead with the "Product updates" topic set to match, so a broadcast sent
// to that topic reaches opted-in leads only. D1 is the source of truth; a failed call is
// retried by the cron (`resend_synced_at` older than `product_emails_set_at`).

export async function syncContact(
  apiKey: string,
  topicId: string,
  lead: Pick<LeadRow, "email" | "name" | "product_emails">,
  fetcher: typeof fetch,
): Promise<boolean> {
  const headers = { authorization: `Bearer ${apiKey}`, "content-type": "application/json" };
  const topics = [{ id: topicId, subscription: lead.product_emails === "opted_in" ? "opt_in" : "opt_out" }];
  try {
    const update = await fetcher(`https://api.resend.com/contacts/${encodeURIComponent(lead.email)}/topics`, {
      method: "PATCH",
      headers,
      body: JSON.stringify(topics), // a bare array: Resend answers 422 to { topics: [...] }
    });
    if (update.ok) return true;
    if (update.status !== 404) {
      console.error(`contacts: Resend refused the topic update: ${update.status} ${await update.text()}`);
      return false;
    }
    const create = await fetcher("https://api.resend.com/contacts", {
      method: "POST",
      headers,
      body: JSON.stringify({ email: lead.email, first_name: lead.name.split(/\s+/)[0] ?? "", topics }),
    });
    if (!create.ok) console.error(`contacts: Resend refused the contact: ${create.status} ${await create.text()}`);
    return create.ok;
  } catch (err) {
    console.error(`contacts: sync failed: ${err}`);
    return false;
  }
}

export async function markSynced(db: D1Database, leadId: string, now: string): Promise<void> {
  await db.prepare("UPDATE leads SET resend_synced_at = ?2 WHERE id = ?1").bind(leadId, now).run();
}
