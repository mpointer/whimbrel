#!/usr/bin/env python3
"""Fail when app code gates a feature on a plan value outside the files that hold the server's answer.

    python3 -I scripts/check-paywalls.py [--root DIR] [--allow FILE]

The rule (docs/16-open-core-commercial-model.md, "Always"): a paid feature needs the company's
server or the company's time, and anything that runs on the user's device is in the open app.
So the apps never decide on their own what a plan unlocks. They ask the server (the Worker's
/v1/account/plan and AI balance answers: plan, plan_tier, plus_available, allowance) and show
what it said; the server enforces the limit.

This check looks at app and core source (apps/, core/; tests and generated code are skipped)
for any use of a plan value: isCloud, isPlus, is_plus, planTier, plan_tier, plusAvailable,
isPro, isPaid, entitlement, or a comparison with "plus", "cloud", "pro", "team", "premium" or
"paid". Any use counts, not only an if: `showsExport: isCloud` or `val paid = plan.isPlus`
gates a feature just as well. Such a line is fine only in a file on the allowlist
(scripts/paywall-allow.txt): the files that decode the server's answer, call the server, or
render what it said, each with the reason. Anything else fails the check.

A failure means one of two things:
  * The new gate only changes what the app shows about the server's answer (a plan line, an
    upgrade button, an error message after the server refused). Add the file to the
    allowlist with that reason, narrowed with "::" to the lines that need it.
  * The gate keeps a feature that runs on the device from free users. That is an on-device
    paywall: move the feature to the open app, or make it need the server.

Allowlist format, one entry per line, a reason required:
    path/to/File.swift  # why
    path/to/File.swift :: regex  # why; only lines matching regex are allowed
    ! path/to/File.swift  # a known violation with a decided fix: passes, printed on every run
Standard library only.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_ALLOW = ROOT / "scripts" / "paywall-allow.txt"
SCOPES = ("apps", "core")
EXTENSIONS = (".swift", ".rs", ".ts", ".tsx", ".js", ".mjs", ".cjs", ".kt", ".kts", ".java", ".cs")
SKIP_PATH = re.compile(
    r"(?:^|/)(?:\w*Tests|tests?|__tests__|androidTest|e2e|fixtures|generated|node_modules|target|build)/"
    r"|Tests?\.(?:swift|kt)$|_tests?\.rs$|\.(?:test|spec)\.[jt]sx?$"
)

PLAN_NAMES = r"plus|cloud|pro|team|premium|paid"
PLAN_VALUE = re.compile(
    r"\b(?:isCloud|isPlus|is_cloud|is_plus|isPro|is_pro|isPaid|is_paid|isPremium|is_premium"
    r"|isPlusAccount|plus_flag|PlusFlag|planTier|plan_tier|plusAvailable|plus_available"
    r"|entitlements?|hasEntitlement|has_entitlement)\b"
    rf"|[=!]==?\s*(?:Some\()?[\"'](?:{PLAN_NAMES})[\"']"
    rf"|[\"'](?:{PLAN_NAMES})[\"']\)?\s*[=!]==?"
)
COMMENT_LINE = re.compile(r"^\s*(?://|/\*|\*|#)")


@dataclass
class Entry:
    path: str
    only: re.Pattern[str] | None
    reason: str
    known: bool
    line: int


def read_allowlist(path: Path) -> list[Entry]:
    entries = []
    for n, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        body, sep, reason = line.partition("  # ")
        if not sep or not reason.strip():
            raise ValueError(f"{path.name}:{n}: an entry needs a reason after '  # '")
        known = body.startswith("! ")
        if known:
            body = body[2:]
        file_part, _, only = body.partition(" :: ")
        entries.append(Entry(file_part.strip(), re.compile(only.strip()) if only.strip() else None,
                             reason.strip(), known, n))
    return entries


def source_files(root: Path) -> list[str]:
    try:
        out = subprocess.run(["git", "ls-files", "--", *SCOPES], cwd=root, check=True,
                             capture_output=True, text=True).stdout
        files = out.splitlines()
    except (OSError, subprocess.CalledProcessError):
        files = []
        for scope in SCOPES:
            for dirpath, _, names in os.walk(root / scope):
                files += [(Path(dirpath) / n).relative_to(root).as_posix() for n in names]
    return sorted(f for f in files if f.endswith(EXTENSIONS) and not SKIP_PATH.search(f))


def gate_lines(text: str) -> list[tuple[int, str]]:
    hits = []
    for n, line in enumerate(text.splitlines(), 1):
        if COMMENT_LINE.match(line):
            continue
        code = re.split(r"\s//\s", line, maxsplit=1)[0]
        if PLAN_VALUE.search(code):
            hits.append((n, line.strip()))
    return hits


def check(root: Path, allow: Path) -> tuple[list[str], list[str], list[str]]:
    """(violations, known violations, allowlist problems)."""
    entries = read_allowlist(allow)
    problems = [f"{allow.name}:{e.line}: {e.path} does not exist" for e in entries if not (root / e.path).is_file()]
    by_path: dict[str, list[Entry]] = {}
    for e in entries:
        by_path.setdefault(e.path, []).append(e)
    violations, known = [], []
    used: set[int] = set()
    for rel in source_files(root):
        path = root / rel
        if not path.is_file():
            continue
        for n, line in gate_lines(path.read_text(encoding="utf-8", errors="replace")):
            match = next((e for e in by_path.get(rel, []) if e.only is None or e.only.search(line)), None)
            where = f"{rel}:{n}: {line[:140]}"
            if match is None:
                violations.append(where)
                continue
            used.add(match.line)
            if match.known:
                known.append(f"{rel}  ({match.reason})")
    problems += [f"{allow.name}:{e.line}: {e.path} has no plan gate left; remove the entry"
                 for e in entries if e.line not in used and (root / e.path).is_file()]
    return violations, known, problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--allow", type=Path, default=DEFAULT_ALLOW)
    args = parser.parse_args(argv)
    try:
        violations, known, problems = check(args.root, args.allow)
    except ValueError as err:
        print(f"error: {err}")
        return 1
    if known:
        print(f"KNOWN: {len(known)} plan gate line(s) on features that run on the device alone, "
              f"listed with '!' in {args.allow.name} until their fix lands:")
        for k in sorted(set(known)):
            print(f"  ! {known.count(k):3d} lines in {k}")
    for p in problems:
        print(f"allowlist: {p}")
    if violations:
        print(f"{len(violations)} plan gate(s) outside the files that hold the server's answer "
              f"(see the help in scripts/check-paywalls.py):")
        for v in violations:
            print(f"  {v}")
    if violations or problems:
        return 1
    print("paywall check: no plan gates outside the allowlisted files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
