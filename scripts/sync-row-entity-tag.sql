-- Out-of-band DDL for sync row compaction (spec D7: docs/superpowers/specs/2026-10-30-sync-row-compaction-design.md).
-- Run once per kestrel-sync-* database BEFORE the release. See docs/sync-compaction-rollout.md.
--
-- Idempotency comes from the procedure, not the SQL: SQLite has no ADD COLUMN IF NOT EXISTS.
-- Run this whole file only when sync_row has no entity_tag column. When the column already
-- exists (a client added it), run only the CREATE INDEX line (it is IF NOT EXISTS).
--
-- Old builds are unaffected (their INSERTs name five columns). To roll back, leave both in place.
ALTER TABLE sync_row ADD COLUMN entity_tag TEXT;
CREATE INDEX IF NOT EXISTS idx_sync_row_entity ON sync_row(device_id, entity_tag, local_seq) WHERE entity_tag IS NOT NULL;
