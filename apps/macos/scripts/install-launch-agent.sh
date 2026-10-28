#!/usr/bin/env bash
set -euo pipefail

# NFR-REL-1 (partial): "crash -> auto-restart." This app is still a single monolithic
# process (no capture-daemon split yet — see docs/04-architecture.md), so what's
# implemented here is the whole-app version of that requirement: a LaunchAgent that
# relaunches Whimbrel if it exits abnormally (crash, uncaught exception, killed by a
# signal — any non-zero/abnormal exit), but does NOT relaunch it after a clean exit
# (a deliberate Quit from the menu, which calls NSApplication.terminate and exits 0).
# This is launchd's KeepAlive.SuccessfulExit=false, verified against a throwaway test
# agent during development: it does NOT restart on exit(0), and DOES restart (after
# launchd's own ~10s throttle) on exit(1) — see the PR description for the transcript.

BUNDLE_ID="com.mpointer.kestrel"
DEFAULT_APP_PATH="/Applications/Whimbrel.app"
APP_PATH="${1:-$DEFAULT_APP_PATH}"
EXECUTABLE="$APP_PATH/Contents/MacOS/Whimbrel"
PLIST_PATH="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"

if [ ! -x "$EXECUTABLE" ]; then
  echo "error: no executable found at $EXECUTABLE" >&2
  echo "usage: $0 [/path/to/Whimbrel.app]" >&2
  echo "(the app must be built/copied to that location first — a DerivedData build path won't survive a rebuild)" >&2
  exit 1
fi

mkdir -p "$HOME/Library/LaunchAgents"
cat > "$PLIST_PATH" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$BUNDLE_ID</string>
    <key>ProgramArguments</key>
    <array>
        <string>$EXECUTABLE</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
PLIST

plutil -lint "$PLIST_PATH"

launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"

echo "Installed and loaded $PLIST_PATH"
echo "Whimbrel will now restart automatically if it crashes (not on a deliberate Quit)."
echo "Check status:  launchctl print gui/$(id -u)/$BUNDLE_ID"
echo "Uninstall:     apps/macos/scripts/uninstall-launch-agent.sh"
