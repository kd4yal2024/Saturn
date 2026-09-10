#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFLIGHT="$REPO_ROOT/update_manager/scripts/saturn-go-build-preflight.sh"
UPDATER="$REPO_ROOT/update_manager/scripts/update-saturn-go.sh"
INSTALLER="$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
BRIDGE_INSTALLER="$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"
RUST_TOOLCHAIN_HELPER="$REPO_ROOT/update_manager/scripts/saturn-rust-toolchain.sh"
RUST_TOOLCHAIN_FILE="$REPO_ROOT/rust-toolchain.toml"
PROVISIONER="$REPO_ROOT/provision/cloud-init/provision-saturn.sh"
CI_WORKFLOW="$REPO_ROOT/.github/workflows/ci.yml"

fail(){
  printf 'low-memory Rust build contract failed: %s\n' "$*" >&2
  exit 1
}

for script in "$PREFLIGHT" "$UPDATER" "$INSTALLER" "$BRIDGE_INSTALLER" "$RUST_TOOLCHAIN_HELPER"; do
  bash -n "$script"
done

grep -Fq 'channel = "1.98.1"' "$RUST_TOOLCHAIN_FILE" \
  || fail "repository does not pin the validated Rust toolchain"
[[ "$(grep -Fc 'uses: dtolnay/rust-toolchain@1.98.1' "$CI_WORKFLOW")" -eq 2 ]] \
  || fail "CI Rust jobs do not install the repository-pinned toolchain"
arm_config="$(SATURN_RUSTUP_TARGET=aarch64-unknown-linux-gnu "$RUST_TOOLCHAIN_HELPER" print-config)"
grep -Fq 'rustup_url=https://static.rust-lang.org/rustup/archive/1.29.1/aarch64-unknown-linux-gnu/rustup-init' <<<"$arm_config" \
  || fail "Rust prerequisite does not use the immutable arm64 rustup artifact"
grep -Fq 'rustup_sha256=15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433' <<<"$arm_config" \
  || fail "Rust prerequisite arm64 checksum changed unexpectedly"
grep -Fq 'toolchain=1.98.1' <<<"$arm_config" \
  || fail "Rust prerequisite does not resolve the repository toolchain pin"
# rustup-init dispatches by argv[0], so its downloaded basename must stay exact.
# shellcheck disable=SC2016
grep -Fq 'installer="${temp_dir}/rustup-init"' "$RUST_TOOLCHAIN_HELPER" \
  || fail "Rust bootstrap does not preserve the required rustup-init executable name"
x86_config="$(SATURN_RUSTUP_TARGET=x86_64-unknown-linux-gnu "$RUST_TOOLCHAIN_HELPER" print-config)"
grep -Fq 'rustup_sha256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71' <<<"$x86_config" \
  || fail "Rust prerequisite x86_64 checksum changed unexpectedly"
armv7_config="$(SATURN_RUSTUP_TARGET=armv7-unknown-linux-gnueabihf "$RUST_TOOLCHAIN_HELPER" print-config)"
grep -Fq 'rustup_sha256=6f34abb0d553273ce08306ea3adb758d0171f21090cc5ad5426f474ded5504d1' <<<"$armv7_config" \
  || fail "Rust prerequisite armv7 checksum changed unexpectedly"
grep -Fq 'run_phase rust-toolchain "Preparing Rust build prerequisite"' "$PROVISIONER" \
  || fail "provisioning does not establish Rust before component builds"
# shellcheck disable=SC2016
grep -Fq 'bash "$RUST_TOOLCHAIN_HELPER" ensure' "$INSTALLER" \
  || fail "Saturn Go installer bypasses the shared Rust prerequisite"
# shellcheck disable=SC2016
grep -Fq 'bash "$SATURN_RUST_TOOLCHAIN_HELPER" ensure' "$BRIDGE_INSTALLER" \
  || fail "Saturn Bridge installer bypasses the shared Rust prerequisite"

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
