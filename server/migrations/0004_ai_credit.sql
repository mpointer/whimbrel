-- Run ONCE (ALTER TABLE ADD COLUMN is not idempotent) against the live control-plane registry BEFORE deploying the version that reads these.
-- Whimbrel AI credit: prepaid balance in micro-dollars, open-hold count, a ledger of model and token counts only (never prompt or response text), and open holds.
ALTER TABLE account ADD COLUMN ai_credit_micros INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account ADD COLUMN ai_open_holds INTEGER NOT NULL DEFAULT 0;
CREATE TABLE ai_ledger(id TEXT PRIMARY KEY, account_id TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('purchase','usage','refund')), micros INTEGER NOT NULL, model TEXT, input_tokens INTEGER, output_tokens INTEGER, stripe_ref TEXT UNIQUE, created_at_unix_ms INTEGER NOT NULL, shortfall_micros INTEGER);
CREATE TABLE ai_hold(id TEXT PRIMARY KEY, account_id TEXT NOT NULL, micros INTEGER NOT NULL, created_at_unix_ms INTEGER NOT NULL);
CREATE INDEX ai_hold_account ON ai_hold(account_id);
CREATE INDEX ai_ledger_account ON ai_ledger(account_id);
