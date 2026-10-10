#!/bin/bash
# Stand-in for the human operator in a local rehearsal of order_live.sh: restarts the REPLAY Bridge with the requested TCP_NODELAY.
#   rehearsal_operator.sh INSTANCE NODELAY
# Needs SATURN_CMP_OUT, SATURN_CMP_BRIDGE_BIN and optionally SATURN_CMP_WISDOM, SATURN_CMP_BRIDGE_PORT. It only ever starts and stops the
# replay Bridge it started itself (pid in $SATURN_CMP_OUT/bridge.pid). It never touches a radio or a host.
set -u
N="$1"; ND="$2"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${SATURN_CMP_OUT:?}"; PORT="${SATURN_CMP_BRIDGE_PORT:-50141}"
if [ -f "$OUT/bridge.pid" ] && kill -0 "$(cat "$OUT/bridge.pid")" 2>/dev/null; then
  kill -TERM "$(cat "$OUT/bridge.pid")"
  for _ in $(seq 1 20); do kill -0 "$(cat "$OUT/bridge.pid")" 2>/dev/null || break; sleep 0.5; done
fi
WORK="$OUT/work_$N"
WISDOM=(); [ -n "${SATURN_CMP_WISDOM:-}" ] && WISDOM=(--wisdom "$SATURN_CMP_WISDOM")
mkdir -p "$OUT/bridge-logs"
SATURN_BRIDGE_TCI_NODELAY="$ND" "$HERE/../replay/run_replay_bridge.sh" --bridge "${SATURN_CMP_BRIDGE_BIN:?}" --work "$WORK" --port "$PORT" --tones 1500:-30 "${WISDOM[@]}" \
  > "$OUT/bridge-logs/bridge_$N.out" 2> "$OUT/bridge-logs/bridge_$N.err" &
echo $! > "$OUT/bridge.pid"
echo "$WORK" > "$OUT/current_workdir"
for _ in $(seq 1 90); do sleep 1; grep -q '"ddc_s": [89][0-9][0-9]' "$WORK/perf.json" 2>/dev/null && break; done
echo "[operator stand-in] replay Bridge #$N pid $(cat "$OUT/bridge.pid") TCP_NODELAY=$ND started"
