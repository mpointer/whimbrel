#!/usr/bin/env bash
# Creates the Whimbrel release keystore in <dir> (outside the repo). Refuses to overwrite one.
set -euo pipefail

dir="${1:?usage: create-release-keystore.sh <dir>}"
mkdir -p "$dir"
dir="$(cd "$dir" && pwd)"
jks="$dir/whimbrel-release.jks"
props="$dir/keystore.properties"
sha_file="$(cd "$(dirname "$0")/.." && pwd)/release-sha1.txt"

if [[ -e "$jks" || -e "$props" ]]; then
  echo "refusing to overwrite an existing keystore in $dir" >&2
  exit 1
fi

umask 077
chmod 700 "$dir"
# Passed to keytool through the environment so it never appears in argv or output.
export KS_PASS="$(openssl rand -base64 32)"

keytool -genkeypair -alias whimbrel -keyalg RSA -keysize 4096 -validity 10000 \
  -dname "CN=Whimbrel, O=Whimbrel" -keystore "$jks" \
  -storepass:env KS_PASS -keypass:env KS_PASS >/dev/null

printf 'storeFile=%s\nstorePassword=%s\nkeyAlias=whimbrel\nkeyPassword=%s\n' "$jks" "$KS_PASS" "$KS_PASS" > "$props"
chmod 600 "$jks" "$props"

sha1="$(keytool -list -v -alias whimbrel -keystore "$jks" -storepass:env KS_PASS | sed -n "s/^[[:space:]]*SHA1:[[:space:]]*//p")"
[[ -n "$sha1" ]] || { echo "could not read SHA-1" >&2; exit 1; }
echo "$sha1" > "$sha_file"

echo "SHA-1 (paste into the Android OAuth client): $sha1"
echo "export WHIMBREL_KEYSTORE_DIR=$dir"
