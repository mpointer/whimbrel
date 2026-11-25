#!/bin/bash
# Apply a reviewed translation sheet. Only rows with x in the "ok" column are applied (as "translated");
# every other row is dropped first, so a rejected translation never becomes "translated".
# Usage: scripts/l10n/apply-reviewed.sh <folder of review-<lang>.csv files>
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
dir="${1:?usage: $0 <folder of review-<lang>.csv files>}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

status=0
shopt -s nullglob
sheets=("$dir"/review-*.csv)
[ ${#sheets[@]} -gt 0 ] || { echo "no review-<lang>.csv in $dir" >&2; exit 2; }
for sheet in "${sheets[@]}"; do
  name="$(basename "$sheet")"
  lang="${name#review-}"
  lang="${lang%.csv}"
  kept="$(python3 -I - "$sheet" "$tmp/$name" <<'EOF'
import csv, sys
src, out = sys.argv[1], sys.argv[2]
with open(src, newline="", encoding="utf-8") as f:
    reader = csv.DictReader(f)
    if "ok" not in (reader.fieldnames or []):
        sys.exit(f"{src}: no ok column")
    rows = [r for r in reader if r["ok"].strip().lower() == "x"]
    fields = reader.fieldnames
with open(out, "w", newline="", encoding="utf-8") as f:
    w = csv.DictWriter(f, fieldnames=fields)
    w.writeheader()
    w.writerows(rows)
print(len(rows))
EOF
)"
  echo "$lang: $kept row(s) marked ok"
  if [ "$kept" -gt 0 ]; then
    python3 -I "$here/translate.py" apply "$tmp/$name" --lang "$lang" || status=1
  fi
done
exit $status
