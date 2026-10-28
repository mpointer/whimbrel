#!/usr/bin/env bash
set -euo pipefail

# Builds, signs, and notarizes a Developer ID Whimbrel.app from the CURRENT
# committed, tagged HEAD, then publishes it as an auto-update. Unlike the old
# release.sh, this script never mutates VERSION/project.yml/Cargo.toml itself
# -- it only validates that they already agree with a clean, tagged commit
# (see apps/macos/scripts/release-prepare.sh for the bump step) and refuses
# to proceed otherwise.
#
# Requires Xcode signed into the paid "Michael Pointer" Developer Program
# account (GP8PFLM3Q3) -- see Xcode > Settings > Accounts. Also requires a
# `notarytool` keychain profile with app-specific-password credentials for
# that same account (one-time setup: `xcrun notarytool store-credentials
# "kestrel-notary" --apple-id <apple-id> --team-id GP8PFLM3Q3`, prompts for
# the app-specific password interactively -- generate one at
# appleid.apple.com if needed). Override the profile name via
# RELEASE_NOTARY_PROFILE if you used a different one. Also requires `gh`
# (GitHub CLI) authenticated, for creating the release.
#
# Export exactly ONCE and notarize/staple that exact artifact. An earlier
# version of this pipeline called `xcodebuild -exportArchive` twice from the
# same archive -- once with destination:upload to submit for notarization,
# once with destination:export to produce a local .app -- on the (wrong)
# assumption both passes would produce byte-identical output. They don't:
# each -exportArchive invocation re-signs independently with its own signing
# timestamp, so the two passes produced different CDHashes. Apple issued a
# ticket for the uploaded binary's hash; `stapler staple` was being run
# against a *different* binary that was never submitted -- hence a 100%
# reproducible "Record not found" every time, regardless of how long you
# waited. Root-caused 2026-08-06 by comparing `codesign -dvvv`'s CDHash
# against `notarytool log`'s ticketContents CDHash.

cd "$(dirname "$0")/../../.."   # repo root (this script lives in apps/macos/scripts/, 3 levels down)

# --- Preflight: refuse to publish anything not backed by a clean, tagged, ---
# --- version-agreeing commit. All checks below run BEFORE any build step. ---

if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree is dirty -- refusing to publish. Commit and tag first (see apps/macos/scripts/release-prepare.sh)." >&2
    exit 1
fi

VERSION=$(tr -d '[:space:]' < VERSION)
TAG="v$VERSION"
if ! git describe --exact-match --match "$TAG" HEAD >/dev/null 2>&1; then
    echo "error: HEAD is not tagged $TAG -- run 'git tag -a $TAG -m \"$TAG\"' after committing the prepared version (apps/macos/scripts/release-prepare.sh)." >&2
    exit 1
fi

scripts/check-version.sh --tag "$TAG"

REMOTE_TAG_COMMIT=$(git ls-remote origin "refs/tags/$TAG^{}" 2>/dev/null | awk '{print $1}')
if [ -z "$REMOTE_TAG_COMMIT" ] || [ "$REMOTE_TAG_COMMIT" != "$(git rev-parse HEAD)" ]; then
    echo "error: tag $TAG is not pushed to origin at this commit -- run 'git push && git push --tags' first." >&2
    exit 1
fi

# notarytool keeps its credentials in the data-protection keychain, which can't be read
# while the Mac is locked ("No Keychain password item found"). Check before the
# ~20-minute build rather than after it.
NOTARY_PROFILE="${RELEASE_NOTARY_PROFILE:-kestrel-notary}"
if ! xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1; then
    echo "error: notarytool can't read keychain profile '$NOTARY_PROFILE'. Unlock the Mac and retry; if it still fails, check 'xcrun notarytool history --keychain-profile $NOTARY_PROFILE' in Terminal." >&2
    exit 1
fi

echo "==> Preflight passed: clean tree, HEAD tagged $TAG, all version sources agree on $VERSION, notary profile readable."

cd apps/macos

# Secrets.xcconfig is gitignored, so a fresh git worktree does NOT inherit the
# one from wherever it was first set up -- each worktree needs its own copy
# or sync silently no-ops at runtime ("Sync isn't configured in this build")
# despite a totally successful, notarized build. Fail loudly here instead of
# shipping a broken binary a second time.
if [ ! -f Secrets.xcconfig ]; then
    echo "error: Secrets.xcconfig not found in $(pwd)." >&2
    echo "  Gitignored -- copy it from another worktree/checkout that has it, or see Secrets.xcconfig.example." >&2
    exit 1
fi

# mcp-servers/google-calendar/node_modules is gitignored, so a fresh git
# worktree/checkout doesn't have it either -- and unlike Secrets.xcconfig,
# there's nothing to copy from another checkout (it's just an npm install).
# XcodeGen's folder-reference resources copy whatever exists on disk at
# build time with no check of their own, so skipping this would silently
# ship a Release .app with Contents/Resources/dist populated but
# Contents/Resources/node_modules empty -- breaking the connector on the
# exact "AirDrop to a second Mac" scenario this script exists for. Install
# rather than just check-and-fail, same self-sufficient pattern as the
# "Build Rust core" prebuild script below. --omit=dev keeps devDependencies
# (vitest, typescript, etc.) out of the signed bundle -- dist/ is prebuilt
# and doesn't need them at runtime.
echo "==> Installing Google Calendar connector dependencies..."
(cd ../../mcp-servers/google-calendar && npm ci --omit=dev)

# Routines Phase B added a second connector, and it is bundled the same way, so it
# needs the same treatment for the same reason. It is NOT optional: the Gmail
# connector's node_modules is copied to Contents/Resources/gmail/node_modules, which
# is the only place Node's resolution walk from gmail/dist/index.js can find its
# `zod`/`@modelcontextprotocol/sdk` (see project.yml's shared `subpath: gmail`).
# Without this step a fresh checkout ships an empty one and send_email fails at
# runtime in the signed build. --omit=dev matters more here than for Calendar,
# because this package's tests live under src/ and a full install would otherwise
# drag vitest and typescript into a notarized bundle.
echo "==> Installing Gmail connector dependencies..."
(cd ../../mcp-servers/gmail && npm ci --omit=dev)

# The Outlook 365 connector is bundled the same way, and needs the same treatment
# for the same reason. Its node_modules is copied to
# Contents/Resources/outlook365/node_modules, which is the only place Node's
# resolution walk from outlook365/dist/index.js can find its
# `zod`/`@modelcontextprotocol/sdk` (see project.yml's shared `subpath: outlook365`).
# Without this step a fresh checkout ships an empty one and Outlook Calendar/Mail
# access fails at runtime in the signed build. --omit=dev keeps this package's
# vitest/typescript dev dependencies out of the notarized bundle.
echo "==> Installing Outlook 365 connector dependencies..."
(cd ../../mcp-servers/outlook365 && npm ci --omit=dev)

# core/memory/models is gitignored, same reasoning as mcp-servers/google-calendar/
# node_modules above -- download it if a fresh checkout doesn't have it yet, so this
# script stays self-sufficient rather than requiring a manual setup step first.
echo "==> Ensuring embedding model files are present..."
mkdir -p ../../core/memory/models
for f in config.json tokenizer.json model.safetensors; do
    if [ ! -f "../../core/memory/models/$f" ]; then
        curl -fL --retry 3 -o "../../core/memory/models/$f" "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/$f"
    fi
done

# XcodeGen's folder-reference resources copy whatever exists on disk at build time
# with no check of their own (same hazard as the Calendar connector's node_modules,
# documented above) -- but this model is load-bearing for every single capture_item
# insert, not just an on-demand connector, so a silently-empty bundle here would
# degrade ambient search continuously. Fail loudly rather than ship it.
for f in config.json tokenizer.json model.safetensors; do
    if [ ! -f "../../core/memory/models/$f" ]; then
        echo "error: ../../core/memory/models/$f still missing after download attempt." >&2
        exit 1
    fi
done

SCRATCH="$(mktemp -d)"
ARCHIVE_PATH="$SCRATCH/Whimbrel.xcarchive"
EXPORT_PATH="$SCRATCH/export"
OUTPUT_ZIP="${1:-$HOME/Desktop/Whimbrel.zip}"
TEAM_ID="GP8PFLM3Q3"

cat > "$SCRATCH/ExportOptions.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key>
	<string>developer-id</string>
	<key>teamID</key>
	<string>$TEAM_ID</string>
	<key>signingStyle</key>
	<string>automatic</string>
</dict>
</plist>
EOF

echo "==> Archiving (Release configuration)..."
xcodebuild archive \
  -project Whimbrel.xcodeproj \
  -scheme Whimbrel \
  -configuration Release \
  -archivePath "$ARCHIVE_PATH"

echo "==> Exporting (single pass -- this exact .app gets notarized and stapled)..."
xcodebuild -exportArchive \
  -archivePath "$ARCHIVE_PATH" \
  -exportPath "$EXPORT_PATH" \
  -exportOptionsPlist "$SCRATCH/ExportOptions.plist"

APP_PATH="$EXPORT_PATH/Whimbrel.app"
NOTARIZE_ZIP="$SCRATCH/Whimbrel-for-notarization.zip"
ditto -c -k --keepParent "$APP_PATH" "$NOTARIZE_ZIP"

echo "==> Submitting for notarization and waiting for a result..."
xcrun notarytool submit "$NOTARIZE_ZIP" --keychain-profile "$NOTARY_PROFILE" --wait

echo "==> Stapling ticket onto the exact exported .app..."
xcrun stapler staple "$APP_PATH"

echo "==> Verifying Gatekeeper acceptance..."
spctl -a -vv --type execute "$APP_PATH"

echo "==> Packaging as $OUTPUT_ZIP..."
rm -f "$OUTPUT_ZIP"
ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" "$OUTPUT_ZIP"

echo ""
echo "Done: $OUTPUT_ZIP is a Developer ID-signed, notarized, stapled build."

echo ""
echo "==> Verifying packaged version matches $VERSION before publishing..."
BUILT_VERSION=$(defaults read "$APP_PATH/Contents/Info.plist" CFBundleShortVersionString)
if [ "$BUILT_VERSION" != "$VERSION" ]; then
    echo "error: built app reports version '$BUILT_VERSION', expected '$VERSION' -- refusing to publish a mismatched build." >&2
    exit 1
fi

SHA256=$(shasum -a 256 "$OUTPUT_ZIP" | awk '{print $1}')
COMMIT=$(git -C ../.. rev-parse HEAD)
PUBLISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
UPDATE_ZIP_NAME="Whimbrel-$VERSION.zip"

echo "==> Creating GitHub Release $TAG..."
# Re-runnable: a release created earlier (e.g. by hand) is kept.
(cd ../.. && { gh release view "$TAG" >/dev/null 2>&1 || gh release create "$TAG" --title "$TAG" --generate-notes; })
RELEASE_NOTES_URL="https://github.com/mpointer/kestrel/releases/tag/$TAG"

echo "==> Uploading $UPDATE_ZIP_NAME to the updates bucket..."
(cd ../../server-updates && npx wrangler r2 object put "kestrel-updates/$UPDATE_ZIP_NAME" --file "$OUTPUT_ZIP" --remote)

VERSION_JSON=$(cat <<JSON
{"version": "$VERSION", "url": "https://updates.whimbrelapp.com/$UPDATE_ZIP_NAME", "sha256": "$SHA256", "commit": "$COMMIT", "tag": "$TAG", "published_at": "$PUBLISHED_AT", "release_notes_url": "$RELEASE_NOTES_URL"}
JSON
)
echo "$VERSION_JSON" > "$SCRATCH/version.json"
echo "==> Publishing version.json..."
(cd ../../server-updates && npx wrangler r2 object put "kestrel-updates/version.json" --file "$SCRATCH/version.json" --remote)

echo "==> Published $VERSION ($TAG, sha256 $SHA256)."

echo ""
echo "Reminder: the whisper transcription model and the two diarization models are"
echo "not bundled -- see core/transcribe/README.md and core/diarize/README.md, or"
echo "use the in-app first-run model downloader once it's built. The embedding model"
echo "(core/memory/README.md) IS bundled -- this script just downloaded/verified it above."

rm -rf "$SCRATCH"
