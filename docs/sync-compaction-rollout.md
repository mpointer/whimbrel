# Sync row compaction rollout

Spec: `docs/superpowers/specs/2026-10-30-sync-row-compaction-design.md` (D7, D10).
You run every step here by hand. Nothing in the app or in CI touches Turso.

Order matters: **index first, then the app release.** The app compacts only after the index
exists, and a missing index is safe (pushes stay correct, but each DELETE scans the device's rows).

Account databases are `kestrel-sync-*`, except `kestrel-sync-control` and `kestrel-sync-engine-test`,
which are not account DBs. Today only `kestrel-sync-0cec58bf-4570-4237-b779-9675f83c4a0c` exists.

## 1. Measure (read-only)

```bash
turso db list
turso db show kestrel-sync-0cec58bf-4570-4237-b779-9675f83c4a0c      # size; repeat per DB
turso db shell kestrel-sync-0cec58bf-4570-4237-b779-9675f83c4a0c "SELECT device_id, row_type, COUNT(*) FROM sync_row GROUP BY 1, 2"
```

This decides whether any DB besides Mike's needs the sweep (D10). A DB with millions of `items`
rows for a device needs step 4.

## 2. Add the column and index

First check by hand on one DB. The last output line must be the bare count (`0` or `1`),
because the loop relies on `tail -1`:

```bash
turso db shell kestrel-sync-0cec58bf-4570-4237-b779-9675f83c4a0c "SELECT COUNT(*) FROM pragma_table_info('sync_row') WHERE name = 'entity_tag'"
```

Then run the loop. It is idempotent. If the column is missing it runs the whole file. If a client
already added the column, it runs only the index line. Any other output stops the loop.

```bash
for db in $(turso db list | awk '$1 ~ /^kestrel-sync-/ && $1 != "kestrel-sync-control" && $1 != "kestrel-sync-engine-test" {print $1}'); do
  echo "== $db"
  has=$(turso db shell "$db" "SELECT COUNT(*) FROM pragma_table_info('sync_row') WHERE name = 'entity_tag'" | tail -1 | tr -d '[:space:]')
  if [ "$has" = 0 ]; then turso db shell "$db" < scripts/sync-row-entity-tag.sql || break
  elif [ "$has" = 1 ]; then grep '^CREATE INDEX' scripts/sync-row-entity-tag.sql | turso db shell "$db" || break
  else echo "unexpected column check output: '$has'"; break; fi
  sleep 2   # paced; CREATE INDEX reads every row once
done
```

## 3. Verify

For each DB, both must hold: the index is listed, and the column count is 1.

```bash
turso db shell kestrel-sync-0cec58bf-4570-4237-b779-9675f83c4a0c "PRAGMA index_list(sync_row)"     # includes idx_sync_row_entity
turso db shell kestrel-sync-0cec58bf-4570-4237-b779-9675f83c4a0c "SELECT COUNT(*) FROM pragma_table_info('sync_row') WHERE name = 'entity_tag'"   # 1
```

Do not release until every DB passes.

## 4. Backfill (optional, `items` only)

Only for a DB that step 1 showed is bloated, and only for `items`. Never for `meeting` (D10).
The script touches only rows with an empty `entity_tag`. Dry run first, then apply with the same arguments:

```bash
python3 -I scripts/sync-sweep-items.py --db <db> --device <device_id> --row-type items
python3 -I scripts/sync-sweep-items.py --db <db> --device <device_id> --row-type items --apply
```

Run the sweep on the device whose `--device` you pass (or pass that device's database with `--local-db`).

Mike's DB was already swept by the original script, so he does not need this step.

## 5. Release

Ship the build.

## 6. After the release

For a meeting edited several times, this returns nothing (allow one pass of in-flight rows):

```bash
turso db shell kestrel-sync-0cec58bf-4570-4237-b779-9675f83c4a0c "SELECT entity_tag, COUNT(*) FROM sync_row WHERE device_id = '<device_id>' AND entity_tag IS NOT NULL GROUP BY 1 HAVING COUNT(*) > 1"
```

The iPhone shows the meeting's newest title.

## 7. Rollback

Leave the column and index in place. Older builds ignore them.

## 8. New account databases

A DB created after this run gets the column from the app's `CREATE TABLE`, but not the index
(clients never create it). Re-run step 2 for it.
