#!/usr/bin/env bash
# The one runnable check for scripts/check-version.sh's own logic. No test
# framework -- a handful of assert-style scratch-repo scenarios, matching this
# repo's established convention for small, self-contained checks.
set -euo pipefail
cd "$(dirname "$0")/.."

FAILURES=0
assert_exit() {
    local desc="$1" expected="$2"
    shift 2
    set +e
    "$@" >/tmp/check-version-test-output.$$  2>&1
    local actual=$?
    set -e
    if [ "$actual" -ne "$expected" ]; then
        echo "FAIL: $desc (expected exit $expected, got $actual)" >&2
        cat /tmp/check-version-test-output.$$ >&2
        FAILURES=$((FAILURES + 1))
    else
        echo "ok: $desc"
    fi
    rm -f /tmp/check-version-test-output.$$
}

SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/apps/macos/Whimbrel.xcodeproj" "$SCRATCH/core" "$SCRATCH/scripts"
cp "$(pwd)/scripts/check-version.sh" "$SCRATCH/scripts/check-version.sh"
chmod +x "$SCRATCH/scripts/check-version.sh"

write_agreeing_scratch() {
    local v="$1"
    echo "$v" > "$SCRATCH/VERSION"
    printf 'targets:\n  Whimbrel:\n    settings:\n      base:\n        MARKETING_VERSION: "%s"\n' "$v" > "$SCRATCH/apps/macos/project.yml"
    printf 'MARKETING_VERSION = %s;\nMARKETING_VERSION = %s;\n' "$v" "$v" > "$SCRATCH/apps/macos/Whimbrel.xcodeproj/project.pbxproj"
    printf '[workspace.package]\nversion = "%s"\n' "$v" > "$SCRATCH/core/Cargo.toml"
}

run_check() {
    "$SCRATCH/scripts/check-version.sh" "$@"
}

write_agreeing_scratch "1.2.3"
assert_exit "agreeing sources pass" 0 run_check

write_agreeing_scratch "1.2.3"
echo "1.2.4" > "$SCRATCH/VERSION"
assert_exit "project.yml mismatch fails" 1 run_check

write_agreeing_scratch "1.2.3"
assert_exit "matching --tag passes" 0 run_check --tag v1.2.3

write_agreeing_scratch "1.2.3"
assert_exit "mismatched --tag fails" 1 run_check --tag v1.2.4

if [ "$FAILURES" -ne 0 ]; then
    echo "$FAILURES check(s) failed" >&2
    exit 1
fi
echo "all check-version.sh scenarios passed"
