#!/usr/bin/env bash
# Copies one platform's gitignored secrets file into this checkout from the
# maintainer's private folder, so a fresh release worktree needs no manual copy.
#
#   scripts/release-secrets.sh macos|ios|android
#
# Source folder: $WHIMBREL_SECRETS_DIR, default ~/.config/whimbrel/secrets/, holding
#   macos-Secrets.xcconfig      -> apps/macos/Secrets.xcconfig
#   ios-Secrets.xcconfig        -> apps/mobile/ios/Secrets.xcconfig
#   android-secrets.properties  -> apps/mobile/android/secrets.properties
#
# An existing file in the checkout is left alone. The contents are never printed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC_DIR="${WHIMBREL_SECRETS_DIR:-$HOME/.config/whimbrel/secrets}"

case "${1:-}" in
    macos)   src="macos-Secrets.xcconfig";     dest="apps/macos/Secrets.xcconfig" ;;
    ios)     src="ios-Secrets.xcconfig";       dest="apps/mobile/ios/Secrets.xcconfig" ;;
    android) src="android-secrets.properties"; dest="apps/mobile/android/secrets.properties" ;;
    *) echo "usage: scripts/release-secrets.sh macos|ios|android" >&2; exit 2 ;;
esac

if [ -f "$ROOT/$dest" ]; then
    echo "==> $dest already present; leaving it."
    exit 0
fi
if [ ! -f "$SRC_DIR/$src" ]; then
    echo "error: $SRC_DIR/$src not found." >&2
    echo "  One-time setup: put the three secrets files in $SRC_DIR (see the header of scripts/release-secrets.sh)," >&2
    echo "  or set WHIMBREL_SECRETS_DIR to where they live." >&2
    exit 1
fi
install -m 600 "$SRC_DIR/$src" "$ROOT/$dest"
echo "==> Copied $src into $dest."
