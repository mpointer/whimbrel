#!/usr/bin/env bash
set -euo pipefail

BUNDLE_ID="com.mpointer.kestrel"
PLIST_PATH="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"

launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
rm -f "$PLIST_PATH"

echo "Removed $PLIST_PATH — Whimbrel will no longer auto-restart on crash."
