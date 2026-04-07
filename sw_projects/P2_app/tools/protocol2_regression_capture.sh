#!/usr/bin/env bash
set -euo pipefail

FILTER="udp and (port 1024 or port 1025 or port 1026 or port 1027 or port 1028 or port 1029 or portrange 1035-1044)"

usage() {
  cat <<'EOF'
Usage:
  protocol2_regression_capture.sh --iface <iface> --out-dir <dir> [options]

Options:
  --iface <iface>            Capture interface (required)
  --out-dir <dir>            Output directory for pcap + run record (required)
  --label <label>            Label prefix for output files (default: protocol2-regression)
  --client <name>            Client under test (default: unknown)
  --client-version <ver>     Client version/build (default: unknown)
  --app <name>               App identity under test (default: p2app)
  --app-commit <sha>         App git commit (default: unknown)
  --profile <text>           Startup args/profile (default: unknown)
  --panel-mode <mode>        SATURN_FRONT_PANEL_MODE value (default: auto)
  --fpga-version <ver>       FPGA version/bitstream (default: unknown)
  --capture-side <side>      Capture side/host (default: Saturn)
  --state <state>            Test state label (default: idle)
  --tester <name>            Tester name (default: unknown)
  --notes <text>             Free-form notes appended to the run record
  -h, --help                 Show this help text

The script writes:
  <label>-<timestamp>.pcap
  <label>-<timestamp>.txt

Then it execs:
  sudo tcpdump -i <iface> -nn -s 0 -w <pcap> '<filter>'

Stop the capture with Ctrl-C when the test state is complete.
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'ERROR: required command not found: %s\n' "$1" >&2
    exit 1
  }
}

slugify() {
  printf '%s' "$1" | tr '[:space:]/' '--' | tr -cd '[:alnum:]_.-'
}

IFACE=""
OUT_DIR=""
LABEL="protocol2-regression"
CLIENT="unknown"
CLIENT_VERSION="unknown"
APP_NAME="p2app"
APP_COMMIT="unknown"
PROFILE="unknown"
PANEL_MODE="auto"
FPGA_VERSION="unknown"
CAPTURE_SIDE="Saturn"
TEST_STATE="idle"
TESTER="unknown"
NOTES=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --iface) IFACE="${2:-}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:-}"; shift 2 ;;
    --label) LABEL="${2:-}"; shift 2 ;;
    --client) CLIENT="${2:-}"; shift 2 ;;
    --client-version) CLIENT_VERSION="${2:-}"; shift 2 ;;
    --app) APP_NAME="${2:-}"; shift 2 ;;
    --app-commit) APP_COMMIT="${2:-}"; shift 2 ;;
    --profile) PROFILE="${2:-}"; shift 2 ;;
    --panel-mode) PANEL_MODE="${2:-}"; shift 2 ;;
    --fpga-version) FPGA_VERSION="${2:-}"; shift 2 ;;
    --capture-side) CAPTURE_SIDE="${2:-}"; shift 2 ;;
    --state) TEST_STATE="${2:-}"; shift 2 ;;
    --tester) TESTER="${2:-}"; shift 2 ;;
    --notes) NOTES="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'ERROR: unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$IFACE" ]] || { printf 'ERROR: --iface is required\n' >&2; exit 2; }
[[ -n "$OUT_DIR" ]] || { printf 'ERROR: --out-dir is required\n' >&2; exit 2; }

need_cmd sudo
need_cmd tcpdump

mkdir -p "$OUT_DIR"
STAMP="$(date '+%Y%m%d-%H%M%S')"
BASE_NAME="$(slugify "$LABEL")-$STAMP"
PCAP_PATH="$OUT_DIR/$BASE_NAME.pcap"
RUN_RECORD_PATH="$OUT_DIR/$BASE_NAME.txt"

cat >"$RUN_RECORD_PATH" <<EOF
Date/Time: $(date '+%Y-%m-%d %H:%M:%S %Z')
Tester: $TESTER
App commit: $APP_COMMIT
Client: $CLIENT
Client version: $CLIENT_VERSION
FPGA version: $FPGA_VERSION
Active app/profile: $APP_NAME / $PROFILE
SATURN_FRONT_PANEL_MODE: $PANEL_MODE
Capture side/interface: $CAPTURE_SIDE / $IFACE
Test state: $TEST_STATE
Baseline pcap:
After-change pcap: $PCAP_PATH
Result: PENDING
Notes: $NOTES
EOF

printf '[INFO] Run record: %s\n' "$RUN_RECORD_PATH"
printf '[INFO] Capture file: %s\n' "$PCAP_PATH"
printf '[INFO] Filter: %s\n' "$FILTER"
printf '[INFO] Starting tcpdump on %s. Stop with Ctrl-C when the test is complete.\n' "$IFACE"

exec sudo tcpdump -i "$IFACE" -nn -s 0 -w "$PCAP_PATH" "$FILTER"
