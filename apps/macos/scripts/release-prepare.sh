#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."   # repo root (this script lives in apps/macos/scripts/)

# Prepares a new release version: bumps every tracked version source,
# regenerates the Xcode project, and verifies they all agree -- then STOPS.
# It never builds, signs, commits, or tags anything itself. Every
# irreversible or shared-state action (commit, tag) is a command the
# operator runs and sees, matching docs/superpowers/specs/2026-08-18-
# release-integrity-design.md's split between "prepare" (mutate + stop) and
# "publish" (refuse unless already committed/tagged).

NEW_VERSION="${1:-}"
if [ -z "$NEW_VERSION" ]; then
    echo "usage: apps/macos/scripts/release-prepare.sh <version>  (e.g. 0.3.12)" >&2
    exit 1
fi
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: '$NEW_VERSION' is not a valid semver (expected e.g. 0.3.12)." >&2
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree is dirty -- commit or stash unrelated changes first. release-prepare.sh refuses to bump a version on top of them." >&2
    exit 1
fi

CURRENT_VERSION=$(tr -d '[:space:]' < VERSION)
IFS='.' read -r CUR_MAJOR CUR_MINOR CUR_PATCH <<< "$CURRENT_VERSION"
IFS='.' read -r NEW_MAJOR NEW_MINOR NEW_PATCH <<< "$NEW_VERSION"
if [ "$NEW_MAJOR" -lt "$CUR_MAJOR" ] || \
   { [ "$NEW_MAJOR" -eq "$CUR_MAJOR" ] && [ "$NEW_MINOR" -lt "$CUR_MINOR" ]; } || \
   { [ "$NEW_MAJOR" -eq "$CUR_MAJOR" ] && [ "$NEW_MINOR" -eq "$CUR_MINOR" ] && [ "$NEW_PATCH" -le "$CUR_PATCH" ]; }; then
    echo "error: $NEW_VERSION is not strictly newer than the current VERSION ($CURRENT_VERSION) -- nothing to prepare." >&2
    exit 1
fi

echo "==> Bumping VERSION: $CURRENT_VERSION -> $NEW_VERSION"
echo "$NEW_VERSION" > VERSION

echo "==> Bumping apps/macos/project.yml's MARKETING_VERSION: $CURRENT_VERSION -> $NEW_VERSION"
sed -i '' "s/MARKETING_VERSION: \"$CURRENT_VERSION\"/MARKETING_VERSION: \"$NEW_VERSION\"/" apps/macos/project.yml

echo "==> Bumping apps/mobile/ios/project.yml's MARKETING_VERSION: $CURRENT_VERSION -> $NEW_VERSION"
sed -i '' "s/MARKETING_VERSION: \"$CURRENT_VERSION\"/MARKETING_VERSION: \"$NEW_VERSION\"/" apps/mobile/ios/project.yml

# core/Cargo.toml's workspace version is what whimbrel_core_version() (Settings,
# CoreSelfTest) reports -- kept in lockstep with VERSION so it can't drift the
# way it did pre-0.3.5 (stuck at 0.2.0 across several app releases).
echo "==> Bumping core/Cargo.toml version: $CURRENT_VERSION -> $NEW_VERSION"
sed -i '' "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" core/Cargo.toml

echo "==> Regenerating the Mac and iPhone Xcode projects (xcodegen generate)..."
(cd apps/macos && xcodegen generate)
(cd apps/mobile/ios && xcodegen generate)

echo "==> Updating core/Cargo.lock for the version bump..."
(cd core && cargo check --workspace >/dev/null)

echo "==> Verifying all version sources now agree..."
scripts/check-version.sh

echo ""
echo "Prepared $NEW_VERSION. Review the diff above (git status / git diff), then:"
echo "  git add VERSION apps/macos/project.yml apps/macos/Whimbrel.xcodeproj/project.pbxproj apps/mobile/ios/project.yml apps/mobile/ios/Whimbrel.xcodeproj/project.pbxproj core/Cargo.toml core/Cargo.lock"
echo "  git commit -m \"release: prepare $NEW_VERSION\""
echo "  git tag -a v$NEW_VERSION -m \"v$NEW_VERSION\""
echo ""
echo "Then push the commit and tag, and run apps/macos/scripts/release-publish.sh"
echo "when ready to build/sign/notarize/publish."
