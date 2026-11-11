#!/usr/bin/env bash
# Publishes both phone builds for a release tag: the TestFlight upload, then the
# Android APK on the public release. scripts/publish-public.sh runs it last, so a
# release needs no separate phone step.
#
#   scripts/release-phones.sh vX
#
# Loads the maintainer's release environment (App Store Connect key, Android keystore)
# from $WHIMBREL_RELEASE_ENV, default ~/.config/whimbrel/release-env.sh, without
# printing it. Each phone script copies its secrets file in itself
# (scripts/release-secrets.sh). Both run even if the first fails; the exit status is
# non-zero if either did.
set -euo pipefail

TAG="${1:?usage: scripts/release-phones.sh vX}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

ENV_FILE="${WHIMBREL_RELEASE_ENV:-$HOME/.config/whimbrel/release-env.sh}"
if [ ! -f "$ENV_FILE" ]; then
    echo "error: $ENV_FILE not found; it sets the App Store Connect key and the Android keystore folder." >&2
    exit 1
fi
set +x
# shellcheck disable=SC1090
source "$ENV_FILE"

status=0
echo "==> iPhone: TestFlight upload for $TAG"
if ! apps/mobile/ios/scripts/testflight-upload.sh; then
    echo "error: the TestFlight upload failed; the Android upload still runs." >&2
    status=1
fi
echo "==> Android: APK for $TAG"
if ! apps/mobile/android/scripts/release-apk.sh "$TAG"; then
    echo "error: the Android APK upload failed." >&2
    status=1
fi
[ "$status" -eq 0 ] && echo "==> Phones done for $TAG"
exit "$status"
