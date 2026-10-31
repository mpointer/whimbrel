#!/bin/bash
# Export the English source strings of the Mac or iPhone app as an .xcloc.
# Usage: scripts/l10n/export-apple.sh <macos|ios> <outdir>
# Prints the path of <outdir>/en.xcloc (consumed by translate.py extract --apple).
# -exportLocalizations rewrites the app's .xcstrings in place; this script puts them back, so
# translate.py extract --apple is the only writer and check --apple compares against the
# catalog as it was before the export.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
case "${1:-}" in
  macos) project="$root/apps/macos/Whimbrel.xcodeproj"; dest="platform=macOS"; catalogs="$root/apps/macos/Whimbrel" ;;
  ios) project="$root/apps/mobile/ios/Whimbrel.xcodeproj"; dest="generic/platform=iOS Simulator"; catalogs="$root/apps/mobile/ios/Whimbrel" ;;
  *) echo "usage: $0 <macos|ios> <outdir>" >&2; exit 2 ;;
esac
out="${2:?usage: $0 <macos|ios> <outdir>}"

mkdir -p "$out"
# -exportLocalizations builds the app but skips its script phases, so the Mac project's Rust
# library (linked from core/target/release) must already be built and current.
if [ "$1" = macos ]; then
  (cd "$root/core" && CARGO_TARGET_DIR="$root/core/target" cargo build --release -p ffi >&2)
fi
saved="$(mktemp -d)"
cp "$catalogs"/*.xcstrings "$saved/"
trap 'cp "$saved"/*.xcstrings "$catalogs/"; rm -rf "$saved"' EXIT
xcodebuild -exportLocalizations -project "$project" -localizationPath "$out" -exportLanguage en \
  -destination "$dest" CODE_SIGNING_ALLOWED=NO >&2
echo "$out/en.xcloc"
