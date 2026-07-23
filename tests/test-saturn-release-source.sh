#!/usr/bin/env bash
# shellcheck disable=SC2016 # Fixed source literals are intentionally not expanded.
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
BUILDER="$REPO_ROOT/update_manager/scripts/saturn-release-build.sh"
TMP_ROOT="$(mktemp -d)"
REMOTE="$TMP_ROOT/remote.git"
SOURCE="$TMP_ROOT/source"

cleanup(){ rm -rf -- "$TMP_ROOT"; }
trap cleanup EXIT

grep -Fq \
  'SOURCE_WORKTREE_ROOT="${SOURCE_WORKTREE_ROOT:-$OUTPUT_ROOT/.source-worktrees}"' \
  "$BUILDER"
if grep -Fq 'mktemp -d "${TMPDIR:-/tmp}/saturn-release-source.' "$BUILDER"; then
  printf 'release source worktree unexpectedly uses the small temporary filesystem\n' >&2
  exit 1
fi

git init --quiet --bare "$REMOTE"
git init --quiet "$SOURCE"
git -C "$SOURCE" config user.name "Saturn Test"
git -C "$SOURCE" config user.email "saturn-test@example.invalid"
printf 'first\n' >"$SOURCE/release-input"
git -C "$SOURCE" add release-input
git -C "$SOURCE" commit --quiet -m "first"
git -C "$SOURCE" branch -M main
FIRST_COMMIT="$(git -C "$SOURCE" rev-parse HEAD)"
git -C "$SOURCE" tag -a release-v1 -m "release v1"
git -C "$SOURCE" push --quiet "$REMOTE" main refs/tags/release-v1

branch_resolution="$("$BUILDER" \
  --source-remote "$REMOTE" \
  --source-ref main \
  --resolve-only)"
grep -Fxq "requested_ref=main" <<<"$branch_resolution"
grep -Fxq "resolved_ref=refs/heads/main" <<<"$branch_resolution"
grep -Fxq "resolved_commit=$FIRST_COMMIT" <<<"$branch_resolution"

tag_resolution="$("$BUILDER" \
  --source-remote "$REMOTE" \
  --source-ref release-v1 \
  --resolve-only)"
grep -Fxq "resolved_ref=refs/tags/release-v1" <<<"$tag_resolution"
grep -Fxq "resolved_commit=$FIRST_COMMIT" <<<"$tag_resolution"

git -C "$SOURCE" branch ambiguous
git -C "$SOURCE" tag ambiguous
git -C "$SOURCE" push --quiet "$REMOTE" \
  refs/heads/ambiguous refs/tags/ambiguous
if "$BUILDER" \
  --source-remote "$REMOTE" \
  --source-ref ambiguous \
  --resolve-only >/dev/null 2>&1; then
  printf 'ambiguous branch/tag source selection unexpectedly passed\n' >&2
  exit 1
fi

explicit_resolution="$("$BUILDER" \
  --source-remote "$REMOTE" \
  --source-ref refs/heads/ambiguous \
  --resolve-only)"
grep -Fxq "resolved_ref=refs/heads/ambiguous" <<<"$explicit_resolution"
grep -Fxq "resolved_commit=$FIRST_COMMIT" <<<"$explicit_resolution"

if "$BUILDER" \
  --source-remote "$REMOTE" \
  --source-ref missing \
  --resolve-only >/dev/null 2>&1; then
  printf 'missing source ref unexpectedly resolved\n' >&2
  exit 1
fi

WRONG_COMMIT="0000000000000000000000000000000000000000"
if SATURN_RELEASE_EXPECTED_COMMIT="$WRONG_COMMIT" \
  SATURN_RELEASE_REPO_ROOT="$REPO_ROOT" \
  "$BUILDER" --dry-run >/dev/null 2>&1; then
  printf 'builder accepted a checkout different from the resolved commit\n' >&2
  exit 1
fi

printf 'Saturn exact release source tests passed\n'
