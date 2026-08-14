#!/usr/bin/env bash
set -Eeuo pipefail

# Exercise the operational direct-XDMA backend for a bounded
# interval. The source-tree binary must already be current; this script never
# builds, installs, or enables the production XDMA backend-selection policy.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="${SATURN_XDMA_RX_SMOKE_REPO_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd -P)}"
BRIDGE_ROOT="$REPO_ROOT/update_manager/saturn-bridge"
BRIDGE_BINARY="${SATURN_XDMA_RX_SMOKE_BRIDGE_BINARY:-}"
DURATION_SECONDS="${SATURN_XDMA_RX_SMOKE_DURATION_SECONDS:-15}"
READY_WAIT_SECONDS="${SATURN_XDMA_RX_SMOKE_READY_WAIT_SECONDS:-}"
READY_FILE="${SATURN_XDMA_RX_SMOKE_READY_FILE:-/tmp/saturn-xdma-ready.json}"
OBSERVED_FILE="${SATURN_XDMA_RX_SMOKE_OBSERVED_FILE:-/tmp/saturn-xdma-ready-observed.json}"
RATE_START_FILE="${SATURN_XDMA_RX_SMOKE_RATE_START_FILE:-/tmp/saturn-xdma-rate-start.json}"
LOG_FILE="${SATURN_XDMA_RX_SMOKE_LOG_FILE:-/tmp/saturn-xdma-operational-rx-smoke.log}"
CLIENT_PROBE_SCRIPT="${SATURN_XDMA_RX_SMOKE_CLIENT_PROBE_SCRIPT:-$SCRIPT_DIR/saturn-xdma-operational-client.py}"
CLIENT_RESULT_FILE="${SATURN_XDMA_RX_SMOKE_CLIENT_RESULT_FILE:-/tmp/saturn-xdma-operational-client-result.json}"
CLIENT_ERROR_FILE="${SATURN_XDMA_RX_SMOKE_CLIENT_ERROR_FILE:-/tmp/saturn-xdma-operational-client-error.log}"
VALIDATION_FILE="${SATURN_XDMA_RX_SMOKE_VALIDATION_FILE:-/var/lib/saturn-state/xdma-telemetry.json}"
PROXY_BASE_URL="${SATURN_XDMA_RX_SMOKE_PROXY_BASE_URL:-wss://127.0.0.1:8443}"
SATURN_GO_SERVICE="${SATURN_XDMA_RX_SMOKE_SATURN_GO_SERVICE:-saturn-go.service}"
TARGET_STREAM_RATE=384000
MIN_STREAM_RATE=376320
MAX_STREAM_RATE=391680

CLIENT_PROBE=0
PROXY_CLIENT_PROBE=0
RF_TX_PROBE=0
TX_CYCLES=5
TX_CYCLES_EXPLICIT=0
RF_TX_CONFIRM_TOKEN="DUMMY_LOAD_CONNECTED_ANT1_7200000HZ_3W"
P2_WAS=""
BRIDGE_WAS=""
SATURN_GO_WAS=""
SATURN_GO_MANAGED=0
WATCHER_PID=""
RESTORED=0
SERVICES_CAPTURED=0

log(){ printf '[saturn-xdma-operational-rx-smoke] %s\n' "$*"; }
die(){ printf '[saturn-xdma-operational-rx-smoke] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage: sudo saturn-xdma-operational-rx-smoke.sh \
  [--duration-seconds SECONDS] [--client-probe | --proxy-client-probe] \
  [--tx-cycles COUNT] [--rf-tx-probe]

Runs the source-tree direct-XDMA RX backend for a bounded interval, requires
advancing DMA/IQ readiness, verifies receive-safe shutdown, and restores the
prior p2app.service and saturn-bridge.service activity.

The client probe requires at least 45 seconds. It connects directly to the TCI
endpoint, verifies IQ and audio frames, retunes to 7.200 MHz, requires a TX
request to exercise the RF-inhibited DUC, and disconnects before cleanup.
By default it performs five complete TX arm/stream/disarm cycles so stale DUC
mux state and second-key regressions are covered.

The proxy client probe performs the same acceptance through Saturn Go's
authenticated TLS split control/media WebSockets. It reads the configured
credential from saturn-go.service without printing it and accepts the
appliance's localhost self-signed certificate only for this bounded test.

--rf-tx-probe changes the client probe to a 2.5-second production RF test,
locked to 7.200 MHz, ANT1, and 3 W. It requires this exact environment token:
  SATURN_XDMA_PRODUCTION_TX_CONFIRM=DUMMY_LOAD_CONNECTED_ANT1_7200000HZ_3W

RX-only smoke tests may use the debug binary. Client acceptance exercises the
real-time DUC path and therefore defaults to the optimized release binary:
  CARGO_BUILD_JOBS=1 cargo build --release \
    --manifest-path update_manager/saturn-bridge/Cargo.toml
EOF
}

positive_integer(){
  [[ "$2" =~ ^[1-9][0-9]*$ ]] || die "$1 must be a positive integer, got: $2"
}

persist_validation(){
  python3 - \
    "$VALIDATION_FILE" "$OBSERVED_FILE" "$RATE_START_FILE" "$READY_FILE" "$CLIENT_RESULT_FILE" \
    "$STREAM_RATE" "$STREAM_ELAPSED_MS" "$TX_CYCLES" <<'PY'
import json
import os
import sys
import tempfile
import time

path, observed_path, rate_start_path, stopped_path, client_path, rate, elapsed_ms, cycles = sys.argv[1:]
with open(observed_path, encoding="utf-8") as handle:
    observed = json.load(handle)
with open(rate_start_path, encoding="utf-8") as handle:
    rate_start = json.load(handle)
with open(stopped_path, encoding="utf-8") as handle:
    stopped = json.load(handle)
client = None
try:
    with open(client_path, encoding="utf-8") as handle:
        client = json.load(handle)
except FileNotFoundError:
    pass

document = {
    "schema_version": 1,
    "updated_at_ms": int(time.time() * 1000),
    "source": "saturn-xdma-operational-rx-smoke",
    "phase": 7,
    "probe": (
        "operational-rx-tx-repeated-key" if client is not None else "operational-rx"
    ),
    "status": "passed",
    "cleanup": "receive-safe-services-restored",
    "error": None,
    "metrics": {
        "steady_iq_pairs_per_second": int(rate),
        "steady_interval_ms": int(elapsed_ms),
        "tx_cycles_requested": int(cycles) if client is not None else 0,
        "tx_cycles_completed": int((client or {}).get("tx_cycles_completed", 0)),
        "rx_dma_reads": int(stopped.get("metrics", {}).get("dma_reads", 0)),
        "rx_iq_pairs": int(stopped.get("metrics", {}).get("iq_pairs", 0)),
        "rf_safe": stopped.get("rf_safe") is True,
    },
    "client": client,
    "observed_ready": observed,
    "steady_rate_start": rate_start,
    "final_stopped": stopped,
}
directory = os.path.dirname(path)
os.makedirs(directory, mode=0o755, exist_ok=True)
fd, temporary = tempfile.mkstemp(prefix=".xdma-validation-", dir=directory)
try:
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump(document, handle, indent=2, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    os.chmod(temporary, 0o644)
    os.replace(temporary, path)
finally:
    try:
        os.unlink(temporary)
    except FileNotFoundError:
        pass
PY
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
  if (( SATURN_GO_MANAGED )) && [[ "$SATURN_GO_WAS" == "active" ]]; then
    systemctl start "$SATURN_GO_SERVICE" || failed=1
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
  if (( SATURN_GO_MANAGED )); then
    if [[ "$SATURN_GO_WAS" == "active" && "$(service_state "$SATURN_GO_SERVICE")" != "active" ]]; then
      printf '[saturn-xdma-operational-rx-smoke] ERROR: %s was not restored\n' "$SATURN_GO_SERVICE" >&2
      failed=1
    fi
    if [[ "$SATURN_GO_WAS" != "active" && "$(service_state "$SATURN_GO_SERVICE")" == "active" ]]; then
      printf '[saturn-xdma-operational-rx-smoke] ERROR: %s was unexpectedly activated\n' "$SATURN_GO_SERVICE" >&2
      failed=1
    fi
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
    --proxy-client-probe)
      CLIENT_PROBE=1
      PROXY_CLIENT_PROBE=1
      shift
      ;;
    --rf-tx-probe)
      CLIENT_PROBE=1
      RF_TX_PROBE=1
      shift
      ;;
    --tx-cycles)
      [[ $# -ge 2 ]] || die "--tx-cycles requires a value"
      TX_CYCLES="$2"
      TX_CYCLES_EXPLICIT=1
      shift 2
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

if (( RF_TX_PROBE && ! TX_CYCLES_EXPLICIT )); then
  TX_CYCLES=1
fi

if [[ -z "$BRIDGE_BINARY" ]]; then
  if (( CLIENT_PROBE )); then
    BRIDGE_BINARY="$BRIDGE_ROOT/target/release/saturn-bridge"
  else
    BRIDGE_BINARY="$BRIDGE_ROOT/target/debug/saturn-bridge"
  fi
fi

BUILD_PROFILE_ARG=""
if [[ "$BRIDGE_BINARY" == "$BRIDGE_ROOT/target/release/saturn-bridge" ]]; then
  BUILD_PROFILE_ARG=" --release"
fi

if [[ -z "$READY_WAIT_SECONDS" ]]; then
  READY_WAIT_SECONDS="$DURATION_SECONDS"
fi
positive_integer "duration" "$DURATION_SECONDS"
positive_integer "readiness wait" "$READY_WAIT_SECONDS"
positive_integer "TX cycles" "$TX_CYCLES"
(( DURATION_SECONDS >= 5 && DURATION_SECONDS <= 300 )) \
  || die "duration must be between 5 and 300 seconds"
(( READY_WAIT_SECONDS >= 5 && READY_WAIT_SECONDS <= DURATION_SECONDS )) \
  || die "readiness wait must be between 5 seconds and the test duration"
if (( CLIENT_PROBE )); then
  (( DURATION_SECONDS >= 45 )) \
    || die "--client-probe requires a duration of at least 45 seconds"
  [[ "$BRIDGE_BINARY" != "$BRIDGE_ROOT/target/debug/saturn-bridge" ]] \
    || die "--client-probe requires an optimized bridge binary; build target/release/saturn-bridge or set SATURN_XDMA_RX_SMOKE_BRIDGE_BINARY to an optimized staged binary"
fi
(( TX_CYCLES <= 20 )) || die "TX cycles must not exceed 20"
if (( RF_TX_PROBE )); then
  [[ "${SATURN_XDMA_PRODUCTION_TX_CONFIRM:-}" == "$RF_TX_CONFIRM_TOKEN" ]] \
    || die "--rf-tx-probe requires SATURN_XDMA_PRODUCTION_TX_CONFIRM=$RF_TX_CONFIRM_TOKEN"
  [[ "$BRIDGE_BINARY" == "$BRIDGE_ROOT/target/release/saturn-bridge" ]] \
    || die "--rf-tx-probe requires the current optimized binary: $BRIDGE_ROOT/target/release/saturn-bridge"
  (( TX_CYCLES == 1 )) \
    || die "--rf-tx-probe permits exactly one TX cycle; pass --tx-cycles 1"
fi
if (( RF_TX_PROBE && PROXY_CLIENT_PROBE )); then
  die "--rf-tx-probe uses the direct localhost TCI lane and cannot be combined with --proxy-client-probe"
fi

(( EUID == 0 )) || die "run this smoke test with sudo"
need_cmd cp
need_cmd find
need_cmd grep
need_cmd jq
need_cmd python3
need_cmd systemctl
need_cmd tee
need_cmd timeout
[[ -x "$BRIDGE_BINARY" ]] || die "bridge binary is not executable: $BRIDGE_BINARY"
if (( CLIENT_PROBE )); then
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
  die "standalone bridge binary is stale (newer input: $NEWER_SOURCE). Run: CARGO_BUILD_JOBS=1 cargo build$BUILD_PROFILE_ARG --manifest-path update_manager/saturn-bridge/Cargo.toml"
fi

P2_WAS="$(service_state p2app.service)"
BRIDGE_WAS="$(service_state saturn-bridge.service)"
SATURN_GO_WAS="$(service_state "$SATURN_GO_SERVICE")"
SERVICES_CAPTURED=1

if (( PROXY_CLIENT_PROBE )) && [[ "$SATURN_GO_WAS" != "active" ]]; then
  die "$SATURN_GO_SERVICE must be active for --proxy-client-probe"
fi

log "Prior services: p2app=$P2_WAS saturn-bridge=$BRIDGE_WAS saturn-go=$SATURN_GO_WAS"
log "Bridge binary: $BRIDGE_BINARY"
if (( RF_TX_PROBE )); then
  SATURN_GO_MANAGED=1
  systemctl stop "$SATURN_GO_SERVICE"
  [[ "$(service_state "$SATURN_GO_SERVICE")" != "active" ]] \
    || die "$SATURN_GO_SERVICE retained proxy/client access during the RF probe"
fi
systemctl stop saturn-bridge.service p2app.service
[[ "$(service_state saturn-bridge.service)" != "active" ]] \
  || die "saturn-bridge.service retained ownership after stop"
[[ "$(service_state p2app.service)" != "active" ]] \
  || die "p2app.service retained ownership after stop"

rm -f -- \
  "$READY_FILE" \
  "$OBSERVED_FILE" \
  "$RATE_START_FILE" \
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
      .metrics.tx_capable == true and
      .metrics.dma_reads >= 4 and
      .metrics.iq_pairs >= 1024
    ' "$READY_FILE" >/dev/null 2>&1; then
      cp -- "$READY_FILE" "$OBSERVED_FILE"
      if (( CLIENT_PROBE )); then
        client_args=(
          --readiness-file "$READY_FILE"
          --retune-hz 7200000
          --timeout-seconds 15
          --tx-cycles "$TX_CYCLES"
        )
        if (( RF_TX_PROBE )); then
          client_args+=(
            --rf-tx-probe
            --tx-duration-ms 2500
            --tx-drive-watts 3
          )
        fi
        if (( PROXY_CLIENT_PROBE )); then
          proxy_session="xdma-acceptance-$$-$RANDOM"
          client_args+=(
            --url "$PROXY_BASE_URL/saturn/control?session=$proxy_session"
            --media-url "$PROXY_BASE_URL/saturn/media?session=$proxy_session"
            --basic-auth-systemd-unit "$SATURN_GO_SERVICE"
            --insecure-tls
          )
        else
          client_args+=(--url ws://127.0.0.1:50001/)
        fi
        if ! python3 "$CLIENT_PROBE_SCRIPT" "${client_args[@]}" \
          >"$CLIENT_RESULT_FILE" 2>"$CLIENT_ERROR_FILE"
        then
          sed 's/^/[saturn-xdma-operational-rx-smoke] client: /' \
            "$CLIENT_ERROR_FILE" >&2
          exit 1
        fi
        rate_deadline="$((SECONDS + 5))"
        while (( SECONDS < rate_deadline )); do
          if jq -e '
            .backend == "xdma" and
            .status == "ready" and
            .rf_safe == true and
            .metrics.rf_safe == true and
            .metrics.tx_stream_active == false and
            .metrics.tx_keyed == false
          ' "$READY_FILE" >/dev/null 2>&1; then
            # Exclude half-duplex TX and its release transition from the
            # receive-only 384 kHz rate gate.
            sleep 1
            if jq -e '
              .backend == "xdma" and
              .status == "ready" and
              .rf_safe == true and
              .metrics.rf_safe == true and
              .metrics.tx_stream_active == false and
              .metrics.tx_keyed == false
            ' "$READY_FILE" >/dev/null 2>&1; then
              cp -- "$READY_FILE" "$RATE_START_FILE"
              exit 0
            fi
          fi
          sleep 0.25
        done
        exit 1
      else
        cp -- "$OBSERVED_FILE" "$RATE_START_FILE"
      fi
      exit 0
    fi
    sleep 0.25
  done
  exit 1
) &
WATCHER_PID="$!"

if (( RF_TX_PROBE )); then
  log "Starting ${DURATION_SECONDS}s direct-XDMA runtime with bounded 7.200 MHz ANT1 RF probe (3 W maximum, 2.5s)"
else
  log "Starting ${DURATION_SECONDS}s RF-inhibited direct-XDMA RX/TX runtime"
fi
set +e
env \
  SATURN_BRIDGE_RADIO_BACKEND=xdma \
  SATURN_REMOTE_TX_RF_ENABLED="$RF_TX_PROBE" \
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
if (( PROXY_CLIENT_PROBE )) && [[ "$(service_state "$SATURN_GO_SERVICE")" != "active" ]]; then
  die "$SATURN_GO_SERVICE stopped during proxy client acceptance"
fi
[[ -s "$OBSERVED_FILE" ]] || die "ready-state observation was not retained"
[[ -s "$RATE_START_FILE" ]] || die "steady receive-rate start state was not retained"

jq -e --argjson target_rate "$TARGET_STREAM_RATE" '
  .metrics.sample_rate_hz == $target_rate
' "$OBSERVED_FILE" >/dev/null \
  || die "direct XDMA readiness did not report the required ${TARGET_STREAM_RATE} Hz IQ rate"

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
grep -Fq 'direct XDMA backend stopped; DDC and DUC disabled and receive-safe cleanup verified' \
  "$LOG_FILE" || die "runtime log is missing the receive-safe cleanup marker"

if (( CLIENT_PROBE )); then
  if (( RF_TX_PROBE )); then
    jq -e '
      .status == "passed" and
      .bridge_ready == true and
      .split_paired == true and
      .remote_tx_rf_enabled == true and
      .rf_tx_exercised == true and
      .tx_keyed_observed == true and
      .tx_duration_ms == 2500 and
      .tx_drive_watts == 3 and
      .peak_forward_watts >= 0.05 and
      .peak_forward_watts <= 4.0 and
      .peak_reverse_watts <= 0.75 and
      .peak_swr <= 3.0 and
      .dsp_burst_continued == true and
      .retune_hz == 7200000 and
      .iq_nonzero == true and
      .audio_nonzero == true
    ' "$CLIENT_RESULT_FILE" >/dev/null \
      || die "production RF client acceptance result is incomplete"
  else
    jq -e '
      .status == "passed" and
      .bridge_ready == true and
      .split_paired == true and
      .remote_tx_rf_enabled == false and
      .control_role == "operator" and
      .rf_inhibited_duc_exercised == true and
      .tx_cycles_completed == .tx_cycles_requested and
      ([.tx_cycle_results[] | select(.dma_writes > 0 and .frames >= 20 and .fifo_faults == 0 and .mux_resets >= 1)] | length) == .tx_cycles_completed and
      .dsp_burst_continued == true and
      .retune_hz == 7200000 and
      .iq_frames >= 3 and
      .iq_pairs > 0 and
      .iq_nonzero == true and
      .audio_frames >= 3 and
      .audio_samples > 0
    ' "$CLIENT_RESULT_FILE" >/dev/null \
      || die "client acceptance result is incomplete"
  fi
  if (( PROXY_CLIENT_PROBE )); then
    jq -e '
      .transport == "split-proxy" and
      .control_text_messages > 0 and
      .media_binary_messages > 0
    ' "$CLIENT_RESULT_FILE" >/dev/null \
      || die "client did not validate the split proxy transport"
  else
    jq -e '.transport == "direct"' "$CLIENT_RESULT_FILE" >/dev/null \
      || die "client did not validate the direct transport"
  fi
  if (( RF_TX_PROBE )); then
    grep -Fq 'TX state -> ON' "$LOG_FILE" \
      || die "runtime log is missing the production TX key marker"
    grep -Fq 'TX state -> OFF' "$LOG_FILE" \
      || die "runtime log is missing the production TX release marker"
    if grep -Eq 'power trip|TX output fault forced receive state' "$LOG_FILE"; then
      die "production TX reported a safety trip"
    fi
  else
    grep -Fq 'TX armed with RF disabled; holding off key' \
      "$LOG_FILE" || die "runtime log is missing RF-inhibited DUC activity"
  fi
  grep -Fq 'TCI websocket client' "$LOG_FILE" \
    || die "runtime log is missing the client connection"
  grep -Fq 'disconnected from' "$LOG_FILE" \
    || die "runtime log is missing the client disconnect"
fi

READY_UPDATED_MS="$(jq -r '.updated_at_ms' "$RATE_START_FILE")"
STOPPED_UPDATED_MS="$(jq -r '.updated_at_ms' "$READY_FILE")"
READY_IQ_PAIRS="$(jq -r '.metrics.iq_pairs' "$RATE_START_FILE")"
STOPPED_IQ_PAIRS="$(jq -r '.metrics.iq_pairs' "$READY_FILE")"
STREAM_ELAPSED_MS="$((STOPPED_UPDATED_MS - READY_UPDATED_MS))"
STREAM_IQ_PAIRS="$((STOPPED_IQ_PAIRS - READY_IQ_PAIRS))"
(( STREAM_ELAPSED_MS > 0 && STREAM_IQ_PAIRS > 0 )) \
  || die "readiness timestamps or IQ counters did not advance"
STREAM_RATE="$((STREAM_IQ_PAIRS * 1000 / STREAM_ELAPSED_MS))"
(( STREAM_RATE >= MIN_STREAM_RATE && STREAM_RATE <= MAX_STREAM_RATE )) \
  || die "steady-state IQ rate ${STREAM_RATE}/s is outside the 384 kHz +/-2% acceptance band"

restore_services || die "could not restore the prior service activity"
persist_validation

log "Steady-state IQ rate: ${STREAM_RATE} pairs/s over ${STREAM_ELAPSED_MS}ms"
if (( CLIENT_PROBE )); then
  log "Client acceptance result:"
  jq . "$CLIENT_RESULT_FILE"
fi
log "Observed ready state:"
jq '{status,rf_safe,metrics:(.metrics | {frequency_hz,sample_rate_hz,dma_reads,dma_bytes,iq_pairs,fifo_hwm,fifo_threshold,fifo_overflow,fifo_underflow,header_resync,header_errors,tx_capable})}' \
  "$OBSERVED_FILE"
log "Final stopped state:"
jq '{status,rf_safe,error,metrics}' "$READY_FILE"
if (( RF_TX_PROBE )); then
  log "Operational direct-XDMA bounded production RF test passed; prior services restored"
else
  log "Operational direct-XDMA RF-inhibited RX/TX smoke test passed; prior services restored"
fi
