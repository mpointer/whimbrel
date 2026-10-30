-- Run once against the live control-plane registry (see docs/open-sign-in-rollout.md).
ALTER TABLE device ADD COLUMN last_seen_unix_ms INTEGER;
ALTER TABLE device ADD COLUMN replaces_device_id TEXT;
ALTER TABLE device ADD COLUMN resetting_since_unix_ms INTEGER;
ALTER TABLE account ADD COLUMN last_seen_unix_ms INTEGER;
ALTER TABLE account ADD COLUMN warned_at_unix_ms INTEGER;
ALTER TABLE account ADD COLUMN unlimited INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account ADD COLUMN reset_notice_pending_unix_ms INTEGER;
ALTER TABLE account ADD COLUMN deleting_since_unix_ms INTEGER;
CREATE TABLE IF NOT EXISTS waitlist (
  account_id TEXT NOT NULL,
  wants TEXT NOT NULL,
  platform TEXT,
  created_at_unix_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS retired_database (
  name TEXT NOT NULL,
  account_id TEXT NOT NULL,
  retired_at_unix_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS account_creation (
  ip_bucket TEXT NOT NULL,
  created_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS account_creation_ip ON account_creation(ip_bucket, created_at_unix_ms);
