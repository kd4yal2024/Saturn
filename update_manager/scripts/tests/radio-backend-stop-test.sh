#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
HELPER="$SCRIPT_DIR/saturn-radio-backend-switch-root.sh"
TEST_ROOT="$(mktemp -d /tmp/saturn-radio-stop.XXXXXX)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT

mkdir -p \
  "$TEST_ROOT/bin" \
  "$TEST_ROOT/etc/systemd/system/saturn-bridge.service.d" \
  "$TEST_ROOT/lock" \
  "$TEST_ROOT/run" \
  "$TEST_ROOT/services" \
  "$TEST_ROOT/state"
touch "$TEST_ROOT/services/saturn-bridge.service"
touch "$TEST_ROOT/services/saturn-bridge.service.enabled"

cat >"$TEST_ROOT/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$MOCK_SYSTEMCTL_LOG"
case "${1:-}" in
  is-active)
    service="${3:-${2:-}}"
    [[ -f "$MOCK_SERVICE_STATE/$service" ]]
    ;;
  is-enabled)
    service="${3:-${2:-}}"
    [[ -f "$MOCK_SERVICE_STATE/$service.enabled" ]]
    ;;
  show)
    printf 'SATURN_BRIDGE_RADIO_BACKEND=%s\n' "${MOCK_RUNTIME_BACKEND:-xdma}"
    ;;
  stop)
    shift
    for service in "$@"; do rm -f -- "$MOCK_SERVICE_STATE/$service"; done
    ;;
  start)
    shift
    for service in "$@"; do touch "$MOCK_SERVICE_STATE/$service"; done
    ;;
  enable)
    shift
    for service in "$@"; do touch "$MOCK_SERVICE_STATE/$service.enabled"; done
    ;;
  disable)
    shift
    for service in "$@"; do rm -f -- "$MOCK_SERVICE_STATE/$service.enabled"; done
    ;;
  daemon-reload)
    ;;
  *)
    printf 'unexpected mock systemctl command: %s\n' "$*" >&2
    exit 2
    ;;
esac
EOF
chmod 0755 "$TEST_ROOT/bin/systemctl"

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
cat >"$TEST_ROOT/state/selection.json" <<'EOF'
{
  "schema_version": 1,
  "requested": "xdma",
  "active": "xdma",
  "status": "ready"
}
EOF
cat >"$TEST_ROOT/etc/systemd/system/saturn-bridge.service.d/20-radio-backend.conf" <<'EOF'
[Unit]
Conflicts=p2app.service
[Service]
Environment=SATURN_BRIDGE_RADIO_BACKEND=xdma
EOF

env \
  PATH="$TEST_ROOT/bin:$PATH" \
  MOCK_SERVICE_STATE="$TEST_ROOT/services" \
  MOCK_SYSTEMCTL_LOG="$TEST_ROOT/systemctl.log" \
  SATURN_RADIO_BACKEND_CONFIG="$TEST_ROOT/backend.conf" \
  SATURN_RADIO_BACKEND_TEST_MODE=1 \
  "$HELPER" stop xdma >/dev/null

[[ ! -e "$TEST_ROOT/services/saturn-bridge.service" ]]
[[ ! -e "$TEST_ROOT/services/p2app.service" ]]

status="$(env \
  PATH="$TEST_ROOT/bin:$PATH" \
  MOCK_SERVICE_STATE="$TEST_ROOT/services" \
  MOCK_SYSTEMCTL_LOG="$TEST_ROOT/systemctl.log" \
  SATURN_RADIO_BACKEND_CONFIG="$TEST_ROOT/backend.conf" \
  SATURN_RADIO_BACKEND_TEST_MODE=1 \
  "$HELPER" status)"
python3 - "$status" <<'PY'
import json
import sys

value = json.loads(sys.argv[1])
assert value["selected"] == "xdma"
assert value["persisted_status"] == "stopped"
assert value["operational_status"] == "stopped"
assert value["services"] == {"p2app": "inactive", "saturn_bridge": "inactive"}
assert value["mutual_exclusion_ok"] is True
PY

cat >"$TEST_ROOT/state/selection.json" <<'EOF'
{
  "schema_version": 1,
  "requested": "p2",
  "active": "p2",
  "status": "ready"
}
EOF
cat >"$TEST_ROOT/etc/systemd/system/saturn-bridge.service.d/20-radio-backend.conf" <<'EOF'
[Unit]
Conflicts=p2app.service
[Service]
Environment=SATURN_BRIDGE_RADIO_BACKEND=p2
EOF
touch "$TEST_ROOT/services/p2app.service" "$TEST_ROOT/services/saturn-bridge.service"
: >"$TEST_ROOT/systemctl.log"

env \
  PATH="$TEST_ROOT/bin:$PATH" \
  MOCK_SERVICE_STATE="$TEST_ROOT/services" \
  MOCK_SYSTEMCTL_LOG="$TEST_ROOT/systemctl.log" \
  MOCK_RUNTIME_BACKEND=p2 \
  SATURN_RADIO_BACKEND_CONFIG="$TEST_ROOT/backend.conf" \
  SATURN_RADIO_BACKEND_TEST_MODE=1 \
  "$HELPER" restart bridge >/dev/null

[[ -e "$TEST_ROOT/services/p2app.service" ]]
[[ -e "$TEST_ROOT/services/saturn-bridge.service" ]]
mapfile -t bridge_transitions < <(
  grep -E '^(stop|start) saturn-bridge\.service$' "$TEST_ROOT/systemctl.log"
)
[[ "${bridge_transitions[*]}" == "stop saturn-bridge.service start saturn-bridge.service" ]]

printf 'radio backend stop test passed\n'
