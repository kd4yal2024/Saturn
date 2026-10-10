#!/bin/bash
# The planned order against a LIVE Bridge: C, A-off, B-off, B-on, A-on, A-on, B-on, B-off, A-off, C.
#
# This driver starts nothing on the radio host and restarts nothing. At each point where the planned order needs a different Bridge
# instance it prints exactly what the operator is to do, then WAITS until the owner's own cached telemetry (read through the bounded
# collector) shows the wanted instance: a different process, the requested TCP_NODELAY setting, the planned RXC1 status, build and
# FPGA image, and all of it unchanged for a settle period. Only then does a window start. Each window's identity and RXC1 record are built
# from that observation, never typed. At the end the fail-closed checker runs. Exit status as in order_lib.sh: 0 / 1 / 3.
#
# Required environment:
#   SATURN_CMP_OUT            empty or new output directory
#   SATURN_CMP_LIVE_ORIGIN    the origin that serves the remote page, for example https://<host>:8443
#   SATURN_CMP_LOGIN_FILE     file containing user:password for that page's basic auth
#   SATURN_CMP_COLLECTOR      ssh (live: also SATURN_CMP_COLLECTOR_HOST, and _PATH / _SERVICE) or local (rehearsal; see collector.mjs)
#   SATURN_CMP_EXPECT_BUILD   the 40-hex commit the Bridge must report as its build
#   SATURN_CMP_EXPECT_FPGA    the FPGA build id the owner must report (date_code_hex, for example 53460004)
# Optional: SATURN_CMP_EXPECT_RXC1 (default valid), SATURN_CMP_SECONDS (window length, default 600), SATURN_CMP_PROFILE (default live),
#   SATURN_CMP_SETTLE_S (default 30), SATURN_CMP_WAIT_S (per restart, default 1800), SATURN_CMP_CHROME (default google-chrome),
#   SATURN_CMP_OPERATOR_HOOK (rehearsal only: a command called as "HOOK <instance> <nodelay>" that performs the restart; it must also
#   write the new Bridge's work directory to $SATURN_CMP_OUT/current_workdir when the collector is local).
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=order_lib.sh
source "$HERE/order_lib.sh"
: "${SATURN_CMP_OUT:?set SATURN_CMP_OUT to an empty or new directory}"
: "${SATURN_CMP_LIVE_ORIGIN:?set SATURN_CMP_LIVE_ORIGIN}"
: "${SATURN_CMP_LOGIN_FILE:?set SATURN_CMP_LOGIN_FILE}"
: "${SATURN_CMP_COLLECTOR:?set SATURN_CMP_COLLECTOR (ssh or local)}"
: "${SATURN_CMP_EXPECT_BUILD:?set SATURN_CMP_EXPECT_BUILD}"
: "${SATURN_CMP_EXPECT_FPGA:?set SATURN_CMP_EXPECT_FPGA}"
RXC1="${SATURN_CMP_EXPECT_RXC1:-valid}"
SECS="${SATURN_CMP_SECONDS:-600}"
PROFILE="${SATURN_CMP_PROFILE:-live}"
SETTLE="${SATURN_CMP_SETTLE_S:-30}"
WAIT="${SATURN_CMP_WAIT_S:-1800}"
CHROME="${SATURN_CMP_CHROME:-google-chrome}"
HOOK="${SATURN_CMP_OPERATOR_HOOK:-}"
CDP_PORT="${SATURN_CMP_CDP_PORT:-9335}"
[[ "$SATURN_CMP_EXPECT_BUILD" =~ ^[0-9a-f]{40}$ ]] || { echo "SATURN_CMP_EXPECT_BUILD must be a 40-hex commit" >&2; exit 2; }
if [ -n "$(ls -A "$SATURN_CMP_OUT" 2>/dev/null)" ]; then echo "SATURN_CMP_OUT must be empty: $SATURN_CMP_OUT" >&2; exit 2; fi
mkdir -p "$SATURN_CMP_OUT/windows"
ORDER_DIR="$SATURN_CMP_OUT"
LOGIN="$(cat "$SATURN_CMP_LOGIN_FILE")"
CHROME_DIR="$SATURN_CMP_OUT/chrome-profile"

cleanup() { [ -f "$SATURN_CMP_OUT/chrome.pid" ] && kill -TERM "$(cat "$SATURN_CMP_OUT/chrome.pid")" 2>/dev/null; sleep 2; }
trap cleanup EXIT
# Chrome must not inherit a long TMPDIR: its singleton socket path has a length limit. The page is the host's own, so its certificate is accepted.
env -u TMPDIR "$CHROME" --headless=new --remote-debugging-port="$CDP_PORT" --user-data-dir="$CHROME_DIR" --no-sandbox --disable-gpu \
  --ignore-certificate-errors --autoplay-policy=no-user-gesture-required --window-size=1400,900 about:blank \
  > "$SATURN_CMP_OUT/chrome.out" 2>&1 &
echo $! > "$SATURN_CMP_OUT/chrome.pid"
sleep 5

json() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2], {'d': d}))" "$1" "$2"; }

CUR=""; N=0; IDX=0; ABORT=0; WORKDIR="-"; EXE=""
for ENTRY in C:1 A-off:0 B-off:0 B-on:1 A-on:1 A-on:1 B-on:1 B-off:0 A-off:0 C:1; do
  NAME=${ENTRY%%:*}; ND=${ENTRY##*:}; IDX=$((IDX + 1))
  [ "$ABORT" -ne 0 ] && continue
  if [ "$CUR" != "$ND" ]; then
    N=$((N + 1)); CUR=$ND
    echo
    echo "=================== RESTART REQUIRED: Bridge instance $N, TCP_NODELAY=$ND (before window $IDX, $NAME) ==================="
    echo "Do runbook section 1 (a)-(d) now: review the previous trial, set SATURN_BRIDGE_TCI_NODELAY=$ND, create the fresh RXC1 arm, restart through the controlled owner path."
    echo "Waiting up to ${WAIT}s for the owner to report: a different process, TCP_NODELAY=$ND, RXC1 status '$RXC1', FPGA $SATURN_CMP_EXPECT_FPGA, build ${SATURN_CMP_EXPECT_BUILD:0:12}, unchanged for ${SETTLE}s."
    [ -n "$HOOK" ] && "$HOOK" "$N" "$ND"
    [ -f "$SATURN_CMP_OUT/current_workdir" ] && WORKDIR="$(cat "$SATURN_CMP_OUT/current_workdir")"
    ARGS=(--sidecar "$SATURN_CMP_OUT/instance_$N.collector.jsonl" --out "$SATURN_CMP_OUT/instance_$N.json" --nodelay "$ND" --rxc1 "$RXC1"
          --build "$SATURN_CMP_EXPECT_BUILD" --fpga-build "$SATURN_CMP_EXPECT_FPGA" --settle-s "$SETTLE" --timeout-s "$WAIT")
    [ "$WORKDIR" != "-" ] && ARGS+=(--work-dir "$WORKDIR")
    [ "$N" -gt 1 ] && ARGS+=(--previous "$SATURN_CMP_OUT/instance_$((N - 1)).json" --exe-sha "$EXE")
    if ! node "$HERE/await_instance.mjs" "${ARGS[@]}"; then
      echo "INSTANCE $N NEVER REACHED THE WANTED STATE; the remaining windows are not run (details in instance_$N.json)"
      FAILED_WINDOWS+=("${IDX}:${NAME}:instance")
      ABORT=1; continue
    fi
    [ "$N" -eq 1 ] && EXE="$(json "$SATURN_CMP_OUT/instance_1.json" "d['instance']['exeSha256']")"
  fi
  case $NAME in A-*) DISP=iq; MODE=normal;; B-*) DISP=spectrum; MODE=normal;; C) DISP=iq; MODE=c;; esac
  PID="$(json "$SATURN_CMP_OUT/instance_$N.json" "d['instance']['pid']")"
  META="$(python3 - "$SATURN_CMP_OUT/instance_$N.json" "$IDX" "$NAME" "$ND" "$N" <<'PYEOF'
import json, sys
inst = json.load(open(sys.argv[1]))["instance"]
print(json.dumps({"index": int(sys.argv[2]), "arm": sys.argv[3], "bridgeNoDelay": int(sys.argv[4]), "bridgePid": inst["pid"], "bridgeSha256": inst["exeSha256"],
                  "bridgeRestartNumber": int(sys.argv[5]),
                  "rxc1": {"state": inst["rxc1Status"], "pid": inst["pid"], "identity": f"FPGA {inst['fpgaBuildId']} fw {inst['firmwareMajor']}.{inst['firmwareMinor']}, build {inst['buildGitSha'][:12]}"}}))
PYEOF
)"
  run_window "$IDX" "$NAME" node "$HERE/window.mjs" "$CDP_PORT" "$LOGIN" \
    "$SATURN_CMP_LIVE_ORIGIN/remote-next?transport=split&tx_opus=1&tx_cfc=1&display_transport=$DISP" \
    "$WORKDIR" "$SECS" "$SATURN_CMP_OUT/windows/w${IDX}_$NAME" "$MODE" "$META"
done

finish_order python3 "$HERE/check_order.py" "$SATURN_CMP_OUT/windows" --profile "$PROFILE" --report-dir "$SATURN_CMP_OUT/check_report" \
  --require-owner --expect-rxc1 "$RXC1" --expect-fpga-build "$SATURN_CMP_EXPECT_FPGA"
STATUS=$?
echo "LIVE ORDER FINISHED with status $STATUS (0 valid and verified, 1 failed or invalid, 3 valid but something is UNVERIFIED)"
exit "$STATUS"
