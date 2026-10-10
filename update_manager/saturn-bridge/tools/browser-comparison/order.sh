#!/bin/bash
# The planned order, rehearsed locally: C, A-off, B-off, B-on, A-on, A-on, B-on, B-off, A-off, C.
# Starts a replay Bridge, the proxy and a headless Chrome on loopback, runs the ten windows, restarts the
# Bridge between windows only when the TCP_NODELAY setting changes, then runs the checker. Everything it starts
# is stopped on exit. The exit status is the combined one from order_lib.sh (0 / 1 / 3).
#
# Required environment (nothing here is a default for a particular machine):
#   SATURN_CMP_OUT         empty or new output directory
#   SATURN_CMP_BRIDGE_BIN  saturn-bridge built with the real WDSP (SATURN_BRIDGE_WDSP_COMMIT set)
#   SATURN_CMP_PROXY_BIN   the saturn-go proxy binary
#   SATURN_CMP_WEBROOT     page directory: the repo's templates plus the built saturn-remote-next.js
#   SATURN_CMP_STATE_DIR   scratch state directory for the proxy
#   SATURN_CMP_LOGIN_FILE  file containing user:password for the proxy's basic auth (throwaway)
# Optional: SATURN_CMP_WISDOM (FFTW wisdom file), SATURN_CMP_SECONDS (default 30), SATURN_CMP_PROFILE (default
# rehearsal), SATURN_CMP_CHROME (default google-chrome), SATURN_CMP_CHECKER_ARGS (extra arguments for check_order.py,
# for example "--require-owner"), SATURN_CMP_COLLECTOR=local (read the Bridge's telemetry through the owner reader; see
# window.mjs and collector.mjs).
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=order_lib.sh
source "$HERE/order_lib.sh"
: "${SATURN_CMP_OUT:?set SATURN_CMP_OUT to an empty or new directory}"
: "${SATURN_CMP_BRIDGE_BIN:?set SATURN_CMP_BRIDGE_BIN}"
: "${SATURN_CMP_PROXY_BIN:?set SATURN_CMP_PROXY_BIN}"
: "${SATURN_CMP_WEBROOT:?set SATURN_CMP_WEBROOT}"
: "${SATURN_CMP_STATE_DIR:?set SATURN_CMP_STATE_DIR}"
: "${SATURN_CMP_LOGIN_FILE:?set SATURN_CMP_LOGIN_FILE}"
SECS="${SATURN_CMP_SECONDS:-30}"
PROFILE="${SATURN_CMP_PROFILE:-rehearsal}"
CHROME="${SATURN_CMP_CHROME:-google-chrome}"
CHECK_ARGS=(); [ -n "${SATURN_CMP_CHECKER_ARGS:-}" ] && read -r -a CHECK_ARGS <<< "$SATURN_CMP_CHECKER_ARGS"
REPO_ROOT="$(cd "$HERE/../../../.." && pwd)"
RUNNER="$HERE/../replay/run_replay_bridge.sh"
if [ -n "$(ls -A "$SATURN_CMP_OUT" 2>/dev/null)" ]; then echo "SATURN_CMP_OUT must be empty: $SATURN_CMP_OUT" >&2; exit 2; fi
mkdir -p "$SATURN_CMP_OUT/windows" "$SATURN_CMP_OUT/bridge-logs"
ORDER_DIR="$SATURN_CMP_OUT"
LOGIN="$(cat "$SATURN_CMP_LOGIN_FILE")"
BSHA="$(sha256sum "$SATURN_CMP_BRIDGE_BIN" | cut -d' ' -f1)"
BRIDGE_PORT=50141; TLS_PORT=18443; CDP_PORT=9335
CHROME_DIR="$SATURN_CMP_OUT/chrome-profile"

cleanup() {
  for f in "$SATURN_CMP_OUT/chrome.pid" "$SATURN_CMP_OUT/proxy.pid" "$SATURN_CMP_OUT/bridge.pid"; do
    [ -f "$f" ] && kill -TERM "$(cat "$f")" 2>/dev/null
  done
  sleep 3
}
trap cleanup EXIT

CUR=""; N=0
start_bridge() {  # $1 = TCP_NODELAY setting (0|1); restarts the Bridge, outside any window
  if [ -f "$SATURN_CMP_OUT/bridge.pid" ] && kill -0 "$(cat "$SATURN_CMP_OUT/bridge.pid")" 2>/dev/null; then
    kill -TERM "$(cat "$SATURN_CMP_OUT/bridge.pid")"
    for _ in $(seq 1 20); do kill -0 "$(cat "$SATURN_CMP_OUT/bridge.pid")" 2>/dev/null || break; sleep 0.5; done
  fi
  N=$((N + 1))
  local work="$SATURN_CMP_OUT/work_$N"
  local wisdom=(); [ -n "${SATURN_CMP_WISDOM:-}" ] && wisdom=(--wisdom "$SATURN_CMP_WISDOM")
  SATURN_BRIDGE_TCI_NODELAY="$1" "$RUNNER" --bridge "$SATURN_CMP_BRIDGE_BIN" --work "$work" --port "$BRIDGE_PORT" --tones 1500:-30 "${wisdom[@]}" \
    > "$SATURN_CMP_OUT/bridge-logs/bridge_$N.out" 2> "$SATURN_CMP_OUT/bridge-logs/bridge_$N.err" &
  echo $! > "$SATURN_CMP_OUT/bridge.pid"
  for _ in $(seq 1 90); do sleep 1; grep -q '"ddc_s": [89][0-9][0-9]' "$work/perf.json" 2>/dev/null && break; done
  CUR=$1
  echo "[restart] bridge #$N pid $(cat "$SATURN_CMP_OUT/bridge.pid") TCP_NODELAY=$1 (outside any window)"
}

SATURN_WEBROOT="$SATURN_CMP_WEBROOT" SATURN_STATE_DIR="$SATURN_CMP_STATE_DIR" SATURN_REPO_ROOT="$REPO_ROOT" \
  SATURN_ADDR=127.0.0.1:18080 SATURN_REMOTE_TLS_ADDR="127.0.0.1:$TLS_PORT" SATURN_REMOTE_BASIC_AUTH="$LOGIN" \
  SATURN_REMOTE_BRIDGE_WS="ws://127.0.0.1:$BRIDGE_PORT" "$SATURN_CMP_PROXY_BIN" > "$SATURN_CMP_OUT/proxy.out" 2> "$SATURN_CMP_OUT/proxy.err" &
echo $! > "$SATURN_CMP_OUT/proxy.pid"
# Chrome must not inherit a long TMPDIR: its singleton socket path has a length limit.
env -u TMPDIR "$CHROME" --headless=new --remote-debugging-port="$CDP_PORT" --user-data-dir="$CHROME_DIR" --no-sandbox --disable-gpu \
  --ignore-certificate-errors --autoplay-policy=no-user-gesture-required --window-size=1400,900 about:blank \
  > "$SATURN_CMP_OUT/chrome.out" 2>&1 &
echo $! > "$SATURN_CMP_OUT/chrome.pid"
sleep 5
echo "bridge binary sha256 $BSHA"

IDX=0
for ENTRY in C:1 A-off:0 B-off:0 B-on:1 A-on:1 A-on:1 B-on:1 B-off:0 A-off:0 C:1; do
  NAME=${ENTRY%%:*}; ND=${ENTRY##*:}; IDX=$((IDX + 1))
  [ "$CUR" != "$ND" ] && start_bridge "$ND"
  case $NAME in A-*) DISP=iq; MODE=normal;; B-*) DISP=spectrum; MODE=normal;; C) DISP=iq; MODE=c;; esac
  META="{\"index\":$IDX,\"arm\":\"$NAME\",\"bridgeNoDelay\":$ND,\"bridgePid\":$(cat "$SATURN_CMP_OUT/bridge.pid"),\"bridgeSha256\":\"$BSHA\",\"bridgeRestartNumber\":$N,\"rxc1\":\"not applicable (replay)\"}"
  run_window "$IDX" "$NAME" node "$HERE/window.mjs" "$CDP_PORT" "$LOGIN" \
    "https://127.0.0.1:$TLS_PORT/remote-next?transport=split&tx_opus=1&tx_cfc=1&display_transport=$DISP" \
    "$SATURN_CMP_OUT/work_$N" "$SECS" "$SATURN_CMP_OUT/windows/w${IDX}_$NAME" "$MODE" "$META"
done

finish_order python3 "$HERE/check_order.py" "$SATURN_CMP_OUT/windows" --bridge-logs "$SATURN_CMP_OUT/bridge-logs" \
  --profile "$PROFILE" --report-dir "$SATURN_CMP_OUT/check_report" ${CHECK_ARGS[@]+"${CHECK_ARGS[@]}"}
STATUS=$?
echo "ORDER FINISHED with status $STATUS (0 valid and verified, 1 failed or invalid, 3 valid but something is UNVERIFIED: TCP_NODELAY, freshness or the owner identity)"
exit "$STATUS"
