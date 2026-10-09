#!/usr/bin/env bash
# Run the unmodified Saturn Bridge against a paced, repeatable receive stream
# with no XDMA hardware. Receive only: the DUC device is /dev/null and RF TX is
# inhibited. See README.md.
#
#   run_replay_bridge.sh --bridge PATH --work DIR [--port N] [--tones SPEC]
#                        [--noise-dbfs N] [--source FILE] [--pacing free]
#                        [--wisdom FILE]
#
# --work must be a new or empty directory; everything the Bridge writes goes
# there. The Bridge is exec'd, so its output and exit status are this script's.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bridge=""
work=""
port=50001
tones="1500:-30"
noise="-100"
source_file="synthetic"
pacing="realtime"
wisdom=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bridge) bridge="$2"; shift 2 ;;
    --work) work="$2"; shift 2 ;;
    --port) port="$2"; shift 2 ;;
    --tones) tones="$2"; shift 2 ;;
    --noise-dbfs) noise="$2"; shift 2 ;;
    --source) source_file="$2"; shift 2 ;;
    --pacing) pacing="$2"; shift 2 ;;
    --wisdom) wisdom="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -x "$bridge" ]] || { echo "--bridge must be an executable saturn-bridge" >&2; exit 2; }
[[ -n "$work" ]] || { echo "--work is required" >&2; exit 2; }

# Never run next to a real radio: the replay would conflict with the live
# Bridge and could be mistaken for it.
if [[ -e /dev/xdma0_user && "${SATURN_REPLAY_ALLOW_RADIO_HOST:-0}" != "1" ]]; then
  echo "refusing to run: /dev/xdma0_user exists, so this looks like a radio host" >&2
  exit 3
fi
case "$(realpath -m "$work")" in
  /dev/*|/proc/*|/sys/*) echo "refusing to use $work as a work directory" >&2; exit 3 ;;
esac
mkdir -p "$work"
if [[ -n "$(ls -A "$work")" ]]; then
  echo "--work must be empty: $work" >&2
  exit 2
fi

python3 "$here/make_register_file.py" "$work/xdma_user.bin"
: > "$work/xdma_c2h.placeholder"
cc -O2 -shared -fPIC -o "$work/xdma_replay_shim.so" "$here/xdma_replay_shim.c" -ldl -lm -lpthread

export LD_PRELOAD="$work/xdma_replay_shim.so"
export SATURN_REPLAY_RX_DEVICE="$work/xdma_c2h.placeholder"
export SATURN_REPLAY_SOURCE="$source_file"
export SATURN_REPLAY_TONES="$tones"
export SATURN_REPLAY_NOISE_DBFS="$noise"
export SATURN_REPLAY_PACING="$pacing"
export SATURN_REPLAY_STATS_PATH="$work/replay-stats.json"

export SATURN_BRIDGE_RADIO_BACKEND=xdma
export SATURN_BRIDGE_XDMA_USER_DEVICE="$work/xdma_user.bin"
export SATURN_BRIDGE_XDMA_RX_DEVICE="$work/xdma_c2h.placeholder"
export SATURN_BRIDGE_XDMA_DUC_DEVICE=/dev/null
export SATURN_BRIDGE_TCI_HOST=127.0.0.1
export SATURN_BRIDGE_TCI_PORT="$port"
export SATURN_BRIDGE_PERF_PATH="$work/perf.json"
export SATURN_BRIDGE_XDMA_READY_PATH="$work/ready.json"
export SATURN_BRIDGE_XDMA_TELEMETRY_PATH="$work/xdma-telemetry.json"
export SATURN_BRIDGE_SATP_STATUS_PATH="$work/satp-status.json"
# Without wisdom, WDSP plans its FFTs at start-up, which takes about a minute.
# --wisdom FILE reuses a file made by the Bridge itself, creating it on first
# use. Wisdom is specific to this machine's CPU and FFTW build.
if [[ -n "$wisdom" ]]; then
  if [[ ! -f "$wisdom" ]]; then
    mkdir -p "$(dirname "$wisdom")"
    "$bridge" --generate-fftw-wisdom "$wisdom"
  fi
  "$bridge" --validate-fftw-wisdom "$wisdom"
  export SATURN_BRIDGE_FFTW_WISDOM_PATH="$wisdom"
else
  export SATURN_BRIDGE_FFTW_WISDOM_PATH="$work/wisdom"
fi
export SATURN_REMOTE_TX_RF_ENABLED=0

exec "$bridge"
