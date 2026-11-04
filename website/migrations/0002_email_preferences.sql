-- Product-email preference per lead, the signup page and language, the next-day follow-up,
-- and a log of every preference change
-- (docs/superpowers/specs/2026-11-04-lead-email-preferences-design.md).
ALTER TABLE leads ADD COLUMN product_emails TEXT NOT NULL DEFAULT 'opted_out';
ALTER TABLE leads ADD COLUMN product_emails_set_at TEXT;
ALTER TABLE leads ADD COLUMN product_emails_source TEXT;
ALTER TABLE leads ADD COLUMN signup_page TEXT;
ALTER TABLE leads ADD COLUMN signup_language TEXT;
ALTER TABLE leads ADD COLUMN followup_sent_at TEXT;
-- Double opt-in: when the last "confirm product emails" mail went out (one a day at most).
ALTER TABLE leads ADD COLUMN product_emails_confirm_sent_at TEXT;
-- When the Resend contact last matched product_emails (NULL or older than set_at: resync).
ALTER TABLE leads ADD COLUMN resend_synced_at TEXT;

CREATE TABLE IF NOT EXISTS email_preference_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  lead_id TEXT NOT NULL REFERENCES leads(id),
  value TEXT NOT NULL,
  source TEXT NOT NULL,
  page TEXT,
  language TEXT,
  at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS email_preference_events_lead_id ON email_preference_events(lead_id);

-- Backfill (idempotent): every existing answer kept, yes -> opted_in, anything else -> opted_out.
-- Existing leads count as already followed up: they were welcomed by hand.
UPDATE leads SET
  product_emails = IIF(marketing_opt_in = 1, 'opted_in', 'opted_out'),
  product_emails_set_at = created_at,
  product_emails_source = 'signup_form',
  signup_language = IIF(landing_page = '/es' OR landing_page LIKE '/es/%', 'es',
    IIF(landing_page = '/fr' OR landing_page LIKE '/fr/%', 'fr', 'en')),
  followup_sent_at = COALESCE(followup_sent_at, created_at)
WHERE product_emails_source IS NULL;

INSERT INTO email_preference_events (lead_id, value, source, page, language, at)
SELECT id, product_emails, 'backfill', landing_page, signup_language, product_emails_set_at FROM leads
WHERE NOT EXISTS (SELECT 1 FROM email_preference_events e WHERE e.lead_id = leads.id);
