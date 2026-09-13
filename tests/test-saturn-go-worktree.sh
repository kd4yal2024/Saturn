#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
UPDATER="$REPO_ROOT/update_manager/scripts/update-saturn-go.sh"
TEST_ROOT="$(mktemp -d)"
FIXTURE_REPO="$TEST_ROOT/normal-checkout"
LINKED_WORKTREE="$TEST_ROOT/linked-worktree"

fail(){
  printf 'Saturn Go worktree validation failed: %s\n' "$*" >&2
  exit 1
}

cleanup(){
  git -C "$FIXTURE_REPO" worktree remove --force "$LINKED_WORKTREE" >/dev/null 2>&1 || true
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT

# The regression intentionally matches the literal updater expression.
# shellcheck disable=SC2016
grep -Fq 'git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 ||' "$UPDATER" \
  || fail "updater does not use Git-aware checkout validation"

mkdir -p \
  "$FIXTURE_REPO/update_manager/rust-server" \
  "$FIXTURE_REPO/update_manager/templates" \
  "$FIXTURE_REPO/update_manager/scripts" \
  "$FIXTURE_REPO/update_manager/saturn-bridge" \
  "$FIXTURE_REPO/scripts"
touch \
  "$FIXTURE_REPO/update_manager/rust-server/Cargo.toml" \
  "$FIXTURE_REPO/update_manager/templates/.keep" \
  "$FIXTURE_REPO/update_manager/saturn-bridge/Cargo.toml" \
  "$FIXTURE_REPO/update_manager/scripts/saturn-go-web-assets.sh" \
  "$FIXTURE_REPO/update_manager/scripts/saturn-go-build-preflight.sh" \
  "$FIXTURE_REPO/update_manager/scripts/install-saturn-bridge.sh" \
  "$FIXTURE_REPO/update_manager/scripts/saturn-go-deploy-root.sh" \
  "$FIXTURE_REPO/update_manager/scripts/config.json" \
  "$FIXTURE_REPO/update_manager/scripts/themes.json" \
  "$FIXTURE_REPO/scripts/fix-LED-power-button.sh" \
  "$FIXTURE_REPO/scripts/install-shutdown-waiter-service.sh" \
  "$FIXTURE_REPO/scripts/shutdown-waiter.sh" \
  "$FIXTURE_REPO/scripts/setup-eth-fallback.sh"
chmod 0755 \
  "$FIXTURE_REPO/update_manager/scripts/install-saturn-bridge.sh" \
  "$FIXTURE_REPO/update_manager/scripts/saturn-go-deploy-root.sh"

git -C "$FIXTURE_REPO" init --quiet
git -C "$FIXTURE_REPO" config user.name 'Saturn test'
git -C "$FIXTURE_REPO" config user.email 'saturn-test@example.invalid'
git -C "$FIXTURE_REPO" add .
git -C "$FIXTURE_REPO" commit --quiet -m fixture
git -C "$FIXTURE_REPO" worktree add --quiet --detach "$LINKED_WORKTREE" HEAD

validate_candidate(){
  local label="$1"
  local candidate="$2"
  local output

  output="$(
    SATURN_ACTIVE_REPO_ROOT="$candidate" \
    SATURN_SATURNGO_DEPLOY_STATUS_FILE="$TEST_ROOT/$label-status.json" \
      bash "$UPDATER" --skip-git --skip-build --skip-deploy --dry-run 2>&1
  )" || fail "$label checkout was rejected: $output"

  grep -Fq "Repo root: $candidate" <<<"$output" \
    || fail "$label checkout did not pass repository validation"
  grep -Fq 'Skipping deploy (--skip-deploy)' <<<"$output" \
    || fail "$label checkout dry-run did not reach its non-deploying exit"
}

validate_candidate normal "$FIXTURE_REPO"
validate_candidate linked "$LINKED_WORKTREE"

printf 'Saturn Go normal-checkout and linked-worktree validation passed\n'
