#!/usr/bin/env bash
# Test, build, and atomically deploy the production Protocol 2 application.

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SATURN_ROOT="${SATURN_REPO_ROOT:-${SATURN_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}}"
P2_DIR="$SATURN_ROOT/sw_projects/P2_app"
DEPLOY_HELPER="${SATURN_P2_DEPLOY_HELPER:-/usr/local/lib/saturn-go/scripts/saturn-p2-deploy.sh}"
BUILD_JOBS="${SATURN_P2_BUILD_JOBS:-1}"
SKIP_TESTS="${SATURN_P2_SKIP_TESTS:-0}"

info(){ printf '[update-p2app] %s\n' "$*"; }
die(){ printf '[update-p2app] ERROR: %s\n' "$*" >&2; exit 1; }

[[ -f "$P2_DIR/Makefile" ]] || die "P2 source tree not found: $P2_DIR"
[[ "$BUILD_JOBS" =~ ^[1-9][0-9]*$ ]] || die "SATURN_P2_BUILD_JOBS must be a positive integer"

if [[ "$SKIP_TESTS" != 1 ]]; then
  info "Running Protocol 2 tests"
  make -C "$P2_DIR" test
fi

info "Building Protocol 2 runtime"
make -C "$P2_DIR" clean
make -C "$P2_DIR" -j"$BUILD_JOBS"
[[ -x "$P2_DIR/p2app" ]] || die "build did not produce $P2_DIR/p2app"

[[ -x "$DEPLOY_HELPER" ]] || die "trusted deploy helper is missing; rerun the appliance installer: $DEPLOY_HELPER"
info "Deploying Protocol 2 runtime"
if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
  "$DEPLOY_HELPER"
elif [[ -t 0 ]]; then
  sudo "$DEPLOY_HELPER"
else
  sudo -n "$DEPLOY_HELPER" || die "P2 deployment through the trusted helper failed"
fi

runtime_bin="${SATURN_P2_RUNTIME_BIN:-/opt/saturn-radio/bin/p2app}"
[[ -f "$runtime_bin" ]] || die "production P2 binary is missing after deployment: $runtime_bin"
[[ "$(sha256sum "$P2_DIR/p2app" | awk '{print $1}')" == "$(sha256sum "$runtime_bin" | awk '{print $1}')" ]] || \
  die "deployed P2 binary does not match this checkout; rerun the appliance installer to refresh the deploy contract"

info "Protocol 2 update complete"
