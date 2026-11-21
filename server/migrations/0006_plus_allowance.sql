-- Run ONCE (ALTER TABLE ADD COLUMN is not idempotent) against the live control-plane registry BEFORE deploying the version that reads these.
-- Plus plan: a monthly AI allowance that is spent before prepaid credit. plan_tier says which paid product (NULL for free accounts); the allowance and its expiry live on the account; holds remember the allowance share and window they drew from; grants have their own table (ai_ledger.kind has a CHECK constraint SQLite cannot alter in place); server_setting holds the one tunable (plus_allowance_micros).
ALTER TABLE account ADD COLUMN plan_tier TEXT;
ALTER TABLE account ADD COLUMN ai_allowance_micros INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account ADD COLUMN ai_allowance_expires_ms INTEGER;
ALTER TABLE ai_hold ADD COLUMN from_allowance_micros INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_hold ADD COLUMN allowance_expires_ms INTEGER;
ALTER TABLE ai_ledger ADD COLUMN from_allowance_micros INTEGER;
CREATE TABLE ai_allowance_grant(grant_key TEXT PRIMARY KEY, account_id TEXT NOT NULL, micros INTEGER NOT NULL, period_end_unix_ms INTEGER NOT NULL, created_at_unix_ms INTEGER NOT NULL);
CREATE INDEX ai_allowance_grant_account ON ai_allowance_grant(account_id);
CREATE TABLE server_setting(key TEXT PRIMARY KEY, value TEXT NOT NULL);
