#!/usr/bin/env python3
"""Sweep of superseded `items` rows a device pushed to the content-sync relay (parameterized from the
one-time sweep that already ran on Mike's Mac).

Before 2026-10-22 a calendar bug re-stamped the same events on every poll; each
re-stamp got a new local_seq and was pushed as a new `sync_row` (push is ON CONFLICT
DO NOTHING), so the relay holds ~8.9M `items` rows for this device, of which only the
current version of each local item matters.

Keep rule (a row is kept if ANY holds):
  * live:   its local_seq is still in this device's local `items` table (superset of the
            `unpushed_item_rows` selection; non-pushable seqs were never pushed, so they're
            harmless in the NOT IN list);
  * recent: created_at_unix_ms >= the calendar-fix cutoff (2026-10-22 17:00 America/New_York);
  * future: local_seq > the push cursor at snapshot time (the sweep never looks above it).
Items deletions never push a tombstone (plain local DELETE), so there is nothing else to keep.

Pullers read `local_seq > cursor ORDER BY local_seq LIMIT n`, so gaps are invisible.

Only `items` rows with an empty `entity_tag` are ever touched; rows the app compacts are left alone.
`meeting` is deliberately not supported (spec D10).

Usage (run on the device whose local DB holds the live set):
  sync-sweep-items.py --db kestrel-sync-<id> --device <device_id>           # dry run (default)
  sync-sweep-items.py --db kestrel-sync-<id> --device <device_id> --apply   # SAME snapshot + SQL; resumable
  --cutoff ISO8601_WITH_OFFSET   rows created at or after it are kept (default: snapshot time)
  --local-db PATH                the device's local sqlite (default: the Mac path)
"""
import argparse
import bisect
import json
import os
import random
import re
import sqlite3
import subprocess
import sys
import time
from datetime import datetime

LOCAL = os.path.expanduser("~/Library/Application Support/Whimbrel/core-memory.sqlite")
# Row type -> local live-set query. `meeting` is absent on purpose (spec D10): legacy segments
# resolve through an exact `meeting_local_seq`, so a local_seq keep-list cannot cover them.
ROW_TYPES = {"items": "SELECT local_seq FROM items WHERE device_id = ? AND local_seq IS NOT NULL"}
WINDOW = 100_000
MAX_KEEP = 2_000
PLAN_DIR = os.path.expanduser("~/.cache/whimbrel-sync-sweep")


def plan_path(db):
    return os.path.join(PLAN_DIR, f"{db}.json")


def parser():
    ap = argparse.ArgumentParser(description="Sweep superseded relay rows; dry run unless --apply.")
    ap.add_argument("--db", required=True, help="Turso DB, kestrel-sync-<hex id>")
    ap.add_argument("--device", required=True, help="device_id whose rows to sweep")
    ap.add_argument("--row-type", choices=list(ROW_TYPES), default="items")
    ap.add_argument("--local-db", default=LOCAL, help="the device's local sqlite (read-only)")
    ap.add_argument("--cutoff", help="ISO 8601 with offset; rows created at or after it are kept (default: snapshot time)")
    ap.add_argument("--apply", action="store_true", help="delete (default is dry run)")
    ap.add_argument("--sleep", type=float, default=0.5, help="seconds between --apply windows")
    return ap


def parse(argv=None):
    ap = parser()
    a = ap.parse_args(argv)
    if not re.fullmatch(r"kestrel-sync-[0-9a-f-]+", a.db):
        ap.error("--db must match ^kestrel-sync-[0-9a-f-]+$")
    if not re.fullmatch(r"[A-Za-z0-9+/=_-]+", a.device):  # interpolated into SQL
        ap.error("--device must match ^[A-Za-z0-9+/=_-]+$")
    a.cutoff_ms = None
    if a.cutoff:
        try:
            dt = datetime.fromisoformat(a.cutoff)
        except ValueError:
            ap.error("--cutoff is not ISO 8601")
        if dt.tzinfo is None:
            ap.error("--cutoff needs a UTC offset, e.g. 2026-10-22T17:00:00-04:00")
        a.cutoff_ms = int(dt.timestamp() * 1000)
    return a


def turso(db, sql):
    """Run one statement; return the result rows (list of whitespace-split lines, header dropped)."""
    for attempt in range(5):
        p = subprocess.run(["turso", "db", "shell", db, sql], capture_output=True, text=True, timeout=90)
        out = (p.stdout + p.stderr).strip()
        if p.returncode == 0 and "error" not in out.lower():
            lines = [l.split() for l in p.stdout.splitlines() if l.strip()]
            return lines[1:]
        time.sleep(2 ** attempt)
    sys.exit(f"turso failed: {out[:500]}")


def snapshot(a):
    """Read-only snapshot of the live set and push cursor."""
    c = sqlite3.connect(f"file:{a.local_db}?mode=ro", uri=True)
    # A device never has a pull cursor for itself, so a local DB with one for --device belongs to
    # another device: its keep set would be wrong and --apply would delete live rows.
    if c.execute("SELECT 1 FROM sync_cursor WHERE peer_device_id = ?", (a.device,)).fetchone():
        sys.exit(f"refusing: {a.local_db} has --device {a.device} as a sync peer, so it is another device's database. "
                 "Run the sweep on the device whose --device you pass (or pass that device's database with --local-db).")
    row = c.execute("SELECT last_pushed_seq FROM sync_push_cursor WHERE id = 1").fetchone()
    if row is None:
        sys.exit(f"refusing: {a.local_db} has no push cursor, so it is not a pushing device's database.")
    cap = row[0]
    keep = sorted(r[0] for r in c.execute(ROW_TYPES[a.row_type], (a.device,)))
    c.close()
    now = int(time.time() * 1000)
    return {"db": a.db, "device": a.device, "row_type": a.row_type, "keep": keep, "cap": cap,
            "cutoff_ms": a.cutoff_ms or now, "taken_at": datetime.now().isoformat(), "next_lo": 0}


def windows(keep, cap, lo=0):
    """Deterministic (lo, hi, keep_in_window) windows: <=WINDOW seqs, <=MAX_KEEP keep values each."""
    while lo < cap:
        hi = min(lo + WINDOW, cap)
        i = bisect.bisect_right(keep, lo)
        j = bisect.bisect_right(keep, hi)
        if j - i > MAX_KEEP:
            j = i + MAX_KEEP
            hi = keep[j - 1]
        yield lo, hi, keep[i:j]
        lo = hi


def base(device, row_type, lo, hi):
    # entity_tag IS NULL: rows the app tags are compacted by the app, never by this sweep.
    return (f"device_id = '{device}' AND row_type = '{row_type}' AND entity_tag IS NULL "
            f"AND local_seq > {lo} AND local_seq <= {hi}")


def where(device, row_type, lo, hi, ks, cutoff):
    w = f"{base(device, row_type, lo, hi)} AND created_at_unix_ms < {cutoff}"
    if ks:
        w += f" AND local_seq NOT IN ({','.join(map(str, ks))})"
    return w


def save(plan):
    path = plan_path(plan["db"])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(plan, f)
    os.replace(tmp, path)


def dry_run(a):
    plan = snapshot(a)
    save(plan)
    db, dev, rt = a.db, a.device, a.row_type
    keep, cap, cutoff = plan["keep"], plan["cap"], plan["cutoff_ms"]
    print(f"snapshot: {len(keep)} local {rt} seqs, push cursor {cap}, "
          f"cutoff {cutoff}; plan -> {plan_path(db)}", flush=True)
    tot = {"server": 0, "live": 0, "recent": 0, "delete": 0}
    n = 0
    for lo, hi, ks in windows(keep, cap):
        ks_sql = ",".join(map(str, ks)) or "NULL"
        server, live, recent = map(int, turso(db,
            f"SELECT count(*) AS a, coalesce(sum(local_seq IN ({ks_sql})),0) AS b, "
            f"coalesce(sum(created_at_unix_ms >= {cutoff} AND local_seq NOT IN ({ks_sql})),0) AS c "
            f"FROM sync_row WHERE {base(dev, rt, lo, hi)}")[0])
        delete = int(turso(db, f"SELECT count(*) AS n FROM sync_row WHERE {where(dev, rt, lo, hi, ks, cutoff)}")[0][0])
        assert delete == server - live - recent, (lo, hi, server, live, recent, delete)
        for k, v in zip(tot, (server, live, recent, delete)):
            tot[k] += v
        n += 1
        print(f"({lo},{hi}] server={server} keep_live={live} keep_recent={recent} delete={delete}", flush=True)
    print(f"TOTAL windows={n} server={tot['server']} keep_live={tot['live']} "
          f"keep_recent={tot['recent']} delete={tot['delete']}", flush=True)

    # Spot-check: 20 random to-be-deleted rows must be absent from local items.
    if not tot["delete"]:
        print("nothing to delete; no sample check", flush=True)
        return
    c = sqlite3.connect(f"file:{a.local_db}?mode=ro", uri=True)
    ws = list(windows(keep, cap))
    checked = 0
    while checked < 20:
        lo, hi, ks = random.choice(ws)
        r = turso(db, f"SELECT local_seq AS s FROM sync_row WHERE {where(dev, rt, lo, hi, ks, cutoff)} "
                      f"AND local_seq > {random.randint(lo, hi)} ORDER BY local_seq LIMIT 1")
        if not r:
            continue
        seq = int(r[0][0])
        hit = c.execute("SELECT count(*) FROM items WHERE device_id = ? AND local_seq = ?", (dev, seq)).fetchone()[0]
        print(f"sample seq={seq} in_local_items={hit}", flush=True)
        assert hit == 0, seq
        checked += 1
    print("sample check: 20/20 absent from local items", flush=True)


def apply(a):
    plan = json.load(open(plan_path(a.db)))
    for k, v in (("db", a.db), ("device", a.device), ("row_type", a.row_type)):
        if plan.get(k) != v:
            sys.exit(f"plan {k}={plan.get(k)!r} differs from --{k.replace('_', '-')}={v!r}; rerun the dry run")
    keep, cap, cutoff = plan["keep"], plan["cap"], plan["cutoff_ms"]
    print(f"applying plan from {plan['taken_at']}, resuming at {plan['next_lo']}", flush=True)
    total = 0
    for lo, hi, ks in windows(keep, cap, plan["next_lo"]):
        w = where(a.device, a.row_type, lo, hi, ks, cutoff)
        n = int(turso(a.db, f"SELECT count(*) AS n FROM sync_row WHERE {w}")[0][0])
        if n:
            turso(a.db, f"DELETE FROM sync_row WHERE {w}")
            left = int(turso(a.db, f"SELECT count(*) AS n FROM sync_row WHERE {w}")[0][0])
            assert left == 0, (lo, hi, left)
        total += n
        plan["next_lo"] = hi
        save(plan)
        print(f"({lo},{hi}] deleted={n} total={total}", flush=True)
        time.sleep(a.sleep)  # ponytail: fixed pacing; adaptive backoff if Turso throttles
    print(f"DONE deleted={total}", flush=True)


if __name__ == "__main__":
    a = parse()
    apply(a) if a.apply else dry_run(a)
