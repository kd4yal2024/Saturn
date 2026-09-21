#!/usr/bin/env bash
# Copy to the root of a source bundle, then run as pi. Build/test only:
# no service operations, device access, installation, or radio configuration.
set -Eeuo pipefail
stage=$(cd -- "$(dirname -- "$0")" && pwd)
cd "$stage"
[[ $(id -u) != 0 ]] || { echo 'Build as pi, not root.' >&2; exit 1; }
[[ $(uname -m) == aarch64 ]] || { echo 'Build on the ARM64 G2.' >&2; exit 1; }
[[ ! -e SHA256SUMS ]] || { echo 'Bundle already sealed; use a fresh staging directory.' >&2; exit 1; }
sha256sum --check --quiet SOURCE-SHA256SUMS
sha256sum --check --quiet installed-before.sha256
export PATH=/home/pi/.cargo/bin:$PATH
unset SATURN_BRIDGE_STUB_NATIVE RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
read -r SATURN_BUILD_COMMIT < source-commit.txt
export SATURN_BUILD_COMMIT
export SATURN_BUILD_DIRTY=true
export SATURN_BRIDGE_WDSP_FLAVOR=wdsp2
export SATURN_BRIDGE_WDSP_COMMIT=584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3
export SATURN_WDSP_DIR=/home/pi/github/Saturn/update_manager/saturn-bridge/target-local/wdsp2-linux-arm
export CARGO_TARGET_DIR="$stage/target"
export SATURN_BRIDGE_SATP_STATUS_PATH="$stage/test-output/satp-status.json"
test -f "$SATURN_WDSP_DIR/libwdsp.a"
sha256sum "$SATURN_WDSP_DIR/libwdsp.a" > wdsp-library.sha256
cd "$stage/update_manager/saturn-bridge"
nice -n 15 cargo build --release --locked -j 2
# SATP uses localhost and inhibited RF in tests; MON uses file-backed mocks.
# Never run the production executable during staging.
for suite in satp::tests satp_control::tests tx_monitor::tests; do
    nice -n 15 cargo test --release --locked -j 2 "$suite" -- --test-threads=1
done
cd "$stage"
install -m 755 "$CARGO_TARGET_DIR/release/saturn-bridge" saturn-bridge
file saturn-bridge
ldd saturn-bridge
sha256sum --check --quiet SOURCE-SHA256SUMS
sha256sum --check --quiet wdsp-library.sha256
sha256sum --check --quiet installed-before.sha256
sha256sum saturn-bridge preflight.sh build.sh SOURCE-SHA256SUMS \
    installed-before.sha256 source-commit.txt wdsp-library.sha256 \
    update_manager/templates/saturn-remote-next.html \
    update_manager/templates/settings.html \
    update_manager/remote-web/dist/saturn-remote-next.js \
    update_manager/remote-web/dist/saturn-remote-next.js.sha256 > SHA256SUMS
bash preflight.sh --check
