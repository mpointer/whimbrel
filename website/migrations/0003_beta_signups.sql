-- Mobile beta requests (docs/superpowers/specs/2026-11-06-mobile-beta-design.md). Mike
-- approves or declines each from signed links in the alert email.
CREATE TABLE IF NOT EXISTS beta_signups (
  id TEXT PRIMARY KEY,
  lead_id TEXT NOT NULL REFERENCES leads(id),
  email TEXT NOT NULL,
  phones TEXT NOT NULL,                     -- iphone | android | both
  computer TEXT NOT NULL,                   -- mac | windows | none
  status TEXT NOT NULL DEFAULT 'pending',   -- pending | approved | declined
  created_at TEXT NOT NULL,
  decided_at TEXT,
  install_email_sent_at TEXT                -- set once; guards the install email
);

-- One open request per email: a repeat sign-up updates the pending row.
CREATE UNIQUE INDEX IF NOT EXISTS beta_signups_pending_email ON beta_signups(email) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS beta_signups_lead_id ON beta_signups(lead_id);
