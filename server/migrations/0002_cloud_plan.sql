-- Run ONCE (ALTER TABLE ADD COLUMN is not idempotent) against the live control-plane registry BEFORE deploying the version that reads these columns.
-- One transaction: if any statement fails, nothing is applied (run it as one script that stops at the first error; see server/README.md for recovery).
BEGIN;
ALTER TABLE account ADD COLUMN plan TEXT NOT NULL DEFAULT 'free';
ALTER TABLE account ADD COLUMN plan_until_unix_ms INTEGER;
ALTER TABLE account ADD COLUMN plan_interval TEXT;
ALTER TABLE account ADD COLUMN stripe_customer_id TEXT;
ALTER TABLE account ADD COLUMN stripe_subscription_id TEXT;
CREATE TABLE IF NOT EXISTS stripe_event (
  id TEXT PRIMARY KEY,
  received_at_unix_ms INTEGER NOT NULL
);
COMMIT;
