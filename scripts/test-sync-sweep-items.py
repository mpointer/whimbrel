#!/usr/bin/env python3
"""Offline self-check for sync-sweep-items.py. Never touches Turso. Run: python3 -I scripts/test-sync-sweep-items.py"""
import importlib.util
import os

spec = importlib.util.spec_from_file_location("sweep", os.path.join(os.path.dirname(os.path.abspath(__file__)), "sync-sweep-items.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

assert list(m.windows([5, 150_001], 200_000)) == [(0, 100_000, [5]), (100_000, 200_000, [150_001])]
assert m.where("dev+/=", "items", 0, 10, [3], 99) == (
    "device_id = 'dev+/=' AND row_type = 'items' AND entity_tag IS NULL AND local_seq > 0 "
    "AND local_seq <= 10 AND created_at_unix_ms < 99 AND local_seq NOT IN (3)")
assert "entity_tag IS NULL" in m.base("dev", "items", 0, 10)
assert "meeting" not in m.ROW_TYPES
for bad in (["--row-type", "meeting"], ["--device", "x' OR 1=1 --"], ["--db", "prod"], ["--cutoff", "2026-10-22T17:00:00"]):
    args = ["--db", "kestrel-sync-0cec58bf", "--device", "dev"] + bad
    try:
        m.parse(args)
    except SystemExit:
        continue
    raise AssertionError(bad)
a = m.parse(["--db", "kestrel-sync-0cec58bf", "--device", "dev+/="])
assert not a.apply and a.row_type == "items"

# Dry run + apply end to end against local sqlite files: a fake relay behind turso(), a fake local DB.
import sqlite3, tempfile, time
tmp = tempfile.mkdtemp()
m.PLAN_DIR = os.path.join(tmp, "plans")
loc = sqlite3.connect(os.path.join(tmp, "local.sqlite"))
loc.executescript("CREATE TABLE items (device_id TEXT, local_seq INTEGER); CREATE TABLE sync_push_cursor (id INTEGER, last_pushed_seq INTEGER);"
                  "CREATE TABLE sync_cursor (peer_device_id TEXT PRIMARY KEY, last_pulled_seq INTEGER NOT NULL DEFAULT 0);"
                  "INSERT INTO sync_cursor VALUES ('peer', 3);"
                  "INSERT INTO items VALUES ('dev', 5); INSERT INTO sync_push_cursor VALUES (1, 10);")
loc.commit(); loc.close()
relay = sqlite3.connect(":memory:")
relay.execute("CREATE TABLE sync_row (device_id TEXT, local_seq INTEGER, row_type TEXT, created_at_unix_ms INTEGER, entity_tag TEXT)")
old = int(time.time() * 1000) - 10_000
rows = [("dev", 1, "items", old, None), ("dev", 2, "items", old, None), ("dev", 3, "items", old, "tagged"),  # 3: app-compacted
        ("dev", 5, "items", old, None),                        # 5: live
        ("dev", 7, "meeting", old, None), ("other", 4, "items", old, None),  # wrong type / device
        ("dev", 9, "items", old + 10**9, None),                # newer than the cutoff
        ("dev", 12, "items", old, None)]                       # above the push cursor
relay.executemany("INSERT INTO sync_row VALUES (?,?,?,?,?)", rows)
def fake_turso(db, sql):
    assert db == "kestrel-sync-0cec58bf"
    return [[str(v) for v in r] for r in relay.execute(sql).fetchall()]
m.turso = fake_turso
m.time.sleep = lambda s: None
base_args = ["--db", "kestrel-sync-0cec58bf", "--device", "dev", "--local-db", os.path.join(tmp, "local.sqlite")]
# A local DB that has --device as a peer belongs to another device: refused, nothing saved.
try:
    m.dry_run(m.parse(["--db", "kestrel-sync-0cec58bf", "--device", "peer", "--local-db", os.path.join(tmp, "local.sqlite")]))
    raise AssertionError("wrong-device local DB not refused")
except SystemExit as e:
    assert "another device" in str(e), e
assert not os.path.exists(m.plan_path("kestrel-sync-0cec58bf"))
m.dry_run(m.parse(base_args))
assert relay.execute("SELECT count(*) FROM sync_row").fetchone()[0] == len(rows)  # dry run deletes nothing
try:
    m.apply(m.parse(["--db", "kestrel-sync-0cec58bf", "--device", "dev2", "--apply"]))
    raise AssertionError("plan mismatch not caught")
except SystemExit:
    pass
m.apply(m.parse(base_args + ["--apply"]))
left = sorted(r[0] for r in relay.execute("SELECT local_seq FROM sync_row WHERE device_id = 'dev'"))
assert left == [3, 5, 7, 9, 12], left  # only seqs 1 and 2 go
print("ok")
