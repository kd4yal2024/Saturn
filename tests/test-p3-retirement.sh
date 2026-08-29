#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
P3_DIR="$REPO_ROOT/sw_projects/P3_app"
SERVER="$REPO_ROOT/update_manager/rust-server/src/main.rs"
README="$REPO_ROOT/README.md"
CI="$REPO_ROOT/.github/workflows/ci.yml"
VERSION_INFO="$REPO_ROOT/update_manager/scripts/g2-version-info.sh"
MANAGER="$REPO_ROOT/update_manager/scripts/p23-app-manager.sh"

fail(){
  printf 'P3 retirement contract failed: %s\n' "$*" >&2
  exit 1
}

[[ ! -e "$P3_DIR/p3app" ]] \
  || fail "a stale p3app executable remains in the historical source tree"
grep -Fq '**Archived staging tree:**' "$P3_DIR/README.md" \
  || fail "historical P3 source is not clearly marked as archived"
grep -Fq '.DEFAULT_GOAL := archived' "$P3_DIR/Makefile" \
  || fail "P3 defaults to an active application build"
grep -Fq 'SATURN_ALLOW_ARCHIVED_P3_BUILD=1' "$P3_DIR/Makefile" \
  || fail "historical comparison build is not explicitly gated"
if make -s -C "$P3_DIR" >/dev/null 2>&1; then
  fail "default P3 make invocation unexpectedly succeeded"
fi

if grep -Eq 'deploy_p3|p3_dir|p3_bin|"p3app"[[:space:]]*=>' "$SERVER"; then
  fail "Saturn Go still exposes P3 as a source, deployment, or runtime choice"
fi
if grep -Fq 'make -C sw_projects/P3_app' "$README"; then
  fail "main README still recommends building archived P3"
fi
if grep -Fq 'Build P3 app' "$CI"; then
  fail "CI still treats P3 as an active application"
fi
if grep -Fq 'p3app)' "$VERSION_INFO"; then
  fail "version reporting still treats p3app as a supported runtime"
fi
if grep -Eq -- '--(build|deploy|restart|switch) \[p2\|p3\]' "$MANAGER"; then
  fail "service manager help still advertises P3 selection"
fi

printf 'P3 retirement contract passed\n'
