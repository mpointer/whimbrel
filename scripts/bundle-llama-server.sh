#!/bin/sh
# Builds the local model helper from a pinned llama.cpp commit and copies it into the app.
# Release builds always get it. Debug builds copy it only if it's already cached.
set -e
TAG=b11877
COMMIT=e321bdad7e8c6ebf5675b641369544dc3fa3a54b
CACHE="$HOME/Library/Caches/Whimbrel/llama-$COMMIT"
BIN="$CACHE/whimbrel-llama-server"

if [ ! -f "$BIN" ]; then
  if [ "$CONFIGURATION" != "Release" ]; then
    echo "Skipping the local model helper in a Debug build (nothing cached)."
    exit 0
  fi
  rm -rf "$CACHE/src"
  mkdir -p "$CACHE"
  git clone --quiet --depth 1 --branch "$TAG" https://github.com/ggml-org/llama.cpp "$CACHE/src"
  [ "$(git -C "$CACHE/src" rev-parse HEAD)" = "$COMMIT" ] || { echo "error: $TAG is not $COMMIT"; exit 1; }
  cmake -S "$CACHE/src" -B "$CACHE/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
    -DGGML_METAL_EMBED_LIBRARY=ON -DLLAMA_OPENSSL=OFF -DLLAMA_CURL=OFF \
    -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_SERVER=ON
  cmake --build "$CACHE/build" --target llama-server -j"$(sysctl -n hw.ncpu)"
  strip -x -o "$BIN" "$CACHE/build/bin/llama-server"
  # A Homebrew library here would crash on every Mac without it.
  if otool -L "$BIN" | grep -q '/opt/homebrew\|/usr/local'; then echo "error: helper links a local library"; exit 1; fi
fi

DEST="$CODESIGNING_FOLDER_PATH/Contents/Resources"
mkdir -p "$DEST"
cp -f "$BIN" "$DEST/whimbrel-llama-server"
