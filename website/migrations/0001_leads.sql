-- One row per email address. Re-registering updates the row rather than duplicating it.
CREATE TABLE IF NOT EXISTS leads (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  company TEXT,
  role TEXT,
  team_size TEXT,
  platform TEXT NOT NULL,
  marketing_opt_in INTEGER NOT NULL DEFAULT 0,
  utm_source TEXT,
  utm_medium TEXT,
  utm_campaign TEXT,
  referrer TEXT,
  landing_page TEXT,
  country TEXT,
  registrations INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- One row per download redirect served, attributed to a lead when the visitor has registered.
CREATE TABLE IF NOT EXISTS downloads (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  lead_id TEXT REFERENCES leads(id),
  platform TEXT NOT NULL,
  version TEXT,
  country TEXT,
  created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS downloads_lead_id ON downloads(lead_id);
CREATE INDEX IF NOT EXISTS leads_created_at ON leads(created_at);
