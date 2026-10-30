#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

EXPECTED_TAG=""
if [ "${1:-}" = "--tag" ]; then
    EXPECTED_TAG="${2:-}"
    if [ -z "$EXPECTED_TAG" ]; then
        echo "error: --tag requires a value (e.g. --tag v0.3.12)" >&2
        exit 1
    fi
elif [ -n "${1:-}" ]; then
    echo "error: unknown argument '$1' (expected --tag vX.Y.Z or no arguments)" >&2
    exit 1
fi

if [ ! -f VERSION ]; then
    echo "error: VERSION file not found at repo root" >&2
    exit 1
fi
VERSION_FILE_VALUE=$(tr -d '[:space:]' < VERSION)
if [ -z "$VERSION_FILE_VALUE" ]; then
    echo "error: VERSION file is empty" >&2
    exit 1
fi

PROJECT_YML_VALUE=$(grep -oE 'MARKETING_VERSION: "[0-9]+\.[0-9]+\.[0-9]+"' apps/macos/project.yml | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
IOS_PROJECT_YML_VALUE=$(grep -oE 'MARKETING_VERSION: "[0-9]+\.[0-9]+\.[0-9]+"' apps/mobile/ios/project.yml | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
CARGO_VALUE=$(grep -E '^version = "[0-9]+\.[0-9]+\.[0-9]+"' core/Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)

FAIL=0

check_one() {
    local name="$1"
    local value="$2"
    if [ -z "$value" ]; then
        echo "error: could not read a version from $name" >&2
        FAIL=1
    elif [ "$value" != "$VERSION_FILE_VALUE" ]; then
        echo "error: $name reports '$value', VERSION says '$VERSION_FILE_VALUE'" >&2
        FAIL=1
    fi
}

check_one "apps/macos/project.yml's MARKETING_VERSION" "$PROJECT_YML_VALUE"
check_one "apps/mobile/ios/project.yml's MARKETING_VERSION" "$IOS_PROJECT_YML_VALUE"
check_one "core/Cargo.toml's version" "$CARGO_VALUE"

# Every build config in a generated project must agree, and with VERSION.
check_pbxproj() {
    local pbx="$1"
    local values count
    values=$(grep -oE 'MARKETING_VERSION = [0-9]+\.[0-9]+\.[0-9]+;' "$pbx" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | sort -u || true)
    count=$(echo "$values" | grep -c . || true)
    if [ "$count" -eq 0 ]; then
        echo "error: could not read any MARKETING_VERSION from $pbx" >&2
        FAIL=1
    elif [ "$count" -ne 1 ]; then
        echo "error: $pbx has disagreeing MARKETING_VERSION entries across build configs: $(echo "$values" | tr '\n' ' ')" >&2
        FAIL=1
    elif [ "$values" != "$VERSION_FILE_VALUE" ]; then
        echo "error: $pbx reports '$values', VERSION says '$VERSION_FILE_VALUE'" >&2
        FAIL=1
    fi
}

check_pbxproj apps/macos/Whimbrel.xcodeproj/project.pbxproj
check_pbxproj apps/mobile/ios/Whimbrel.xcodeproj/project.pbxproj

if [ -n "$EXPECTED_TAG" ]; then
    TAG_VERSION="${EXPECTED_TAG#v}"
    if [ "$TAG_VERSION" != "$VERSION_FILE_VALUE" ]; then
        echo "error: tag '$EXPECTED_TAG' does not match VERSION '$VERSION_FILE_VALUE'" >&2
        FAIL=1
    fi
fi

if [ "$FAIL" -ne 0 ]; then
    exit 1
fi

echo "ok: all version sources agree on $VERSION_FILE_VALUE"
