#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFLIGHT="$REPO_ROOT/update_manager/scripts/saturn-go-build-preflight.sh"
UPDATER="$REPO_ROOT/update_manager/scripts/update-saturn-go.sh"
INSTALLER="$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
BRIDGE_INSTALLER="$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"

fail(){
  printf 'low-memory Rust build contract failed: %s\n' "$*" >&2
  exit 1
}

for script in "$PREFLIGHT" "$UPDATER" "$INSTALLER" "$BRIDGE_INSTALLER"; do
  bash -n "$script"
done

# The patterns below intentionally match literal shell parameter expansions.
# shellcheck disable=SC2016
grep -Fq 'SWAP_MIB="${SATURN_SATURNGO_BUILD_SWAP_MIB:-2048}"' "$PREFLIGHT" \
  || fail "preflight does not default to 2 GiB disk-backed swap"
# shellcheck disable=SC2016
grep -Fq 'RESERVE_MIB="${SATURN_SATURNGO_BUILD_RESERVE_MIB:-512}"' "$PREFLIGHT" \
  || fail "preflight does not retain a disk-space safety reserve"
# shellcheck disable=SC2016
grep -Fq 'BUILD_JOBS="${SATURN_SATURNGO_BUILD_JOBS:-1}"' "$UPDATER" \
  || fail "Saturn Go updater does not default Cargo to one job"
# shellcheck disable=SC2016
grep -Fq 'RUST_BUILD_JOBS="${SATURN_SATURNGO_BUILD_JOBS:-1}"' "$INSTALLER" \
  || fail "Saturn Go installer does not default Cargo to one job"
# shellcheck disable=SC2016
grep -Fq 'SATURN_BRIDGE_BUILD_JOBS="${SATURN_BRIDGE_BUILD_JOBS:-${SATURN_SATURNGO_BUILD_JOBS:-1}}"' "$BRIDGE_INSTALLER" \
  || fail "Saturn Bridge installer does not default Cargo to one job"
grep -Fq 'ensure_low_memory_build_capacity' "$BRIDGE_INSTALLER" \
  || fail "Saturn Bridge installer does not run the build preflight"
# shellcheck disable=SC2016
grep -Fq 'cargo "${cargo_args[@]}" -j "$SATURN_BRIDGE_BUILD_JOBS"' "$BRIDGE_INSTALLER" \
  || fail "Saturn Bridge Cargo invocation does not enforce bounded jobs"
# Boot enablement belongs to the transactional radio-owner helper. In P2 mode
# it enables only P2app; in direct-XDMA mode it enables Saturn Bridge.
# shellcheck disable=SC2016
grep -Fq '"$SATURN_BRIDGE_BACKEND_SWITCH_HELPER" switch "$SATURN_BRIDGE_PRESERVED_BACKEND"' "$BRIDGE_INSTALLER" \
  || fail "Saturn Bridge installer does not delegate startup policy to the backend transaction"
# shellcheck disable=SC2016
if grep -Fq 'systemctl enable "$service_name"' "$BRIDGE_INSTALLER"; then
  fail "Saturn Bridge installer unconditionally enables the service instead of preserving backend startup policy"
fi
# shellcheck disable=SC2016
grep -Fq 'systemctl restart "$service_name"' "$BRIDGE_INSTALLER" \
  || fail "Saturn Bridge installer lacks the broker-unavailable restart fallback"
# shellcheck disable=SC2016
if grep -Fq 'systemctl enable --now "$(basename "$SATURN_BRIDGE_SERVICE")"' "$BRIDGE_INSTALLER"; then
  fail "Saturn Bridge installer can leave an already-running deleted executable active"
fi

if SATURN_SATURNGO_BUILD_SWAP_MIB=invalid bash "$PREFLIGHT" status >/dev/null 2>&1; then
  fail "preflight accepted an invalid swap size"
fi

bash "$PREFLIGHT" status
printf 'low-memory Rust build contract passed\n'
