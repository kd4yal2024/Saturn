#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="$REPO_ROOT/update_manager/scripts/saturn-radio-backend-switch-root.sh"
INSTALLER="$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/bin" "$TMP_DIR/systemd" "$TMP_DIR/state" "$TMP_DIR/run" \
  "$TMP_DIR/mock"

cat >"$TMP_DIR/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

service_state() {
  printf '%s/%s.active' "$MOCK_STATE" "$1"
}

backend_from_dropin() {
  if [[ -f "$MOCK_BRIDGE_DROPIN" ]]; then
    sed -n 's/^Environment=SATURN_BRIDGE_RADIO_BACKEND=//p' \
      "$MOCK_BRIDGE_DROPIN" | tail -n 1
  else
    printf 'p2\n'
  fi
}

case "${1:-}" in
  is-active)
    shift
    [[ "${1:-}" == "--quiet" ]] && shift
    [[ -f "$(service_state "$1")" ]]
    ;;
  show)
    printf 'SATURN_BRIDGE_RADIO_BACKEND=%s\n' "$(backend_from_dropin)"
    ;;
  stop)
    service="$2"
    if [[ "$service" == "p2app.service" && -f "$MOCK_STATE/refuse-p2-stop" ]]; then
      exit 0
    fi
    rm -f "$(service_state "$service")"
    printf 'stop %s\n' "$service" >>"$MOCK_STATE/calls"
    ;;
  start)
    service="$2"
    if [[ "$service" == "saturn-bridge.service" && -f "$MOCK_STATE/fail-bridge-once" ]]; then
      rm -f "$MOCK_STATE/fail-bridge-once"
      printf 'failed-start %s\n' "$service" >>"$MOCK_STATE/calls"
      exit 1
    fi
    : >"$(service_state "$service")"
    if [[ "$service" == "saturn-bridge.service" \
      && "$(backend_from_dropin)" == "xdma" \
      && ! -f "$MOCK_STATE/suppress-xdma-ready" ]]; then
      mkdir -p "$(dirname "$MOCK_XDMA_READY_FILE")"
      printf '{"schema_version":1,"updated_at_ms":%s,"source":"saturn-bridge","backend":"xdma","status":"ready","rf_safe":true,"error":null,"metrics":{"dma_reads":8,"iq_pairs":2048,"tx_capable":true}}\n' \
        "$(date +%s%3N)" >"$MOCK_XDMA_READY_FILE"
    fi
    printf 'start %s\n' "$service" >>"$MOCK_STATE/calls"
    ;;
  daemon-reload)
    printf 'daemon-reload\n' >>"$MOCK_STATE/calls"
    ;;
  *)
    printf 'unexpected systemctl command: %s\n' "$*" >&2
    exit 1
    ;;
esac
EOF
chmod 0755 "$TMP_DIR/bin/systemctl"

write_config() {
  local xdma_enabled="$1"
  cat >"$TMP_DIR/config" <<EOF
XDMA_OPERATIONAL_ENABLED="$xdma_enabled"
STATE_FILE="$TMP_DIR/state/radio-backend.json"
TRANSACTION_FILE="$TMP_DIR/run/transaction.json"
LOCK_FILE="$TMP_DIR/run/radio.lock"
SYSTEMD_ROOT="$TMP_DIR/systemd"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
BRIDGE_DROPIN_NAME="20-radio-backend.conf"
READY_TIMEOUT_SECONDS="1"
XDMA_READY_FILE="$TMP_DIR/run/xdma-ready.json"
STATE_GROUP="$(id -gn)"
EOF
}

run_helper() {
  env \
    PATH="$TMP_DIR/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    MOCK_STATE="$TMP_DIR/mock" \
    MOCK_BRIDGE_DROPIN="$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf" \
    MOCK_XDMA_READY_FILE="$TMP_DIR/run/xdma-ready.json" \
    SATURN_RADIO_BACKEND_CONFIG="$TMP_DIR/config" \
    SATURN_RADIO_BACKEND_TEST_MODE=1 \
    "$HELPER" "$@"
}

reset_fixture() {
  rm -rf "$TMP_DIR/systemd" "$TMP_DIR/state" "$TMP_DIR/run" "$TMP_DIR/mock"
  mkdir -p "$TMP_DIR/systemd" "$TMP_DIR/state" "$TMP_DIR/run" "$TMP_DIR/mock"
  : >"$TMP_DIR/mock/p2app.service.active"
  : >"$TMP_DIR/mock/saturn-bridge.service.active"
  : >"$TMP_DIR/mock/calls"
  write_config "$1"
}

assert_state_backend() {
  local expected="$1"
  python3 - "$TMP_DIR/state/radio-backend.json" "$expected" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    state = json.load(handle)
assert state["requested"] == sys.argv[2], state
assert state["active"] == sys.argv[2], state
assert state["status"] == "ready", state
PY
}

# Status is read-only and defaults safely to P2.
reset_fixture 0
run_helper status >"$TMP_DIR/status.json"
python3 - "$TMP_DIR/status.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    status = json.load(handle)
assert status["selected"] == "p2", status
assert status["requested"] == "p2", status
assert status["runtime"] == "p2", status
assert status["transaction_status"] == "idle", status
assert status["mutual_exclusion_ok"] is True, status
assert status["xdma_operational_enabled"] is False, status
PY
[[ ! -s "$TMP_DIR/mock/calls" ]]

# Production-default XDMA gating occurs before any service or state mutation.
if run_helper switch xdma >"$TMP_DIR/gated.log" 2>&1; then
  printf 'probe-only XDMA backend was incorrectly activated\n' >&2
  exit 1
fi
[[ ! -e "$TMP_DIR/state/radio-backend.json" ]]
[[ ! -e "$TMP_DIR/run/transaction.json" ]]
[[ ! -s "$TMP_DIR/mock/calls" ]]
[[ -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]

# P2 transaction persists only after both services are ready.
run_helper switch p2 >/dev/null
assert_state_backend p2
grep -Fq 'Environment=SATURN_BRIDGE_RADIO_BACKEND=p2' \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"
if grep -Fq 'Conflicts=p2app.service' \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"; then
  printf 'P2 backend retained the XDMA-only systemd conflict\n' >&2
  exit 1
fi
[[ -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]
[[ ! -e "$TMP_DIR/run/transaction.json" ]]

# Fixture-enabled XDMA proves exclusive ownership and persistence.
reset_fixture 1
run_helper switch xdma >/dev/null
assert_state_backend xdma
grep -Fq 'Environment=SATURN_BRIDGE_RADIO_BACKEND=xdma' \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"
grep -Fq 'Conflicts=p2app.service' \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"
[[ ! -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]

# The same test-gated transaction returns ownership cleanly to P2.
run_helper switch p2 >/dev/null
assert_state_backend p2
grep -Fq 'Environment=SATURN_BRIDGE_RADIO_BACKEND=p2' \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"
if grep -Fq 'Conflicts=p2app.service' \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"; then
  printf 'P2 round trip retained the XDMA-only systemd conflict\n' >&2
  exit 1
fi
[[ -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]
[[ ! -e "$TMP_DIR/run/transaction.json" ]]

# A bridge that starts but never proves a fresh RX stream is rolled back.
reset_fixture 1
run_helper switch p2 >/dev/null
cp "$TMP_DIR/state/radio-backend.json" "$TMP_DIR/original-state"
: >"$TMP_DIR/mock/suppress-xdma-ready"
if run_helper switch xdma >"$TMP_DIR/readiness-rollback.log" 2>&1; then
  printf 'missing direct XDMA readiness was incorrectly accepted\n' >&2
  exit 1
fi
cmp "$TMP_DIR/original-state" "$TMP_DIR/state/radio-backend.json"
[[ -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]
[[ ! -e "$TMP_DIR/run/transaction.json" ]]

# A bridge start failure restores the prior service, drop-in, and state.
reset_fixture 1
mkdir -p "$TMP_DIR/systemd/saturn-bridge.service.d"
printf '%s\n' \
  '[Service]' \
  'Environment=SATURN_BRIDGE_RADIO_BACKEND=p2' \
  >"$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"
run_helper switch p2 >/dev/null
cp "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf" \
  "$TMP_DIR/original-dropin"
cp "$TMP_DIR/state/radio-backend.json" "$TMP_DIR/original-state"
: >"$TMP_DIR/mock/fail-bridge-once"
if run_helper switch xdma >"$TMP_DIR/rollback.log" 2>&1; then
  printf 'failed bridge start was incorrectly accepted\n' >&2
  exit 1
fi
cmp "$TMP_DIR/original-dropin" \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"
cmp "$TMP_DIR/original-state" "$TMP_DIR/state/radio-backend.json"
[[ -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]
[[ ! -e "$TMP_DIR/run/transaction.json" ]]

# A failed return from XDMA to P2 restores exclusive XDMA ownership.
reset_fixture 1
run_helper switch xdma >/dev/null
cp "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf" \
  "$TMP_DIR/original-dropin"
cp "$TMP_DIR/state/radio-backend.json" "$TMP_DIR/original-state"
: >"$TMP_DIR/mock/fail-bridge-once"
if run_helper switch p2 >"$TMP_DIR/p2-rollback.log" 2>&1; then
  printf 'failed P2 return was incorrectly accepted\n' >&2
  exit 1
fi
cmp "$TMP_DIR/original-dropin" \
  "$TMP_DIR/systemd/saturn-bridge.service.d/20-radio-backend.conf"
cmp "$TMP_DIR/original-state" "$TMP_DIR/state/radio-backend.json"
[[ ! -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]
[[ ! -e "$TMP_DIR/run/transaction.json" ]]

# A P2 service that remains active blocks XDMA ownership and rolls back.
reset_fixture 1
run_helper switch p2 >/dev/null
cp "$TMP_DIR/state/radio-backend.json" "$TMP_DIR/original-state"
: >"$TMP_DIR/mock/refuse-p2-stop"
if run_helper switch xdma >"$TMP_DIR/exclusion.log" 2>&1; then
  printf 'concurrent P2 and XDMA ownership was incorrectly accepted\n' >&2
  exit 1
fi
cmp "$TMP_DIR/original-state" "$TMP_DIR/state/radio-backend.json"
[[ -f "$TMP_DIR/mock/p2app.service.active" ]]
[[ -f "$TMP_DIR/mock/saturn-bridge.service.active" ]]
[[ ! -e "$TMP_DIR/run/transaction.json" ]]

grep -Fq "\"\$SOURCE_DIR/scripts/\$SATURN_RADIO_BACKEND_SWITCH_NAME\"" "$INSTALLER"
grep -Fq "\${PRIVILEGED_SCRIPTS_DIR}/\${SATURN_RADIO_BACKEND_SWITCH_NAME} status" \
  "$INSTALLER"
grep -Fq "\${PRIVILEGED_SCRIPTS_DIR}/\${SATURN_RADIO_BACKEND_SWITCH_NAME} switch p2" \
  "$INSTALLER"
grep -Fq "\${PRIVILEGED_SCRIPTS_DIR}/\${SATURN_RADIO_BACKEND_SWITCH_NAME} switch xdma" \
  "$INSTALLER"
if grep -Fq 'After=network-online.target p2app.service' \
  "$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"; then
  printf 'bridge base unit still carries an implicit P2 dependency\n' >&2
  exit 1
fi
grep -Fq 'RuntimeDirectory=saturn-bridge' \
  "$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"
grep -Fq 'SATURN_BRIDGE_XDMA_READY_PATH=/run/saturn-bridge/xdma-ready.json' \
  "$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"
grep -Fq 'capture_selected_backend' \
  "$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"
grep -Fq "switch \"\$SATURN_BRIDGE_PRESERVED_BACKEND\"" \
  "$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"
grep -Fq 'LimitRTPRIO=21' \
  "$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"

printf 'Saturn radio backend transaction tests passed\n'
