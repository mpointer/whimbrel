#!/usr/bin/env bash
set -euo pipefail

# Builds the iPhone app from the current committed HEAD and uploads it to App Store
# Connect for TestFlight. Modelled on apps/macos/scripts/release-publish.sh: preflight
# first, archive once, export once (the export is the upload).
#
# Signing: automatic, team GP8PFLM3Q3, via the same App Store Connect API key that
# uploads, so it works without an Xcode account session too.
#
# Credentials (none live in the repo; one-time setup in apps/mobile/ios/README.md,
# "TestFlight"):
#   ASC_KEY_ID      the API key's Key ID
#   ASC_ISSUER_ID   the team's Issuer ID
#   ASC_KEY_PATH    optional; default ~/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8
#
# Build number (CFBundleVersion): the UTC time of the run as YYYYMMDD.HHMM (HHMM without
# leading zeros, as each part must be a plain integer), passed as
# CURRENT_PROJECT_VERSION at archive time. It increases with every upload and needs no
# commit. Two runs in the same minute collide and App Store Connect rejects the second.

cd "$(dirname "$0")/../../../.."   # repo root (this script lives in apps/mobile/ios/scripts/)

TEAM_ID="GP8PFLM3Q3"
BUNDLE_ID="com.mpointer.whimbrel.mobile"

# --- Preflight: everything that can fail fast runs before the build. ---

if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree is dirty -- refusing to upload a build that no commit describes." >&2
    exit 1
fi

scripts/check-version.sh
VERSION=$(tr -d '[:space:]' < VERSION)

: "${ASC_KEY_ID:?set ASC_KEY_ID to the App Store Connect API key ID (see apps/mobile/ios/README.md, TestFlight)}"
: "${ASC_ISSUER_ID:?set ASC_ISSUER_ID to the App Store Connect issuer ID (see apps/mobile/ios/README.md, TestFlight)}"
ASC_KEY_PATH="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8}"
if [ ! -f "$ASC_KEY_PATH" ]; then
    echo "error: API key file not found at $ASC_KEY_PATH (set ASC_KEY_PATH if it lives elsewhere)." >&2
    exit 1
fi
AUTH=(-allowProvisioningUpdates
      -authenticationKeyPath "$ASC_KEY_PATH"
      -authenticationKeyID "$ASC_KEY_ID"
      -authenticationKeyIssuerID "$ASC_ISSUER_ID")

cd apps/mobile/ios

# Secrets.xcconfig is gitignored, so a fresh worktree has none, and without it the app
# builds fine but says Google sign-in is not configured. Same trap as the Mac script.
if [ ! -f Secrets.xcconfig ]; then
    echo "error: Secrets.xcconfig not found in $(pwd). Copy it from another checkout or from Secrets.xcconfig.example." >&2
    exit 1
fi
if grep -qE '^GOOGLE_IOS_CLIENT_ID *= *(REPLACE_ME)? *$' Secrets.xcconfig; then
    echo "error: GOOGLE_IOS_CLIENT_ID in Secrets.xcconfig is not set." >&2
    exit 1
fi

NOW=$(date -u +%Y%m%d%H%M)
BUILD_NUMBER="${NOW:0:8}.$((10#${NOW:8:4}))"   # HHMM without leading zeros: 20261030.911
echo "==> Preflight passed: clean tree at $(git rev-parse --short HEAD), version $VERSION, build $BUILD_NUMBER."

echo "==> Building the Rust core (release xcframework)..."
scripts/build-ffi-xcframework.sh

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
ARCHIVE_PATH="$SCRATCH/Whimbrel.xcarchive"

cat > "$SCRATCH/ExportOptions.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key>
	<string>app-store-connect</string>
	<key>destination</key>
	<string>upload</string>
	<key>teamID</key>
	<string>$TEAM_ID</string>
	<key>signingStyle</key>
	<string>automatic</string>
	<key>uploadSymbols</key>
	<true/>
</dict>
</plist>
EOF

echo "==> Archiving (Release, generic iOS device)..."
xcodebuild archive \
  -project Whimbrel.xcodeproj \
  -scheme Whimbrel \
  -configuration Release \
  -destination 'generic/platform=iOS' \
  -archivePath "$ARCHIVE_PATH" \
  CODE_SIGN_STYLE=Automatic \
  DEVELOPMENT_TEAM="$TEAM_ID" \
  CURRENT_PROJECT_VERSION="$BUILD_NUMBER" \
  "${AUTH[@]}"

APP_PLIST="$ARCHIVE_PATH/Products/Applications/Whimbrel.app/Info.plist"
BUILT_VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_PLIST")
BUILT_BUILD=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP_PLIST")
BUILT_ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP_PLIST")
if [ "$BUILT_VERSION" != "$VERSION" ] || [ "$BUILT_BUILD" != "$BUILD_NUMBER" ] || [ "$BUILT_ID" != "$BUNDLE_ID" ]; then
    echo "error: archive reports $BUILT_ID $BUILT_VERSION ($BUILT_BUILD), expected $BUNDLE_ID $VERSION ($BUILD_NUMBER) -- refusing to upload." >&2
    echo "  A PRODUCT_BUNDLE_IDENTIFIER line in Secrets.xcconfig overrides Base.xcconfig's; remove it." >&2
    exit 1
fi

echo "==> Exporting and uploading to App Store Connect..."
xcodebuild -exportArchive \
  -archivePath "$ARCHIVE_PATH" \
  -exportPath "$SCRATCH/export" \
  -exportOptionsPlist "$SCRATCH/ExportOptions.plist" \
  "${AUTH[@]}"

echo ""
echo "Uploaded Whimbrel $VERSION ($BUILD_NUMBER). It appears in App Store Connect > TestFlight"
echo "after processing, usually 5-30 minutes."
