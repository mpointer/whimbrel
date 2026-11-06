#!/usr/bin/env bash
# Build the signed Android APK for a release tag and attach it to the public release.
#
#   apps/mobile/android/scripts/release-apk.sh [--dry-run] vX    # clean tree, HEAD at tag vX, after step 6 of docs/RELEASING.md
#
# Builds and verifies everything, then uploads Whimbrel-<version>.apk to the public mirror
# release for vX, so a phone browser can download it without a GitHub sign-in. --dry-run stops
# before the upload and leaves the APK in app/build/outputs/apk/release/.
#
# Needs: the Android toolchain (source ~/.config/whimbrel-android-env.sh), $WHIMBREL_KEYSTORE_DIR
# with keystore.properties, apps/mobile/android/secrets.properties, and gh logged in.
#
# Environment:
#   PUBLIC_REPO   owner/name of the public repository   (default mpointer/whimbrel)
set -euo pipefail

DRY_RUN=0
TAG=""
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        -*) echo "error: unknown option '$arg' (usage: release-apk.sh [--dry-run] vX)" >&2; exit 1 ;;
        *) TAG="$arg" ;;
    esac
done
[ -n "$TAG" ] || { echo "error: usage: release-apk.sh [--dry-run] vX" >&2; exit 1; }

PUBLIC_REPO="${PUBLIC_REPO:-mpointer/whimbrel}"
ANDROID_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$(git rev-parse --show-toplevel)"

if [ -n "$(git status --porcelain)" ]; then
    echo "error: the tree is not clean; release from a clean checkout at $TAG" >&2
    exit 1
fi
if [ "$(git rev-parse HEAD)" != "$(git rev-parse "$TAG^{commit}" 2>/dev/null || true)" ]; then
    echo "error: HEAD is not at $TAG; check out the tag first" >&2
    exit 1
fi
if [ -z "${WHIMBREL_KEYSTORE_DIR:-}" ] || [ ! -f "$WHIMBREL_KEYSTORE_DIR/keystore.properties" ]; then
    echo "error: ${WHIMBREL_KEYSTORE_DIR:-<WHIMBREL_KEYSTORE_DIR unset>}/keystore.properties is missing; set WHIMBREL_KEYSTORE_DIR to the release keystore directory (apps/mobile/android/README.md)" >&2
    exit 1
fi
if [ ! -f "$ANDROID_DIR/secrets.properties" ]; then
    echo "error: $ANDROID_DIR/secrets.properties is missing; copy secrets.properties.example and fill it in" >&2
    exit 1
fi
if ! gh release view "$TAG" -R "$PUBLIC_REPO" >/dev/null 2>&1; then
    echo "error: $PUBLIC_REPO has no release $TAG; run step 6 first (scripts/publish-public.sh $TAG)" >&2
    exit 1
fi

scripts/check-version.sh --tag "$TAG"
VERSION="${TAG#v}"

echo "==> Building the release APK"
(cd "$ANDROID_DIR" && ./gradlew :app:assembleRelease)
APK_DIR="$ANDROID_DIR/app/build/outputs/apk/release"
APK="$APK_DIR/Whimbrel-$VERSION.apk"
cp "$APK_DIR/app-release.apk" "$APK"

echo "==> Verifying the signature"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
APKSIGNER="$(command -v apksigner || ls "$SDK"/build-tools/*/apksigner 2>/dev/null | sort -V | tail -1 || true)"
[ -n "$APKSIGNER" ] || { echo "error: apksigner not found; set ANDROID_HOME (source ~/.config/whimbrel-android-env.sh)" >&2; exit 1; }
ACTUAL="$("$APKSIGNER" verify --print-certs "$APK" | sed -n 's/.*SHA-1 digest: //p' | head -1 | tr -d ':' | tr 'A-F' 'a-f')"
EXPECTED="$(tr -d ':[:space:]' < "$ANDROID_DIR/release-sha1.txt" | tr 'A-F' 'a-f')"
if [ -z "$ACTUAL" ] || [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "error: APK signing SHA-1 '$ACTUAL' does not match release-sha1.txt '$EXPECTED'; wrong keystore?" >&2
    exit 1
fi

if [ "$DRY_RUN" = 1 ]; then
    echo "==> Dry run: built and verified $APK; not uploading"
    exit 0
fi
echo "==> Uploading to $PUBLIC_REPO $TAG"
gh release upload "$TAG" "$APK" --clobber -R "$PUBLIC_REPO"
echo "==> Done: https://github.com/$PUBLIC_REPO/releases/tag/$TAG"
