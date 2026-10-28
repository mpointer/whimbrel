#!/bin/sh
# Waits for the given PID to exit, replaces the running app's own bundle (its
# actual path, not a hardcoded /Applications guess -- a DerivedData debug
# build must not clobber a real /Applications install) with the staged
# update, and relaunches. Invoked by UpdateChecker's "Restart Now" action
# with its own PID, the staged .app's path, and its own bundle path -- never
# run manually.
#
# The staged bundle's own name decides where it is installed: next to the
# running app, under the staged name. The rename release ships Whimbrel.app
# where every earlier zip held Whimbrel.app (docs/superpowers/specs/
# 2026-10-21-kestrel-to-whimbrel-rename-design.md, step 4a); when the names
# match, this is the same in-place swap as before.
set -e

PID="$1"
NEW_APP_PATH="$2"
CURRENT_APP_PATH="$3"
# Tests point this at /usr/bin/true so a fake bundle is never launched.
OPEN="${RELAUNCH_OPEN_CMD:-open}"

while kill -0 "$PID" 2>/dev/null; do
  sleep 0.5
done

TARGET_APP_PATH="$(dirname "$CURRENT_APP_PATH")/$(basename "$NEW_APP_PATH")"

if [ "$TARGET_APP_PATH" != "$CURRENT_APP_PATH" ]; then
  # A renamed bundle: install it beside the old one, and remove the old one only
  # after the copy succeeded. A failed copy leaves the old app exactly as it was.
  rm -rf "$TARGET_APP_PATH"
  if ditto "$NEW_APP_PATH" "$TARGET_APP_PATH"; then
    rm -rf "$CURRENT_APP_PATH" || logger -t whimbrel-relaunch "installed $TARGET_APP_PATH but could not remove $CURRENT_APP_PATH"
    "$OPEN" "$TARGET_APP_PATH"
  else
    logger -t whimbrel-relaunch "ditto failed copying $NEW_APP_PATH to $TARGET_APP_PATH; relaunching $CURRENT_APP_PATH without installing the update"
    rm -rf "$TARGET_APP_PATH"
    "$OPEN" "$CURRENT_APP_PATH"
  fi
  exit 0
fi

# Atomic-ish swap: rename the running app aside first instead of deleting it
# outright. If `ditto` then fails (disk full, staged dir gone, permissions),
# restore the original from the backup instead of leaving nothing installed.
BACKUP_APP_PATH="${CURRENT_APP_PATH}.old"
rm -rf "$BACKUP_APP_PATH"

# Guarded like the ditto step below: a non-admin user without write access to
# CURRENT_APP_PATH's parent (e.g. a shared-Mac /Applications install) makes this
# mv fail. Under `set -e` an unguarded failure here would exit before the
# logger call or the trailing `open`, silently leaving no app running and no
# diagnostic trail -- the exact bug the ditto branch below was already fixed for.
if ! mv "$CURRENT_APP_PATH" "$BACKUP_APP_PATH"; then
  logger -t whimbrel-relaunch "mv failed renaming $CURRENT_APP_PATH aside (permission denied on a non-admin install?); relaunching original in place without installing the update"
  "$OPEN" "$CURRENT_APP_PATH"
  exit 0
fi

if ditto "$NEW_APP_PATH" "$CURRENT_APP_PATH"; then
  rm -rf "$BACKUP_APP_PATH"
else
  logger -t whimbrel-relaunch "ditto failed copying $NEW_APP_PATH to $CURRENT_APP_PATH; restoring original from $BACKUP_APP_PATH"
  rm -rf "$CURRENT_APP_PATH"
  mv "$BACKUP_APP_PATH" "$CURRENT_APP_PATH"
fi

# Always relaunch, whether the swap succeeded (new version) or failed and was
# rolled back (restored original) -- the user should never be left with no
# running app. `log show` picks up the logger call above for diagnosis.
"$OPEN" "$CURRENT_APP_PATH"
