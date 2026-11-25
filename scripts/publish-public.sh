#!/usr/bin/env bash
# Publish a release of this tree to the public repository.
#
# Development happens in the private repository (mpointer/kestrel), which runs the
# self-hosted CI runners. The public repository (mpointer/whimbrel) is a mirror of
# main at each release: one commit per release carrying the whole tree except the
# paths listed in scripts/public-exclude.txt (design plans and specs, redesign research,
# pricing and competitive analysis, translation review sheets), tagged with the same
# version, with the GitHub release notes copied across. No private history is pushed.
#
#   scripts/publish-public.sh vX [--no-phones] [--yes] [--dry-run]
#
# Run it from a clean tree with HEAD at tag vX. Before anything is pushed it prints a
# review: the top-level and second-level paths that are new or gone since the last
# public release, every file the exclude list kept out, and any file that looks like
# private material (scripts/public-review.py). Then it asks you to type the tag.
#
#   --no-phones  skip scripts/release-phones.sh at the end
#   --yes        no prompt (for non-interactive runs); a private-material hit still stops it
#   --dry-run    do everything up to the public commit, then stop: no commit, tag, push,
#                GitHub release or phone builds. Safe to point at the real public repo.
#
# Environment:
#   PUBLIC_REPO   owner/name of the public repository   (default mpointer/whimbrel)
#   PUBLIC_URL    git URL to push to                     (default https://github.com/$PUBLIC_REPO)
#   SKIP_RELEASE  set to 1 to skip the GitHub release    (default: create it from the private one)
#
# Re-runnable: if the public repository already has the tag, nothing is pushed.
set -euo pipefail

usage="usage: scripts/publish-public.sh vX [--no-phones] [--yes] [--dry-run]"
TAG=""
PHONES=1
YES=0
DRY_RUN=0
for arg in "$@"; do
    case "$arg" in
    --no-phones) PHONES=0 ;;
    --yes) YES=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -*) echo "$usage" >&2; exit 2 ;;
    *)
        if [ -n "$TAG" ]; then echo "$usage" >&2; exit 2; fi
        TAG="$arg"
        ;;
    esac
done
[ -n "$TAG" ] || { echo "$usage" >&2; exit 2; }
PUBLIC_REPO="${PUBLIC_REPO:-mpointer/whimbrel}"
PUBLIC_URL="${PUBLIC_URL:-https://github.com/$PUBLIC_REPO}"
PRIVATE_REPO="mpointer/kestrel"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

if [ -n "$(git status --porcelain)" ]; then
    echo "error: the tree is not clean; publish from a clean checkout at $TAG" >&2
    exit 1
fi
if [ "$(git rev-parse HEAD)" != "$(git rev-parse "$TAG^{commit}" 2>/dev/null || true)" ]; then
    echo "error: HEAD is not at $TAG; check out the tag first" >&2
    exit 1
fi
VERSION="${TAG#v}"
if [ "$(cat VERSION)" != "$VERSION" ]; then
    echo "error: VERSION says $(cat VERSION), tag says $VERSION" >&2
    exit 1
fi
PRIVATE_SHA="$(git rev-parse --short=12 HEAD)"

# The leave-out list, one path per line; # starts a comment. "? path" may be absent.
EXCLUDE_FILE="$ROOT/scripts/public-exclude.txt"
EXCLUDE=()
OPTIONAL=" "
while IFS= read -r line || [ -n "$line" ]; do
    line="${line%%#*}"
    line="$(printf '%s' "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e 's#/*$##')"
    case "$line" in
    "") continue ;;
    "? "*)
        line="$(printf '%s' "${line#? }" | sed -e 's/^[[:space:]]*//')"
        OPTIONAL="$OPTIONAL$line "
        ;;
    esac
    EXCLUDE+=("$line")
done < "$EXCLUDE_FILE"
if [ "${#EXCLUDE[@]}" -eq 0 ]; then
    echo "error: $EXCLUDE_FILE lists nothing; refusing to publish everything" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Exporting $TAG (without the ${#EXCLUDE[@]} paths in scripts/public-exclude.txt)"
mkdir -p "$WORK/tree"
git archive --format=tar "$TAG" | tar -x -C "$WORK/tree"
missing=0
: > "$WORK/excluded.txt"
for path in "${EXCLUDE[@]}"; do
    if [ ! -e "$WORK/tree/$path" ]; then
        case "$OPTIONAL" in
        *" $path "*) echo "    $path is not in $TAG yet (marked ? in the list)"; continue ;;
        esac
        echo "error: scripts/public-exclude.txt lists $path, which is not in $TAG." >&2
        missing=1
        continue
    fi
    (cd "$WORK/tree" && find "$path" -type f) >> "$WORK/excluded.txt"
    rm -rf "${WORK:?}/tree/$path"
done
if [ "$missing" = "1" ]; then
    echo "       Renamed or deleted? Update scripts/public-exclude.txt so the document cannot" >&2
    echo "       go out under its new name." >&2
    exit 1
fi

echo "==> Fetching $PUBLIC_URL"
git clone --quiet --depth 1 "$PUBLIC_URL" "$WORK/public" 2>/dev/null || git clone --quiet "$PUBLIC_URL" "$WORK/public"
cd "$WORK/public"
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
    echo "==> $PUBLIC_REPO already has $TAG; nothing to publish"
    exit 0
fi
git checkout --quiet -B main 2>/dev/null || true
: > "$WORK/before.txt"
LAST="nothing published yet"
if git rev-parse --quiet --verify HEAD >/dev/null; then
    git ls-tree -r --name-only HEAD > "$WORK/before.txt"
    LAST="$(git log -1 --format=%s HEAD)"
    git rm -rq --quiet . >/dev/null 2>&1 || true
fi
# Everything the export produced, including dotfiles, over the emptied tree.
cp -a "$WORK/tree/." .
git add -A
git ls-files > "$WORK/after.txt"

# Review: what this release changes in the public tree, before anything leaves.
prefixes() { awk -F/ '{ print $1; if (NF >= 2) print $1 "/" $2 }' "$1" | sort -u; }
prefixes "$WORK/before.txt" > "$WORK/before.prefixes"
prefixes "$WORK/after.txt" > "$WORK/after.prefixes"
echo
echo "==> Review against the public main ($LAST)"
# The long list first, so the short sections end up next to the prompt.
echo "--- Files kept out by scripts/public-exclude.txt:"
sort "$WORK/excluded.txt" | sed 's/^/  x /' | grep . || echo "  none"
echo "--- Kept out, per listed path:"
for path in "${EXCLUDE[@]}"; do
    printf '  %5d  %s\n' "$(grep -c -e "^$path/" -e "^$path\$" "$WORK/excluded.txt" || true)" "$path"
done
echo "--- New top-level and second-level paths:"
comm -13 "$WORK/before.prefixes" "$WORK/after.prefixes" | sed 's/^/  + /' | grep . || echo "  none"
echo "--- Deleted top-level and second-level paths:"
comm -23 "$WORK/before.prefixes" "$WORK/after.prefixes" | sed 's/^/  - /' | grep . || echo "  none"
leaked=0
for path in "${EXCLUDE[@]}"; do
    if [ -n "$(git ls-files -- "$path")" ]; then
        echo "error: $path is staged for the public commit although it is excluded" >&2
        leaked=1
    fi
done
[ "$leaked" = "0" ] || exit 1

echo "--- Possible private material (scripts/public-review.py):"
if ! python3 -I "$ROOT/scripts/public-review.py" "$WORK/tree" --allow "$ROOT/scripts/public-review-allow.txt"; then
    echo "error: private material in the export; nothing was pushed" >&2
    exit 1
fi
echo

if [ "$DRY_RUN" = "1" ]; then
    echo "==> Dry run: the review above is what $TAG would publish. Nothing committed, tagged or pushed."
    exit 0
fi
if [ "$YES" = "0" ]; then
    if [ ! -t 0 ]; then
        echo "error: no terminal to confirm on; pass --yes for a non-interactive run. Nothing was pushed." >&2
        exit 1
    fi
    printf 'Type %s to publish it to %s: ' "$TAG" "$PUBLIC_REPO"
    read -r answer
    if [ "$answer" != "$TAG" ]; then
        echo "==> Not confirmed; nothing was pushed"
        exit 1
    fi
fi

if git diff --cached --quiet; then
    echo "==> Nothing changed since the last publish; tagging the current commit"
else
    git -c user.name="${GIT_AUTHOR_NAME:-$(git -C "$ROOT" config user.name)}" \
        -c user.email="${GIT_AUTHOR_EMAIL:-$(git -C "$ROOT" config user.email)}" \
        commit --quiet -m "Whimbrel $VERSION" -m "Published from $PRIVATE_REPO at $PRIVATE_SHA (tag $TAG)."
fi
git tag -a "$TAG" -m "$TAG"

echo "==> Pushing main and $TAG to $PUBLIC_REPO"
git push --quiet origin main "refs/tags/$TAG"

if [ "${SKIP_RELEASE:-0}" = "1" ]; then
    echo "==> Skipping the GitHub release (SKIP_RELEASE=1)"
    exit 0
fi
echo "==> Creating the GitHub release on $PUBLIC_REPO"
if gh release view "$TAG" --repo "$PUBLIC_REPO" >/dev/null 2>&1; then
    echo "    release exists; kept"
elif gh release view "$TAG" --repo "$PRIVATE_REPO" --json body -q .body > "$WORK/notes.raw" 2>/dev/null && [ -s "$WORK/notes.raw" ]; then
    # The private notes link pull requests and a compare view that are not readable from
    # the mirror; keep the titles, drop the links.
    sed -E -e "s# by @[A-Za-z0-9_-]+ in https://github.com/$PRIVATE_REPO/pull/[0-9]+##" \
           -e "s#https://github.com/$PRIVATE_REPO/pull/([0-9]+)#\\1#g" \
           -e "\#https://github.com/$PRIVATE_REPO/compare/#d" \
           "$WORK/notes.raw" > "$WORK/notes.md"
    gh release create "$TAG" --repo "$PUBLIC_REPO" --title "$TAG" --notes-file "$WORK/notes.md" >/dev/null
    echo "    created with the notes of the $PRIVATE_REPO release (private links stripped)"
else
    gh release create "$TAG" --repo "$PUBLIC_REPO" --title "$TAG" --notes "Whimbrel $VERSION." >/dev/null
    echo "    created (no private release notes to copy)"
fi
echo "==> Done: https://github.com/$PUBLIC_REPO/releases/tag/$TAG"

# The phone builds go out with the mirror, so the release page carries its APK.
if [ "$PHONES" = "1" ]; then
    cd "$ROOT"
    scripts/release-phones.sh "$TAG"
else
    echo "==> Skipping the phone builds (--no-phones); run scripts/release-phones.sh $TAG later."
fi
