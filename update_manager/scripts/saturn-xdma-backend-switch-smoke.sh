#!/usr/bin/env bash
set -Eeuo pipefail

# Exercise the real appliance ownership transaction through the production
# Saturn Go split proxy while keeping the production XDMA selector closed.
#
# Preconditions are deliberately strict: stable P2 ownership, both radio
# services active, a current source-tree debug bridge, and an active Saturn Go
# TLS listener. The bridge binary is staged in a temporary root-owned directory
# under /opt/saturn-go because the production unit deliberately hides /home
# with ProtectHome=yes and the appliance mounts /run noexec. The test-gated
# broker switches P2 -> XDMA -> P2. Cleanup then restores the exact prior
# systemd drop-ins, selection state, readiness file, and service activity even
# after a failed acceptance.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="${SATURN_XDMA_SWITCH_SMOKE_REPO_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd -P)}"
BRIDGE_ROOT="$REPO_ROOT/update_manager/saturn-bridge"
BRIDGE_BINARY="${SATURN_XDMA_SWITCH_SMOKE_BRIDGE_BINARY:-$BRIDGE_ROOT/target/debug/saturn-bridge}"
SWITCH_HELPER="${SATURN_XDMA_SWITCH_SMOKE_HELPER:-$SCRIPT_DIR/saturn-radio-backend-switch-root.sh}"
CLIENT="${SATURN_XDMA_SWITCH_SMOKE_CLIENT:-$SCRIPT_DIR/saturn-xdma-operational-client.py}"
PROXY_BASE_URL="${SATURN_XDMA_SWITCH_SMOKE_PROXY_BASE_URL:-wss://127.0.0.1:8443}"
SYSTEMD_ROOT="${SATURN_XDMA_SWITCH_SMOKE_SYSTEMD_ROOT:-/etc/systemd/system}"
BRIDGE_SERVICE="${SATURN_XDMA_SWITCH_SMOKE_BRIDGE_SERVICE:-saturn-bridge.service}"
P2APP_SERVICE="${SATURN_XDMA_SWITCH_SMOKE_P2APP_SERVICE:-p2app.service}"
SATURN_GO_SERVICE="${SATURN_XDMA_SWITCH_SMOKE_SATURN_GO_SERVICE:-saturn-go.service}"
BACKEND_DROPIN="$SYSTEMD_ROOT/$BRIDGE_SERVICE.d/20-radio-backend.conf"
ACCEPTANCE_DROPIN="$SYSTEMD_ROOT/$BRIDGE_SERVICE.d/95-xdma-acceptance-runtime.conf"
PRODUCTION_STATE="${SATURN_XDMA_SWITCH_SMOKE_PRODUCTION_STATE:-/var/lib/saturn-radio-backend/selection.json}"
XDMA_READY_FILE="${SATURN_XDMA_SWITCH_SMOKE_READY_FILE:-/run/saturn-bridge/xdma-ready.json}"
STATE_GROUP="${SATURN_XDMA_SWITCH_SMOKE_STATE_GROUP:-pi}"
STAGE_PARENT="${SATURN_XDMA_SWITCH_SMOKE_STAGE_PARENT:-/opt/saturn-go}"

TMP_DIR=""
RUNTIME_STAGE_DIR=""
STAGED_BRIDGE_BINARY=""
CONFIG_FILE=""
CLIENT_RESULT=""
CLIENT_ERROR=""
SWITCH_STARTED_AT=""
BACKEND_DROPIN_EXISTED=0
ACCEPTANCE_DROPIN_EXISTED=0
PRODUCTION_STATE_EXISTED=0
READY_FILE_EXISTED=0
RESTORED=0
RESTORE_ARMED=0

log(){ printf '[saturn-xdma-backend-switch-smoke] %s\n' "$*"; }
die(){ printf '[saturn-xdma-backend-switch-smoke] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

service_state(){
  systemctl is-active "$1" 2>/dev/null || true
}

snapshot_file(){
  local source="$1" name="$2" marker_variable="$3"
  if [[ -f "$source" ]]; then
    cp -p -- "$source" "$TMP_DIR/$name"
    printf -v "$marker_variable" '%s' 1
  fi
}

restore_file(){
  local destination="$1" name="$2" existed="$3" mode="$4"
  if (( existed )); then
    install -d -m "$mode" "$(dirname "$destination")"
    cp -p -- "$TMP_DIR/$name" "$destination"
  else
    rm -f -- "$destination"
  fi
}

restore_appliance(){
  local failed=0
  (( RESTORE_ARMED != 0 )) || return 0
  (( RESTORED == 0 )) || return 0
  RESTORED=1
  set +e

  systemctl stop "$BRIDGE_SERVICE" >/dev/null 2>&1 || failed=1
  systemctl start "$P2APP_SERVICE" >/dev/null 2>&1 || failed=1
  restore_file "$BACKEND_DROPIN" backend-dropin "$BACKEND_DROPIN_EXISTED" 0755 \
    || failed=1
  restore_file "$ACCEPTANCE_DROPIN" acceptance-dropin \
    "$ACCEPTANCE_DROPIN_EXISTED" 0755 || failed=1
  restore_file "$PRODUCTION_STATE" production-state \
    "$PRODUCTION_STATE_EXISTED" 0750 || failed=1
  restore_file "$XDMA_READY_FILE" xdma-ready "$READY_FILE_EXISTED" 0755 \
    || failed=1
  systemctl daemon-reload >/dev/null 2>&1 || failed=1
  systemctl start "$BRIDGE_SERVICE" >/dev/null 2>&1 || failed=1

  [[ "$(service_state "$P2APP_SERVICE")" == "active" ]] || failed=1
  [[ "$(service_state "$BRIDGE_SERVICE")" == "active" ]] || failed=1
  [[ "$(service_state "$SATURN_GO_SERVICE")" == "active" ]] || failed=1
  set -e
  (( failed == 0 ))
}

cleanup(){
  local rc="$?" restore_rc=0
  trap - EXIT INT TERM
  restore_appliance || restore_rc=1
  [[ -z "$RUNTIME_STAGE_DIR" ]] || rm -rf -- "$RUNTIME_STAGE_DIR"
  [[ -z "$TMP_DIR" ]] || rm -rf -- "$TMP_DIR"
  if (( rc == 0 && restore_rc != 0 )); then
    rc=1
  fi
  exit "$rc"
}
trap cleanup EXIT INT TERM

(( EUID == 0 )) || die "run this acceptance with sudo"
for command in awk cp date find findmnt install journalctl jq python3 systemctl tail; do
  need_cmd "$command"
done
[[ -x "$BRIDGE_BINARY" ]] || die "bridge binary is not executable: $BRIDGE_BINARY"
[[ -x "$SWITCH_HELPER" ]] || die "backend switch helper is not executable: $SWITCH_HELPER"
[[ -r "$CLIENT" ]] || die "client acceptance script is not readable: $CLIENT"
[[ -d "$STAGE_PARENT" && ! -L "$STAGE_PARENT" ]] \
  || die "bridge staging parent is missing or unsafe: $STAGE_PARENT"
STAGE_MOUNT_OPTIONS="$(findmnt -T "$STAGE_PARENT" -no OPTIONS)" \
  || die "could not inspect the bridge staging filesystem: $STAGE_PARENT"
case ",$STAGE_MOUNT_OPTIONS," in
  *,noexec,*)
    die "bridge staging filesystem is mounted noexec: $STAGE_PARENT"
    ;;
esac

NEWER_SOURCE="$(find "$BRIDGE_ROOT/src" -type f -newer "$BRIDGE_BINARY" -print -quit)"
for build_input in "$BRIDGE_ROOT/Cargo.toml" "$BRIDGE_ROOT/Cargo.lock" "$BRIDGE_ROOT/build.rs"; do
  if [[ -e "$build_input" && "$build_input" -nt "$BRIDGE_BINARY" ]]; then
    NEWER_SOURCE="$build_input"
    break
  fi
done
if [[ -n "$NEWER_SOURCE" ]]; then
  die "standalone bridge binary is stale (newer input: $NEWER_SOURCE)"
fi

[[ "$(service_state "$P2APP_SERVICE")" == "active" ]] \
  || die "$P2APP_SERVICE must be active before the switch acceptance"
[[ "$(service_state "$BRIDGE_SERVICE")" == "active" ]] \
  || die "$BRIDGE_SERVICE must be active before the switch acceptance"
[[ "$(service_state "$SATURN_GO_SERVICE")" == "active" ]] \
  || die "$SATURN_GO_SERVICE must be active before the switch acceptance"

case " $(systemctl show --property=Environment --value "$BRIDGE_SERVICE") " in
  *" SATURN_BRIDGE_RADIO_BACKEND=xdma "*)
    die "refusing to start while the bridge already reports XDMA ownership"
    ;;
esac

TMP_DIR="$(mktemp -d)"
RUNTIME_STAGE_DIR="$(
  mktemp -d "$STAGE_PARENT/.saturn-xdma-backend-switch.XXXXXX"
)"
chmod 0755 "$RUNTIME_STAGE_DIR"
chown root:root "$RUNTIME_STAGE_DIR"
STAGED_BRIDGE_BINARY="$RUNTIME_STAGE_DIR/saturn-bridge"
install -m 0755 -o root -g root "$BRIDGE_BINARY" "$STAGED_BRIDGE_BINARY"
CONFIG_FILE="$TMP_DIR/backend.conf"
CLIENT_RESULT="$TMP_DIR/client-result.json"
CLIENT_ERROR="$TMP_DIR/client-error.log"
snapshot_file "$BACKEND_DROPIN" backend-dropin BACKEND_DROPIN_EXISTED
snapshot_file "$ACCEPTANCE_DROPIN" acceptance-dropin ACCEPTANCE_DROPIN_EXISTED
snapshot_file "$PRODUCTION_STATE" production-state PRODUCTION_STATE_EXISTED
snapshot_file "$XDMA_READY_FILE" xdma-ready READY_FILE_EXISTED
RESTORE_ARMED=1

install -d -m 0755 "$(dirname "$ACCEPTANCE_DROPIN")"
cat >"$ACCEPTANCE_DROPIN" <<EOF
[Service]
RuntimeDirectory=saturn-bridge
RuntimeDirectoryMode=0755
Environment=SATURN_REMOTE_TX_RF_ENABLED=0
ExecStart=
ExecStart=$STAGED_BRIDGE_BINARY
EOF
chmod 0644 "$ACCEPTANCE_DROPIN"
chown root:root "$ACCEPTANCE_DROPIN"

cat >"$CONFIG_FILE" <<EOF
XDMA_OPERATIONAL_ENABLED=1
STATE_FILE=$TMP_DIR/selection.json
TRANSACTION_FILE=$TMP_DIR/transaction.json
LOCK_FILE=/run/lock/saturn-maintenance/radio.lock
SYSTEMD_ROOT=$SYSTEMD_ROOT
BRIDGE_SERVICE=$BRIDGE_SERVICE
P2APP_SERVICE=$P2APP_SERVICE
BRIDGE_DROPIN_NAME=20-radio-backend.conf
READY_TIMEOUT_SECONDS=20
XDMA_READY_FILE=$XDMA_READY_FILE
STATE_GROUP=$STATE_GROUP
EOF
chmod 0600 "$CONFIG_FILE"
chown root:root "$CONFIG_FILE"
systemctl daemon-reload
rm -f -- "$XDMA_READY_FILE"

run_switch(){
  env \
    SATURN_RADIO_BACKEND_CONFIG="$CONFIG_FILE" \
    SATURN_RADIO_BACKEND_TEST_MODE=1 \
    "$SWITCH_HELPER" switch "$1"
}

show_switch_failure_diagnostics(){
  log "Backend switch diagnostics:"
  systemctl show "$BRIDGE_SERVICE" \
    --property=ActiveState \
    --property=SubState \
    --property=Result \
    --property=NRestarts \
    --property=ExecStart \
    --no-pager >&2 || true
  if [[ -f "$XDMA_READY_FILE" ]]; then
    jq '{backend,status,rf_safe,error,updated_at_ms,metrics}' \
      "$XDMA_READY_FILE" >&2 || true
  else
    log "No XDMA readiness file was published at $XDMA_READY_FILE"
  fi
  journalctl -u "$BRIDGE_SERVICE" \
    --since "$SWITCH_STARTED_AT" \
    --no-pager -o short-iso 2>/dev/null \
    | awk '!/saturn-bridge: diag / && !/saturn-bridge: vfoA=/' \
    | tail -n 80 >&2 || true
}

log "Switching P2 -> direct XDMA through the transactional test gate"
SWITCH_STARTED_AT="$(date --iso-8601=seconds)"
if ! run_switch xdma; then
  show_switch_failure_diagnostics
  die "transactional switch to direct XDMA failed"
fi
[[ "$(service_state "$P2APP_SERVICE")" != "active" ]] \
  || die "P2 retained hardware ownership after the XDMA switch"
[[ "$(service_state "$BRIDGE_SERVICE")" == "active" ]] \
  || die "bridge did not start with direct XDMA ownership"
jq -e '
  .backend == "xdma" and
  .status == "ready" and
  .rf_safe == true and
  .metrics.rf_safe == true and
  .metrics.tx_capable == true and
  .metrics.dma_reads >= 4 and
  .metrics.iq_pairs >= 1024
' "$XDMA_READY_FILE" >/dev/null \
  || die "transaction did not produce receive-safe direct-XDMA readiness"

session="xdma-switch-$$-$RANDOM"
if ! python3 "$CLIENT" \
  --url "$PROXY_BASE_URL/saturn/control?session=$session" \
  --media-url "$PROXY_BASE_URL/saturn/media?session=$session" \
  --readiness-file "$XDMA_READY_FILE" \
  --retune-hz 7200000 \
  --timeout-seconds 15 \
  --tx-cycles 5 \
  --basic-auth-systemd-unit "$SATURN_GO_SERVICE" \
  --insecure-tls \
  >"$CLIENT_RESULT" 2>"$CLIENT_ERROR"
then
  sed 's/^/[saturn-xdma-backend-switch-smoke] client: /' "$CLIENT_ERROR" >&2
  die "split-proxy client acceptance failed"
fi
jq -e '
  .status == "passed" and
  .transport == "split-proxy" and
  .split_paired == true and
  .control_text_messages > 0 and
  .media_binary_messages > 0 and
  .bridge_ready == true and
  .dsp_burst_continued == true and
  .rf_inhibited_duc_exercised == true and
  .tx_cycles_requested == 5 and
  .tx_cycles_completed == 5 and
  ([.tx_cycle_results[] | select(.dma_writes > 0 and .frames >= 20 and .fifo_faults == 0 and .mux_resets >= 1)] | length) == 5 and
  .iq_nonzero == true and
  .iq_frames >= 3 and
  .audio_frames >= 3
' "$CLIENT_RESULT" >/dev/null \
  || die "split-proxy client result is incomplete"

log "Switching direct XDMA -> P2 through the same transaction"
run_switch p2
[[ "$(service_state "$P2APP_SERVICE")" == "active" ]] \
  || die "P2 did not reacquire hardware ownership"
[[ "$(service_state "$BRIDGE_SERVICE")" == "active" ]] \
  || die "bridge did not restart on the P2 backend"
jq -e '
  .requested == "p2" and
  .active == "p2" and
  .status == "ready"
' "$TMP_DIR/selection.json" >/dev/null \
  || die "transaction state did not persist the test-gated P2 restoration"

restore_appliance || die "the exact pre-test appliance state could not be restored"
log "Client acceptance result:"
jq . "$CLIENT_RESULT"
log "Transactional P2 -> XDMA -> P2 split-proxy acceptance passed"
log "RF remained inhibited and the prior appliance selection was restored"
