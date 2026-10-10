#!/bin/bash
# Rehearse order_live.sh entirely on loopback: the replay Bridge, the proxy, headless Chrome, the local collector, and a stand-in operator.
# Same environment as order.sh (SATURN_CMP_OUT, _BRIDGE_BIN, _PROXY_BIN, _WEBROOT, _STATE_DIR, _LOGIN_FILE, optional _WISDOM, _SECONDS) plus
# SATURN_CMP_EXPECT_BUILD (the commit the replay Bridge binary reports). Expects the replay's baseline: FPGA 53460003, RXC1 disabled.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${SATURN_CMP_OUT:?}"; : "${SATURN_CMP_PROXY_BIN:?}"; : "${SATURN_CMP_WEBROOT:?}"; : "${SATURN_CMP_STATE_DIR:?}"; : "${SATURN_CMP_LOGIN_FILE:?}"; : "${SATURN_CMP_BRIDGE_BIN:?}"
REPO_ROOT="$(cd "$HERE/../../../.." && pwd)"
mkdir -p "$SATURN_CMP_OUT"
BRIDGE_PORT=50141; TLS_PORT=18443
LOGIN="$(cat "$SATURN_CMP_LOGIN_FILE")"
SATURN_WEBROOT="$SATURN_CMP_WEBROOT" SATURN_STATE_DIR="$SATURN_CMP_STATE_DIR" SATURN_REPO_ROOT="$REPO_ROOT" \
  SATURN_ADDR=127.0.0.1:18080 SATURN_REMOTE_TLS_ADDR="127.0.0.1:$TLS_PORT" SATURN_REMOTE_BASIC_AUTH="$LOGIN" \
  SATURN_REMOTE_BRIDGE_WS="ws://127.0.0.1:$BRIDGE_PORT" "$SATURN_CMP_PROXY_BIN" > "$SATURN_CMP_OUT.proxy.out" 2> "$SATURN_CMP_OUT.proxy.err" &
PROXY=$!
cleanup() {
  kill -TERM "$PROXY" 2>/dev/null
  [ -f "$SATURN_CMP_OUT/bridge.pid" ] && kill -TERM "$(cat "$SATURN_CMP_OUT/bridge.pid")" 2>/dev/null
  sleep 3
}
trap cleanup EXIT
export SATURN_CMP_COLLECTOR=local SATURN_CMP_LIVE_ORIGIN="https://127.0.0.1:$TLS_PORT" SATURN_CMP_EXPECT_FPGA=53460003 SATURN_CMP_EXPECT_RXC1=disabled \
  SATURN_CMP_PROFILE=rehearsal SATURN_CMP_SECONDS="${SATURN_CMP_SECONDS:-30}" SATURN_CMP_SETTLE_S="${SATURN_CMP_SETTLE_S:-5}" SATURN_CMP_WAIT_S=300 \
  SATURN_CMP_OPERATOR_HOOK="$HERE/rehearsal_operator.sh" SATURN_CMP_BRIDGE_PORT=$BRIDGE_PORT
"$HERE/order_live.sh"
