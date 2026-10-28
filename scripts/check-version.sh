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
CARGO_VALUE=$(grep -E '^version = "[0-9]+\.[0-9]+\.[0-9]+"' core/Cargo.toml | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
PBXPROJ_VALUES=$(grep -oE 'MARKETING_VERSION = [0-9]+\.[0-9]+\.[0-9]+;' apps/macos/Whimbrel.xcodeproj/project.pbxproj | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | sort -u || true)

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
check_one "core/Cargo.toml's version" "$CARGO_VALUE"

PBXPROJ_COUNT=$(echo "$PBXPROJ_VALUES" | grep -c . || true)
if [ "$PBXPROJ_COUNT" -eq 0 ]; then
    echo "error: could not read any MARKETING_VERSION from apps/macos/Whimbrel.xcodeproj/project.pbxproj" >&2
    FAIL=1
elif [ "$PBXPROJ_COUNT" -ne 1 ]; then
    echo "error: apps/macos/Whimbrel.xcodeproj/project.pbxproj has disagreeing MARKETING_VERSION entries across build configs: $(echo "$PBXPROJ_VALUES" | tr '\n' ' ')" >&2
    FAIL=1
elif [ "$PBXPROJ_VALUES" != "$VERSION_FILE_VALUE" ]; then
    echo "error: apps/macos/Whimbrel.xcodeproj/project.pbxproj reports '$PBXPROJ_VALUES', VERSION says '$VERSION_FILE_VALUE'" >&2
    FAIL=1
fi

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
