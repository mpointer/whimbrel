#!/usr/bin/env python3
"""Summarise the cloud routine runner's report lines for the 14-day gate.

Reads the runner's stdout (one RunReport JSON object per line: ran,
skipped_left_for_computer, rows_pulled, duration_ms) from the files named on
the command line, or from stdin. Lines that are not a report are ignored, and
unknown fields are ignored. It needs no network and no secret.

The report line has no date. A run is dated by, in order: a date field
(date, day, ts or timestamp) in the JSON, a YYYY-MM-DD at the start of the log
line before the JSON, or a YYYY-MM-DD in the file name. A run with none of
these counts as a run but not as a day.

Dates are the host's local date. A timestamp prefix ending in Z gives the UTC date, so
use local-time stamps or date-named files if the run is near midnight.

The gate streak counts a day only if the daily brief ran ("daily_brief" in ran).
Days where only another routine ran are printed separately.

Run: python3 -I scripts/runner-gate-report.py logs/2026-11-01.log logs/2026-11-02.log
"""
import datetime
import json
import math
import os
import re
import sys

DATE_RE = re.compile(r"\d{4}-\d{2}-\d{2}")
DATE_FIELDS = ("date", "day", "ts", "timestamp")
ONE_SECOND_MS = 1000
MIN_MARGIN_MS = 60_000
BRIEF = "daily_brief"
LOW_SAMPLE = 20


def _day(text):
    m = DATE_RE.search(text) if isinstance(text, str) else None
    if not m:
        return None
    try:
        return datetime.date.fromisoformat(m.group(0)).isoformat()
    except ValueError:
        return None


def _num(v):
    """A finite, non-negative number, else None."""
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        return None
    if isinstance(v, float) and not math.isfinite(v):
        return None
    return v if v >= 0 else None


def parse_lines(lines, default_day=None, skipped=None):
    """Return a list of runs: {day, brief, ok, left, rows, ms}. Noise lines are skipped.

    A report line missing or with an unusable duration_ms or rows_pulled is not
    a run; it adds 1 to skipped["incomplete"] if a dict is given."""
    runs = []
    for raw in lines:
        start = raw.find("{")
        if start < 0:
            continue
        try:
            obj = json.loads(raw[start:])
        except ValueError:
            continue
        if not isinstance(obj, dict) or not isinstance(obj.get("ran"), list):
            continue
        ms, rows = _num(obj.get("duration_ms")), _num(obj.get("rows_pulled"))
        if ms is None or rows is None:
            if skipped is not None:
                skipped["incomplete"] = skipped.get("incomplete", 0) + 1
            continue
        day = None
        for key in DATE_FIELDS:
            day = day or _day(obj.get(key))
        day = day or _day(raw[:start]) or default_day
        left = obj.get("skipped_left_for_computer")
        runs.append({
            "day": day,
            "ok": len(obj["ran"]) > 0,
            "brief": BRIEF in obj["ran"],
            "left": isinstance(left, list) and len(left) > 0,
            "rows": rows,
            "ms": ms,
        })
    return runs


def _percentile(values, p):
    """Nearest-rank percentile; None for no values."""
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil(p / 100 * len(ordered)))
    return ordered[rank - 1]


def _stats(values):
    return {"p50": _percentile(values, 50), "p95": _percentile(values, 95), "max": max(values) if values else None}


def _longest_streak(days):
    best = run = 0
    prev = None
    for d in sorted(datetime.date.fromisoformat(x) for x in days):
        run = run + 1 if prev is not None and (d - prev).days == 1 else 1
        best = max(best, run)
        prev = d
    return best


def summarize(runs, skipped=None):
    per_day = {}
    ok_days = set()
    brief_days = set()
    for r in runs:
        if r["day"]:
            per_day[r["day"]] = per_day.get(r["day"], 0) + 1
            if r["ok"]:
                ok_days.add(r["day"])
            if r["brief"]:
                brief_days.add(r["day"])
    durations = [r["ms"] for r in runs]
    p95 = _percentile(durations, 95)
    suggested = None if p95 is None else int(math.ceil(p95 / ONE_SECOND_MS) * ONE_SECOND_MS)
    margin = None if suggested is None else max(MIN_MARGIN_MS, int(math.ceil(suggested / 2 / ONE_SECOND_MS) * ONE_SECOND_MS))
    return {
        "runs": len(runs),
        "undated_runs": sum(1 for r in runs if not r["day"]),
        "runs_per_day": dict(sorted(per_day.items())),
        "skipped_incomplete": (skipped or {}).get("incomplete", 0),
        "days_with_any_routine": len(ok_days),
        "days_with_run": len(brief_days),
        "longest_streak": _longest_streak(brief_days),
        "longest_streak_any_routine": _longest_streak(ok_days),
        "left_for_computer_runs": sum(1 for r in runs if r["left"]),
        "duration_ms": _stats(durations),
        "rows_pulled": _stats([r["rows"] for r in runs]),
        "suggested_ready_by_p95_ms": suggested,
        "suggested_ready_by_margin_ms": margin,
    }


def print_report(r, out=sys.stdout):
    w = lambda s="": print(s, file=out)
    if r["runs"] == 0:
        w("No runs found. Check the log files and that they hold the runner's report lines.")
        return
    w(f"Runs: {r['runs']} ({r['undated_runs']} without a date)")
    w(f"Skipped/incomplete report lines: {r['skipped_incomplete']}")
    w(f"Days with a successful daily brief: {r['days_with_run']}")
    w(f"Longest streak of consecutive days with a daily brief: {r['longest_streak']} (gate: 14)")
    w(f"For information, any routine: {r['days_with_any_routine']} days, longest streak {r['longest_streak_any_routine']}")
    for day, n in r["runs_per_day"].items():
        w(f"  {day}: {n} run(s)")
    w(f"Runs that left a routine for the computer: {r['left_for_computer_runs']}")
    for name, key in (("Duration ms", "duration_ms"), ("Rows pulled", "rows_pulled")):
        s = r[key]
        w(f"{name}: p50 {s['p50']}, p95 {s['p95']}, max {s['max']}")
    if r["runs"] < LOW_SAMPLE:
        w(f"WARNING: only {r['runs']} runs (under {LOW_SAMPLE}). The p95 is not reliable yet.")
    w(f"Suggested READY_BY_P95_MS = {r['suggested_ready_by_p95_ms']}")
    w(f"Suggested READY_BY_MARGIN_MS = {r['suggested_ready_by_margin_ms']}")


def main(argv):
    runs, skipped = [], {}
    if argv:
        for path in argv:
            with open(path, encoding="utf-8", errors="replace") as f:
                runs += parse_lines(f, _day(os.path.basename(path)), skipped)
    else:
        runs = parse_lines(sys.stdin, None, skipped)
    print_report(summarize(runs, skipped))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
