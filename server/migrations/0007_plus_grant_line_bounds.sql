-- Run ONCE (ALTER TABLE ADD COLUMN is not idempotent) against the live control-plane registry BEFORE deploying the version that reads these.
-- Plus yearly grants: each grant row records its Stripe invoice line's period (start and end, ms), so the cron's month windows follow the paid yearly line, not plan_until (which a mid-year cancel_at moves). Additive and nullable: rows written before this (and zero markers) keep NULL, and the cron falls back to deriving the year from plan_until.
-- Each grant row, zero marker and refund tombstone also records the Stripe subscription it belongs to, so a zero blocks only that subscription (a new subscription bought after a cancel or refund still grants). NULL on rows written before this keeps the old rule: any marker blocks.
ALTER TABLE ai_allowance_grant ADD COLUMN line_start_unix_ms INTEGER;
ALTER TABLE ai_allowance_grant ADD COLUMN line_end_unix_ms INTEGER;
ALTER TABLE ai_allowance_grant ADD COLUMN subscription_id TEXT;
