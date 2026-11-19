-- Run ONCE (ALTER TABLE ADD COLUMN is not idempotent) against the live control-plane registry BEFORE deploying the version that reads this column.
-- SHA-256 (lowercase hex) of the cloud runner's token. NULL for every ordinary device. The token itself is never stored.
ALTER TABLE device ADD COLUMN runner_token_hash TEXT;
