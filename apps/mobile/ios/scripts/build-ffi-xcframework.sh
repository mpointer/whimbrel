#!/usr/bin/env bash
# Builds core/ffi (the Rust core, static lib `libwhimbrel_core.a`) for iOS and packs it
# with core/ffi/include into apps/mobile/ios/build/WhimbrelCore.xcframework (gitignored).
#
# Slices: aarch64-apple-ios (device) and aarch64-apple-ios-sim (Apple Silicon simulator).
# No x86_64 simulator slice: Intel-Mac simulators are not supported (project.yml excludes
# x86_64 for the simulator SDK to match).
#
# --no-default-features drops `diarize` (sherpa-onnx has no prebuilt ONNX Runtime for iOS).
# RUSTFLAGS=+fp16 is needed because gemm-f16 (via candle, memory crate) emits `fullfp16`
# NEON asm. The device binary therefore needs a FEAT_FP16 chip (A11 and later), which
# the iOS 17 deployment target already guarantees. Same flags as the `ios` CI job.
#
# Usage: build-ffi-xcframework.sh [--debug]   (default: release)
set -euo pipefail

IOS_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$IOS_DIR/../../.." && pwd)"
CORE="$REPO/core"
OUT="$IOS_DIR/build/WhimbrelCore.xcframework"

PROFILE=release
CARGO_PROFILE_FLAG=--release
if [ "${1:-}" = "--debug" ]; then
  PROFILE=debug
  CARGO_PROFILE_FLAG=
fi

export RUSTFLAGS="-C target-feature=+fp16"
# Must match project.yml's IPHONEOS_DEPLOYMENT_TARGET so the linker doesn't warn about
# objects built for a newer iOS than the app.
export IPHONEOS_DEPLOYMENT_TARGET=17.0

TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)
for t in "${TARGETS[@]}"; do
  (cd "$CORE" && cargo build -p ffi --no-default-features --target "$t" $CARGO_PROFILE_FLAG)
done

# Honours a configured build.target-dir / CARGO_TARGET_DIR.
TARGET_DIR="$(cd "$CORE" && cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

ARGS=()
for t in "${TARGETS[@]}"; do
  ARGS+=(-library "$TARGET_DIR/$t/$PROFILE/libwhimbrel_core.a" -headers "$CORE/ffi/include")
done

rm -rf "$OUT"
xcodebuild -create-xcframework "${ARGS[@]}" -output "$OUT"
echo "Built $OUT"
