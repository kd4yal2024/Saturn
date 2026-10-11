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

# A build can be pinned to the commit that was resolved earlier: if the ref has moved, nothing is fetched, built or written.
pinned="$("$BUILDER" --source-remote "$REMOTE" --source-ref main --resolve-only --expect-commit "$FIRST_COMMIT")"
grep -Fxq "resolved_commit=$FIRST_COMMIT" <<<"$pinned"
for bad in abc "${FIRST_COMMIT^^}" "${FIRST_COMMIT:0:39}" "${FIRST_COMMIT}0" ""; do
  if "$BUILDER" --source-remote "$REMOTE" --source-ref main --resolve-only --expect-commit "$bad" >/dev/null 2>&1; then
    printf 'builder accepted the malformed --expect-commit %q\n' "$bad" >&2
    exit 1
  fi
done
printf 'second\n' >"$SOURCE/release-input"
git -C "$SOURCE" commit --quiet -am "second"
SECOND_COMMIT="$(git -C "$SOURCE" rev-parse HEAD)"
git -C "$SOURCE" push --quiet "$REMOTE" main
for extra in --resolve-only --dry-run; do
  rm -rf -- "$TMP_ROOT/pinned-out"
  if "$BUILDER" --source-remote "$REMOTE" --source-ref main --output-root "$TMP_ROOT/pinned-out" --expect-commit "$FIRST_COMMIT" "$extra" >"$TMP_ROOT/moved.out" 2>"$TMP_ROOT/moved.err"; then
    printf 'builder built a ref that had moved since it was resolved (%s)\n' "$extra" >&2
    exit 1
  fi
  grep -Fq "source ref moved since it was resolved: expected $FIRST_COMMIT" "$TMP_ROOT/moved.err"
  grep -Fq "$SECOND_COMMIT" "$TMP_ROOT/moved.err"
  grep -Fq "nothing was built" "$TMP_ROOT/moved.err"
  [[ ! -e "$TMP_ROOT/pinned-out" ]] || { printf 'a moved ref still produced output (%s)\n' "$extra" >&2; exit 1; }
done
# resolving again with the new commit pinned works, and a local build is pinned to its checkout too
"$BUILDER" --source-remote "$REMOTE" --source-ref main --resolve-only --expect-commit "$SECOND_COMMIT" >/dev/null
if SATURN_RELEASE_REPO_ROOT="$REPO_ROOT" "$BUILDER" --dry-run --expect-commit "$WRONG_COMMIT" >/dev/null 2>"$TMP_ROOT/local.err"; then
  printf 'a local build ignored --expect-commit\n' >&2
  exit 1
fi
grep -Fq "not the expected $WRONG_COMMIT" "$TMP_ROOT/local.err"

printf 'Saturn exact release source tests passed\n'
