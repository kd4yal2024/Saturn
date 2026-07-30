#!/usr/bin/env bash
set -Eeuo pipefail

# Exercise the operational receive-only direct-XDMA backend for a bounded
# interval. The source-tree binary must already be current; this script never
# builds, installs, or enables the production XDMA backend-selection policy.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="${SATURN_XDMA_RX_SMOKE_REPO_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd -P)}"
BRIDGE_ROOT="$REPO_ROOT/update_manager/saturn-bridge"
BRIDGE_BINARY="${SATURN_XDMA_RX_SMOKE_BRIDGE_BINARY:-$BRIDGE_ROOT/target/debug/saturn-bridge}"
DURATION_SECONDS="${SATURN_XDMA_RX_SMOKE_DURATION_SECONDS:-15}"
READY_WAIT_SECONDS="${SATURN_XDMA_RX_SMOKE_READY_WAIT_SECONDS:-}"
READY_FILE="${SATURN_XDMA_RX_SMOKE_READY_FILE:-/tmp/saturn-xdma-ready.json}"
OBSERVED_FILE="${SATURN_XDMA_RX_SMOKE_OBSERVED_FILE:-/tmp/saturn-xdma-ready-observed.json}"
LOG_FILE="${SATURN_XDMA_RX_SMOKE_LOG_FILE:-/tmp/saturn-xdma-operational-rx-smoke.log}"
CLIENT_PROBE_SCRIPT="${SATURN_XDMA_RX_SMOKE_CLIENT_PROBE_SCRIPT:-$SCRIPT_DIR/saturn-xdma-operational-client.py}"
CLIENT_RESULT_FILE="${SATURN_XDMA_RX_SMOKE_CLIENT_RESULT_FILE:-/tmp/saturn-xdma-operational-client-result.json}"
CLIENT_ERROR_FILE="${SATURN_XDMA_RX_SMOKE_CLIENT_ERROR_FILE:-/tmp/saturn-xdma-operational-client-error.log}"
MIN_STREAM_RATE=188160
MAX_STREAM_RATE=195840

CLIENT_PROBE=0
P2_WAS=""
BRIDGE_WAS=""
WATCHER_PID=""
RESTORED=0
SERVICES_CAPTURED=0

log(){ printf '[saturn-xdma-operational-rx-smoke] %s\n' "$*"; }
die(){ printf '[saturn-xdma-operational-rx-smoke] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage: sudo saturn-xdma-operational-rx-smoke.sh \
  [--duration-seconds SECONDS] [--client-probe]

Runs the source-tree direct-XDMA RX backend for a bounded interval, requires
advancing DMA/IQ readiness, verifies receive-safe shutdown, and restores the
prior p2app.service and saturn-bridge.service activity.

The client probe requires at least 45 seconds. It connects directly to the TCI
endpoint, verifies IQ and audio frames, retunes to 7.200 MHz, requires a TX
request to be refused, and disconnects before runtime cleanup.

Build the standalone debug binary before running this test:
  CARGO_BUILD_JOBS=1 cargo build \
    --manifest-path update_manager/saturn-bridge/Cargo.toml
EOF
}

positive_integer(){
  [[ "$2" =~ ^[1-9][0-9]*$ ]] || die "$1 must be a positive integer, got: $2"
}

service_state(){
  systemctl is-active "$1" 2>/dev/null || true
}

restore_services(){
  local failed=0
  (( SERVICES_CAPTURED != 0 )) || return 0
  (( RESTORED == 0 )) || return 0

  if [[ "$P2_WAS" == "active" ]]; then
    systemctl start p2app.service || failed=1
  fi
  if [[ "$BRIDGE_WAS" == "active" ]]; then
    systemctl start saturn-bridge.service || failed=1
  fi

  if [[ "$P2_WAS" == "active" && "$(service_state p2app.service)" != "active" ]]; then
    printf '[saturn-xdma-operational-rx-smoke] ERROR: p2app.service was not restored\n' >&2
    failed=1
  fi
  if [[ "$BRIDGE_WAS" == "active" && "$(service_state saturn-bridge.service)" != "active" ]]; then
    printf '[saturn-xdma-operational-rx-smoke] ERROR: saturn-bridge.service was not restored\n' >&2
    failed=1
  fi
  if [[ "$P2_WAS" != "active" && "$(service_state p2app.service)" == "active" ]]; then
    printf '[saturn-xdma-operational-rx-smoke] ERROR: p2app.service was unexpectedly activated\n' >&2
    failed=1
  fi
  if [[ "$BRIDGE_WAS" != "active" && "$(service_state saturn-bridge.service)" == "active" ]]; then
    printf '[saturn-xdma-operational-rx-smoke] ERROR: saturn-bridge.service was unexpectedly activated\n' >&2
    failed=1
  fi

  (( failed == 0 )) || return 1
  RESTORED=1
}

cleanup(){
  local rc="$?"
  local restore_rc=0
  trap - EXIT INT TERM
  set +e

  if [[ -n "$WATCHER_PID" ]]; then
    kill -TERM "$WATCHER_PID" >/dev/null 2>&1 || true
    wait "$WATCHER_PID" >/dev/null 2>&1 || true
    WATCHER_PID=""
  fi
  restore_services || restore_rc=1
  if (( rc == 0 && restore_rc != 0 )); then
    rc="$restore_rc"
  fi
  exit "$rc"
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --duration-seconds)
      [[ $# -ge 2 ]] || die "--duration-seconds requires a value"
      DURATION_SECONDS="$2"
      shift 2
      ;;
    --client-probe)
      CLIENT_PROBE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

if [[ -z "$READY_WAIT_SECONDS" ]]; then
  READY_WAIT_SECONDS="$DURATION_SECONDS"
fi
positive_integer "duration" "$DURATION_SECONDS"
positive_integer "readiness wait" "$READY_WAIT_SECONDS"
(( DURATION_SECONDS >= 5 && DURATION_SECONDS <= 300 )) \
  || die "duration must be between 5 and 300 seconds"
(( READY_WAIT_SECONDS >= 5 && READY_WAIT_SECONDS <= DURATION_SECONDS )) \
  || die "readiness wait must be between 5 seconds and the test duration"
if (( CLIENT_PROBE )); then
  (( DURATION_SECONDS >= 45 )) \
    || die "--client-probe requires a duration of at least 45 seconds"
fi

(( EUID == 0 )) || die "run this smoke test with sudo"
need_cmd cp
need_cmd find
need_cmd grep
need_cmd jq
need_cmd systemctl
need_cmd tee
need_cmd timeout
[[ -x "$BRIDGE_BINARY" ]] || die "bridge binary is not executable: $BRIDGE_BINARY"
if (( CLIENT_PROBE )); then
  need_cmd python3
  [[ -r "$CLIENT_PROBE_SCRIPT" ]] \
    || die "client probe is not readable: $CLIENT_PROBE_SCRIPT"
fi

NEWER_SOURCE="$(find "$BRIDGE_ROOT/src" -type f -newer "$BRIDGE_BINARY" -print -quit)"
for build_input in \
  "$BRIDGE_ROOT/Cargo.toml" \
  "$BRIDGE_ROOT/Cargo.lock" \
  "$BRIDGE_ROOT/build.rs"
do
  if [[ -e "$build_input" && "$build_input" -nt "$BRIDGE_BINARY" ]]; then
    NEWER_SOURCE="$build_input"
    break
  fi
done
if [[ -n "$NEWER_SOURCE" ]]; then
  die "standalone bridge binary is stale (newer input: $NEWER_SOURCE). Run: CARGO_BUILD_JOBS=1 cargo build --manifest-path update_manager/saturn-bridge/Cargo.toml"
fi

P2_WAS="$(service_state p2app.service)"
BRIDGE_WAS="$(service_state saturn-bridge.service)"
SERVICES_CAPTURED=1

log "Prior services: p2app=$P2_WAS saturn-bridge=$BRIDGE_WAS"
systemctl stop saturn-bridge.service p2app.service
[[ "$(service_state saturn-bridge.service)" != "active" ]] \
  || die "saturn-bridge.service retained ownership after stop"
[[ "$(service_state p2app.service)" != "active" ]] \
  || die "p2app.service retained ownership after stop"

rm -f -- \
  "$READY_FILE" \
  "$OBSERVED_FILE" \
  "$LOG_FILE" \
  "$CLIENT_RESULT_FILE" \
  "$CLIENT_ERROR_FILE"

(
  deadline="$((SECONDS + READY_WAIT_SECONDS))"
  while (( SECONDS < deadline )); do
    if jq -e '
      .backend == "xdma" and
      .status == "ready" and
      .rf_safe == true and
      .metrics.rf_safe == true and
      .metrics.tx_capable == false and
      .metrics.dma_reads >= 4 and
      .metrics.iq_pairs >= 1024
    ' "$READY_FILE" >/dev/null 2>&1; then
      cp -- "$READY_FILE" "$OBSERVED_FILE"
      if (( CLIENT_PROBE )); then
        if ! python3 "$CLIENT_PROBE_SCRIPT" \
          --url ws://127.0.0.1:50001/ \
          --readiness-file "$READY_FILE" \
          --retune-hz 7200000 \
          --timeout-seconds 15 \
          >"$CLIENT_RESULT_FILE" 2>"$CLIENT_ERROR_FILE"
        then
          sed 's/^/[saturn-xdma-operational-rx-smoke] client: /' \
            "$CLIENT_ERROR_FILE" >&2
          exit 1
        fi
      fi
      exit 0
    fi
    sleep 0.25
  done
  exit 1
) &
WATCHER_PID="$!"

log "Starting ${DURATION_SECONDS}s RX-only direct-XDMA runtime"
set +e
env \
  SATURN_BRIDGE_RADIO_BACKEND=xdma \
  SATURN_BRIDGE_XDMA_READY_PATH="$READY_FILE" \
  timeout --signal=TERM --kill-after=3s "${DURATION_SECONDS}s" \
  "$BRIDGE_BINARY" 2>&1 | tee "$LOG_FILE"
RUNTIME_RC="${PIPESTATUS[0]}"
set -e

set +e
wait "$WATCHER_PID"
WATCHER_RC="$?"
set -e
WATCHER_PID=""

[[ "$RUNTIME_RC" -eq 124 ]] \
  || die "runtime exited with $RUNTIME_RC; expected bounded timeout status 124"
[[ "$WATCHER_RC" -eq 0 ]] \
  || die "runtime readiness or client acceptance did not complete"
[[ -s "$OBSERVED_FILE" ]] || die "ready-state observation was not retained"

jq -e '
  .backend == "xdma" and
  .status == "stopped" and
  .rf_safe == true and
  .error == null and
  .metrics.rf_safe == true and
  .metrics.dma_reads >= 4 and
  .metrics.iq_pairs >= 1024
' "$READY_FILE" >/dev/null \
  || die "final readiness does not prove receive-safe stopped cleanup"

grep -Fq 'direct XDMA RX backend ready' "$LOG_FILE" \
  || die "runtime log is missing the direct-XDMA ready marker"
grep -Fq 'direct XDMA RX backend stopped; DDC disabled and receive-safe cleanup verified' \
  "$LOG_FILE" || die "runtime log is missing the receive-safe cleanup marker"

if (( CLIENT_PROBE )); then
  jq -e '
    .status == "passed" and
    .bridge_ready == true and
    .remote_tx_rf_enabled == false and
    .tx_request_refused == true and
    .dsp_burst_continued == true and
    .retune_hz == 7200000 and
    .iq_frames >= 3 and
    .iq_pairs > 0 and
    .iq_nonzero == true and
    .audio_frames >= 3 and
    .audio_samples > 0
  ' "$CLIENT_RESULT_FILE" >/dev/null \
    || die "client acceptance result is incomplete"
  grep -Fq 'refusing TX request: operational direct XDMA backend is RX-only' \
    "$LOG_FILE" || die "runtime log is missing the RX-only TX refusal"
  grep -Fq 'TCI websocket client' "$LOG_FILE" \
    || die "runtime log is missing the client connection"
  grep -Fq 'disconnected from' "$LOG_FILE" \
    || die "runtime log is missing the client disconnect"
fi

READY_UPDATED_MS="$(jq -r '.updated_at_ms' "$OBSERVED_FILE")"
STOPPED_UPDATED_MS="$(jq -r '.updated_at_ms' "$READY_FILE")"
READY_IQ_PAIRS="$(jq -r '.metrics.iq_pairs' "$OBSERVED_FILE")"
STOPPED_IQ_PAIRS="$(jq -r '.metrics.iq_pairs' "$READY_FILE")"
STREAM_ELAPSED_MS="$((STOPPED_UPDATED_MS - READY_UPDATED_MS))"
STREAM_IQ_PAIRS="$((STOPPED_IQ_PAIRS - READY_IQ_PAIRS))"
(( STREAM_ELAPSED_MS > 0 && STREAM_IQ_PAIRS > 0 )) \
  || die "readiness timestamps or IQ counters did not advance"
STREAM_RATE="$((STREAM_IQ_PAIRS * 1000 / STREAM_ELAPSED_MS))"
(( STREAM_RATE >= MIN_STREAM_RATE && STREAM_RATE <= MAX_STREAM_RATE )) \
  || die "steady-state IQ rate ${STREAM_RATE}/s is outside the 192 kHz +/-2% acceptance band"

restore_services || die "could not restore the prior service activity"

log "Steady-state IQ rate: ${STREAM_RATE} pairs/s over ${STREAM_ELAPSED_MS}ms"
if (( CLIENT_PROBE )); then
  log "Client acceptance result:"
  jq . "$CLIENT_RESULT_FILE"
fi
log "Observed ready state:"
jq '{status,rf_safe,metrics:(.metrics | {frequency_hz,sample_rate_hz,dma_reads,dma_bytes,iq_pairs,fifo_hwm,header_resync,header_errors,tx_capable})}' \
  "$OBSERVED_FILE"
log "Final stopped state:"
jq '{status,rf_safe,error,metrics}' "$READY_FILE"
log "Operational direct-XDMA RX smoke test passed; prior services restored"
