#!/usr/bin/env python3
"""Look for private material in a tree that is about to be published.

    python3 -I scripts/public-review.py TREE [--allow FILE]

TREE is the exported release (scripts/publish-public.sh passes its export folder). Prints one
line per hit and exits 1 when any hit is not allowlisted, 0 otherwise. Three kinds of hit:

* secret: a credential in a known format. The formats are the ones core/memory/src/redact.rs
  removes from captured text; an obvious placeholder (EXAMPLE, abcdefgh, 123456, "secret",
  "fake", ...) is not a hit, so test fixtures pass without an allowlist entry.
* marker: a line or a file name that labels itself private: the upper-case words PRIVATE,
  CONFIDENTIAL, INTERNAL ONLY, DO NOT PUBLISH and the like.
* file: a file whose name says it holds secrets (.env, a key or keystore, Secrets.xcconfig,
  secrets.properties, ...). An .example, .sample or .template copy is fine.

The allowlist (default scripts/public-review-allow.txt) has one repository-relative path per
line followed by "  # why it is safe"; an entry without a reason is an error. Standard library
only, so it runs anywhere python3 does.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_ALLOW = ROOT / "scripts" / "public-review-allow.txt"

SECRETS: list[tuple[str, re.Pattern[str]]] = [
    # A real key has a body; a bare header in a redaction test does not.
    ("private key", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----\s*[A-Za-z0-9+/=\s]{64,}")),
    ("Anthropic key", re.compile(r"sk-ant-[A-Za-z0-9_\-]{20,}")),
    ("OpenAI key", re.compile(r"(?:\bsk-|sk-(?:proj|svcacct|admin)-)[A-Za-z0-9_\-]{20,}")),
    ("GitHub token", re.compile(r"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})")),
    ("AWS key", re.compile(r"(?:AKIA|ASIA)[0-9A-Z]{16}\b")),
    ("Google key", re.compile(r"AIza[0-9A-Za-z_\-]{35}")),
    ("Slack token", re.compile(r"xox[abposr]-[A-Za-z0-9\-]{10,}")),
    ("Stripe key", re.compile(r"[rs]k_live_[A-Za-z0-9]{16,}")),
    ("JWT", re.compile(r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}")),
]

# Values that are plainly made up. Checked against the matched text only.
PLACEHOLDER = re.compile(
    r"(?i)example|abcdefgh|123456|0123|secret|fake|dummy|placeholder|redacted|xxxxxx|test|aaaaaaaa"
)

# Upper case only: "private" in prose or code is ordinary, PRIVATE as a word is a label.
MARKER = re.compile(
    r"\b(?:INTERNAL[ _-]ONLY|INTERNAL USE ONLY|CONFIDENTIAL|DO NOT PUBLISH|NOT FOR PUBLICATION"
    r"|PRIVATE(?! KEY))\b"
)
# The same labels in a file or folder name (PRIVATE-notes.md, INTERNAL/plan.md).
MARKER_NAME = re.compile(r"(?:^|[/_.\- ])(?:PRIVATE|INTERNAL|CONFIDENTIAL|DO[_\- ]NOT[_\- ]PUBLISH)(?:$|[/_.\- ])")

SECRET_NAME = re.compile(
    r"(?:^\.env(?:\..+)?$|^\.dev\.vars$|\.(?:pem|p12|pfx|key|jks|keystore|mobileprovision)$"
    r"|^id_(?:rsa|dsa|ecdsa|ed25519)$|^Secrets\.xcconfig$|^secrets\.properties$"
    r"|^keystore\.properties$|^credentials\.json$|^service-account.*\.json$)"
)
SAFE_COPY = re.compile(r"\.(?:example|sample|template)$")

MAX_BYTES = 8 * 1024 * 1024


def read_allowlist(path: Path) -> dict[str, str]:
    allowed: dict[str, str] = {}
    if not path.exists():
        return allowed
    for n, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        entry, sep, reason = line.partition(" # ")
        if not sep or not reason.strip():
            raise SystemExit(f"{path}:{n}: an allowlist entry needs a reason after ' # '")
        allowed[entry.strip()] = reason.strip()
    return allowed


def scan_text(rel: str, text: str) -> list[str]:
    hits = []
    for kind, pattern in SECRETS:
        for m in pattern.finditer(text):
            if PLACEHOLDER.search(m.group(0)):
                continue
            line = text.count("\n", 0, m.start()) + 1
            # Never print the value itself, only where it is.
            hits.append(f"secret  {rel}:{line}: looks like a {kind}")
    for n, line in enumerate(text.splitlines(), 1):
        m = MARKER.search(line)
        if m:
            hits.append(f"marker  {rel}:{n}: {m.group(0)!r} in {line.strip()[:100]!r}")
    return hits


def scan_tree(tree: Path) -> list[tuple[str, str]]:
    """(relative path, hit line) for everything in the tree."""
    found: list[tuple[str, str]] = []
    for dirpath, dirnames, filenames in os.walk(tree):
        dirnames[:] = [d for d in dirnames if d != ".git"]
        for name in filenames:
            path = Path(dirpath) / name
            rel = path.relative_to(tree).as_posix()
            if SECRET_NAME.search(name) and not SAFE_COPY.search(name):
                found.append((rel, f"file    {rel}: the file name says it holds secrets"))
            if MARKER_NAME.search(rel):
                found.append((rel, f"marker  {rel}: the path labels itself private"))
            if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_BYTES:
                continue
            data = path.read_bytes()
            if b"\0" in data[:8192]:
                continue
            text = data.decode("utf-8", errors="replace")
            found.extend((rel, hit) for hit in scan_text(rel, text))
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tree", type=Path)
    parser.add_argument("--allow", type=Path, default=DEFAULT_ALLOW)
    args = parser.parse_args(argv)
    allowed = read_allowlist(args.allow)
    blocking = 0
    for rel, hit in scan_tree(args.tree):
        if rel in allowed:
            print(f"  allowed {hit}  ({allowed[rel]})")
        else:
            print(f"  {hit}")
            blocking += 1
    if blocking:
        print(f"{blocking} private-material hit(s). Remove them, or add the path to "
              f"{args.allow.name} with the reason it is safe.")
        return 1
    print("  no blocking hits")
    return 0


if __name__ == "__main__":
    sys.exit(main())
