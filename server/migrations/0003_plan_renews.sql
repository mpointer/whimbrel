-- Run ONCE (ALTER TABLE ADD COLUMN is not idempotent) against the live control-plane registry BEFORE deploying the version that reads this column.
-- 1 = the Cloud plan renews; 0 = it was cancelled and ends at plan_until_unix_ms.
ALTER TABLE account ADD COLUMN plan_renews INTEGER NOT NULL DEFAULT 1;
