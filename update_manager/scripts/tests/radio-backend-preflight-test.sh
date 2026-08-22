#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
HELPER="$SCRIPT_DIR/saturn-radio-backend-switch-root.sh"
TEST_ROOT="$(mktemp -d /tmp/saturn-radio-preflight.XXXXXX)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT

mkdir -p \
  "$TEST_ROOT/etc/systemd/system/p2app.service.d" \
  "$TEST_ROOT/lock" \
  "$TEST_ROOT/run" \
  "$TEST_ROOT/state"

cat >"$TEST_ROOT/backend.conf" <<EOF
XDMA_OPERATIONAL_ENABLED="1"
STATE_FILE="$TEST_ROOT/state/selection.json"
TRANSACTION_FILE="$TEST_ROOT/run/transaction.json"
LOCK_FILE="$TEST_ROOT/lock/radio.lock"
XDMA_READY_FILE="$TEST_ROOT/run/xdma-ready.json"
SYSTEMD_ROOT="$TEST_ROOT/etc/systemd/system"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
BRIDGE_DROPIN_NAME="20-radio-backend.conf"
READY_TIMEOUT_SECONDS="1"
STATE_GROUP="$(id -gn)"
EOF

cat >"$TEST_ROOT/etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf" <<EOF
[Service]
ExecStart=
ExecStart=$TEST_ROOT/missing-p2app -s
EOF

set +e
output="$(
  SATURN_RADIO_BACKEND_CONFIG="$TEST_ROOT/backend.conf" \
  SATURN_RADIO_BACKEND_TEST_MODE=1 \
    "$HELPER" switch p2 2>&1
)"
status=$?
set -e

[[ $status -ne 0 ]] || {
  printf 'expected broken P2 preflight to fail\n' >&2
  exit 1
}
[[ "$output" == *"P2 launch override executable is missing"* ]] || {
  printf 'missing actionable preflight error: %s\n' "$output" >&2
  exit 1
}
[[ ! -e "$TEST_ROOT/run/transaction.json" ]] || {
  printf 'preflight failure must happen before a transaction is opened\n' >&2
  exit 1
}

printf 'radio backend P2 launch preflight test passed\n'
