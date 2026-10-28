#!/usr/bin/env bash
# Publish a release of this tree to the public repository.
#
# Development happens in the private repository (mpointer/kestrel), which runs the
# self-hosted CI runners. The public repository (mpointer/whimbrel) is a mirror of
# main at each release: one commit per release carrying the whole tree except
# docs/superpowers/ (design plans and specs), tagged with the same version, with
# the GitHub release notes copied across. No private history is pushed.
#
#   scripts/publish-public.sh vX          # from a clean tree with HEAD at tag vX
#
# Environment:
#   PUBLIC_REPO   owner/name of the public repository   (default mpointer/whimbrel)
#   PUBLIC_URL    git URL to push to                     (default https://github.com/$PUBLIC_REPO)
#   SKIP_RELEASE  set to 1 to skip the GitHub release    (default: create it from the private one)
#
# Re-runnable: if the public repository already has the tag, nothing is pushed.
set -euo pipefail

TAG="${1:?usage: scripts/publish-public.sh vX}"
PUBLIC_REPO="${PUBLIC_REPO:-mpointer/whimbrel}"
PUBLIC_URL="${PUBLIC_URL:-https://github.com/$PUBLIC_REPO}"
PRIVATE_REPO="mpointer/kestrel"
EXCLUDE=(docs/superpowers)

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

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Exporting $TAG (without ${EXCLUDE[*]})"
mkdir -p "$WORK/tree"
git archive --format=tar "$TAG" | tar -x -C "$WORK/tree"
for path in "${EXCLUDE[@]}"; do rm -rf "$WORK/tree/$path"; done

echo "==> Fetching $PUBLIC_URL"
git clone --quiet --depth 1 "$PUBLIC_URL" "$WORK/public" 2>/dev/null || git clone --quiet "$PUBLIC_URL" "$WORK/public"
cd "$WORK/public"
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
    echo "==> $PUBLIC_REPO already has $TAG; nothing to publish"
    exit 0
fi
git checkout --quiet -B main 2>/dev/null || true
if git rev-parse --quiet --verify HEAD >/dev/null; then
    git rm -rq --quiet . >/dev/null 2>&1 || true
fi
# Everything the export produced, including dotfiles, over the emptied tree.
cp -a "$WORK/tree/." .
git add -A
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
