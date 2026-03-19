#!/usr/bin/env bash
set -euo pipefail

SCRIPT_VERSION="1.0"
PERF_URL="${SATURN_LOCAL_P23_PERF_URL:-http://127.0.0.1:8080/p23_perf}"
CURRENT_TARGET="$(readlink -f /opt/saturn-go/p23-apps/current 2>/dev/null || true)"
REPO_ROOT="${SATURN_ACTIVE_REPO_ROOT:-${SATURN_REPO_ROOT:-/home/pi/github/Saturn}}"
TMP_PERF_JSON="$(mktemp)"

cleanup() {
  rm -f "${TMP_PERF_JSON}"
}

trap cleanup EXIT

say() {
  printf '%s\n' "$*"
}

say "Saturn App / Firmware Info"
say "Script version: ${SCRIPT_VERSION}"
say "Generated: $(date '+%Y-%m-%d %H:%M:%S %Z')"
say

ACTIVE_APP="unknown"
APP_SOURCE=""
if [[ -n "${CURRENT_TARGET}" ]]; then
  case "$(basename "${CURRENT_TARGET}")" in
    p2app)
      ACTIVE_APP="p2"
      APP_SOURCE="${REPO_ROOT}/sw_projects/P2_app/p2app.c"
      ;;
    p3app)
      ACTIVE_APP="p3"
      APP_SOURCE="${REPO_ROOT}/sw_projects/P3_app/p2app.c"
      ;;
  esac
fi

APP_VERSION_FALLBACK=""
if [[ -n "${APP_SOURCE}" && -f "${APP_SOURCE}" ]]; then
  APP_VERSION_FALLBACK="$(
    grep -E '^#define P[23]APPVERSION[[:space:]]+[0-9]+' "${APP_SOURCE}" 2>/dev/null \
      | tail -n 1 \
      | awk '{print $3}' || true
  )"
fi

BINARY_MODIFIED=""
if [[ -n "${CURRENT_TARGET}" && -f "${CURRENT_TARGET}" ]]; then
  BINARY_MODIFIED="$(date -d "@$(stat -c '%Y' "${CURRENT_TARGET}")" '+%Y-%m-%d %H:%M:%S %Z' 2>/dev/null || true)"
fi

say "Runtime"
say "  Service: p2app.service"
say "  Active app: ${ACTIVE_APP}"
say "  Active binary: ${CURRENT_TARGET:-unknown}"
if [[ -n "${APP_VERSION_FALLBACK}" ]]; then
  say "  App version (source): ${APP_VERSION_FALLBACK}"
fi
if [[ -n "${BINARY_MODIFIED}" ]]; then
  say "  Binary modified: ${BINARY_MODIFIED}"
fi

if command -v curl >/dev/null 2>&1; then
  curl -fsS "${PERF_URL}" -o "${TMP_PERF_JSON}" 2>/dev/null || true
fi

if [[ -s "${TMP_PERF_JSON}" ]]; then
  say "  Extra live telemetry:"
  CURRENT_TARGET="${CURRENT_TARGET}" python3 - "${TMP_PERF_JSON}" <<'PY'
import json
import os
import sys

path = sys.argv[1]
current_target = os.environ.get("CURRENT_TARGET", "")

try:
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
except Exception as exc:
    print("Runtime")
    print(f"  Unable to parse /p23_perf output: {exc}")
    print("")
    sys.exit(0)

perf = data.get("perf") or {}
telemetry = perf.get("app_telemetry") or {}
current = telemetry.get("current") or {}
workload = perf.get("workload") or {}
service = perf.get("service") or {}

active_app = workload.get("selected_app") or current.get("app") or "unknown"
active_binary = workload.get("current_target_abs") or workload.get("current_target") or current_target or "unknown"
app_version = current.get("version")
app_pid = current.get("pid") or service.get("main_pid") or "unknown"
uptime_sec = current.get("uptime_sec")
startup_mode = workload.get("startup_mode") or "unknown"
panel_mode = workload.get("panel_mode") or "unknown"

print("Runtime")
print(f"  Service: {service.get('name', 'p2app.service')}")
print(f"  Active app: {active_app}")
print(f"  Active binary: {active_binary}")
print(f"  App version: {app_version if app_version is not None else 'unknown'}")
print(f"  PID: {app_pid}")
if uptime_sec is not None:
    print(f"  Uptime: {uptime_sec} sec")
print(f"  Startup mode: {startup_mode}")
print(f"  Panel mode: {panel_mode}")
print("")
PY
else
  say "  Extra live telemetry: unavailable (/p23_perf)"
  say
fi

say "Latest startup banner"
STARTUP_LINES="$(
  journalctl -u p2app.service --no-pager -o cat 2>/dev/null \
    | grep -E 'FPGA BIT file data code| Product:| FPGA Firmware loaded:|All clocks present|Die Temp =' \
    | tail -n 5 || true
)"

if [[ -n "${STARTUP_LINES}" ]]; then
  while IFS= read -r line; do
    [[ -n "${line}" ]] || continue
    say "  ${line}"
  done <<< "${STARTUP_LINES}"
else
  say "  No matching startup lines found in p2app.service journal."
fi

say
say "Notes"
say "  - In Trixie the app runs as p2app.service in the background."
say "  - The startup banner above replaces the old terminal window scrollback."
say "  - Die Temp is the value logged during the most recent service start."
