#!/usr/bin/env bash
set -euo pipefail

SCRIPT_VERSION="1.4"
PERF_URL="${SATURN_LOCAL_P23_PERF_URL:-http://127.0.0.1:8080/p23_perf}"
CURRENT_TARGET="$(readlink -f /opt/saturn-go/p23-apps/current 2>/dev/null || true)"
REPO_ROOT="${SATURN_ACTIVE_REPO_ROOT:-${SATURN_REPO_ROOT:-/home/pi/github/Saturn}}"
TMP_PERF_JSON="$(mktemp)"
PERF_FETCH_STATUS=""

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

if command -v curl >/dev/null 2>&1; then
  if ! curl -fsS "${PERF_URL}" -o "${TMP_PERF_JSON}" 2>/dev/null; then
    PERF_FETCH_STATUS="fetch failed"
  elif [[ ! -s "${TMP_PERF_JSON}" ]]; then
    PERF_FETCH_STATUS="empty response"
  fi
else
  PERF_FETCH_STATUS="curl not installed"
fi

if [[ -s "${TMP_PERF_JSON}" ]]; then
  CURRENT_TARGET="${CURRENT_TARGET}" \
  ACTIVE_APP_FALLBACK="${ACTIVE_APP}" \
  APP_VERSION_FALLBACK="${APP_VERSION_FALLBACK}" \
  BINARY_MODIFIED="${BINARY_MODIFIED}" \
  python3 - "${TMP_PERF_JSON}" <<'PY'
import json
import os
import sys

path = sys.argv[1]
current_target = os.environ.get("CURRENT_TARGET", "")
active_app_fallback = os.environ.get("ACTIVE_APP_FALLBACK", "unknown")
app_version_fallback = os.environ.get("APP_VERSION_FALLBACK", "")
binary_modified = os.environ.get("BINARY_MODIFIED", "")

try:
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
except Exception as exc:
    print(f"  Unable to parse /p23_perf output: {exc}")
    sys.exit(0)

perf = data.get("perf") or {}
telemetry = perf.get("app_telemetry") or {}
current = telemetry.get("current") or {}
workload = perf.get("workload") or {}
service = perf.get("service") or {}

deployment_slot = workload.get("selected_app") or active_app_fallback or "unknown"
app_identity = current.get("app") or deployment_slot or active_app_fallback or "unknown"
active_binary = workload.get("current_target_abs") or workload.get("current_target") or current_target or "unknown"
app_version = current.get("version")
if app_version is None:
    app_version = app_version_fallback or "unknown"
app_pid = current.get("pid") or service.get("main_pid") or "unknown"
uptime_sec = current.get("uptime_sec")
startup_mode = workload.get("startup_mode") or "unknown"
panel_mode = workload.get("panel_mode") or "unknown"
fpga = current.get("fpga") or {}

print(f"  Service: {service.get('name', 'p2app.service')}")
print(f"  Active binary family: {deployment_slot}")
print(f"  Runtime app identity: {app_identity}")
print(f"  Active binary path: {active_binary}")
print(f"  App version: {app_version}")
if binary_modified:
    print(f"  Binary modified: {binary_modified}")
print(f"  PID: {app_pid}")
if uptime_sec is not None:
    print(f"  Uptime: {uptime_sec} sec")
print(f"  Startup mode: {startup_mode}")
print(f"  Panel mode: {panel_mode}")
if fpga.get("available"):
    print(f"  FPGA product: {fpga.get('product', 'unknown')}; Version = {fpga.get('product_version', 'unknown')}")
    print(
        "  FPGA firmware: "
        f"{fpga.get('firmware_name', 'unknown')}; "
        f"FW Version = {fpga.get('firmware_version', 'unknown')}, "
        f"major version = {fpga.get('firmware_major_version', 'unknown')}"
    )
    if fpga.get("date_code_hex"):
        print(f"  FPGA BIT file date code: {fpga.get('date_code_hex')}")
    if fpga.get("all_clocks_present") is True:
        print("  FPGA clocks: all present")
    elif fpga.get("clock_mask") is not None:
        print(f"  FPGA clock mask: 0x{int(fpga.get('clock_mask', 0)):X}")
    print(f"  FPGA fallback config: {'yes' if fpga.get('fallback_config') else 'no'}")
    if fpga.get("die_temp_valid"):
        try:
            print(f"  Die Temp (runtime): {float(fpga.get('die_temp_c')):.1f}C")
        except Exception:
            pass
PY
else
  if [[ -n "${PERF_FETCH_STATUS}" ]]; then
    say "  WARN: Live telemetry unavailable (${PERF_FETCH_STATUS})"
  else
    say "  WARN: Live telemetry unavailable (/p23_perf returned no data)"
  fi
  say "  Service: p2app.service"
  say "  Active binary family: ${ACTIVE_APP}"
  say "  Runtime app identity: ${ACTIVE_APP}"
  say "  Active binary path: ${CURRENT_TARGET:-unknown}"
  if [[ -n "${APP_VERSION_FALLBACK}" ]]; then
    say "  App version: ${APP_VERSION_FALLBACK}"
  fi
  if [[ -n "${BINARY_MODIFIED}" ]]; then
    say "  Binary modified: ${BINARY_MODIFIED}"
  fi
fi

say
say "Latest startup banner"
ACTIVE_SINCE="$(systemctl show -p ActiveEnterTimestamp --value p2app.service 2>/dev/null || true)"
CURRENT_STARTUP_LINES=""
LAST_KNOWN_STARTUP_LINES=""
if [[ -n "${ACTIVE_SINCE}" ]]; then
  CURRENT_STARTUP_LINES="$(
    journalctl -u p2app.service --since "${ACTIVE_SINCE}" --no-pager -o cat 2>/dev/null \
      | grep -E 'FPGA BIT file data code| Product:| FPGA Firmware loaded:|All clocks present|Die Temp =' || true
  )"
fi
LAST_KNOWN_STARTUP_LINES="$(
  journalctl -u p2app.service --no-pager -o cat 2>/dev/null \
    | grep -E 'FPGA BIT file data code| Product:| FPGA Firmware loaded:|All clocks present|Die Temp =' \
    | tail -n 5 || true
)"

if [[ -n "${CURRENT_STARTUP_LINES}" ]]; then
  while IFS= read -r line; do
    [[ -n "${line}" ]] || continue
    say "  ${line}"
  done <<< "${CURRENT_STARTUP_LINES}"
elif [[ -n "${LAST_KNOWN_STARTUP_LINES}" ]]; then
  say "  Current p2app.service start at ${ACTIVE_SINCE:-unknown time} did not emit matching startup banner lines."
  say "  Showing most recent captured banner from an earlier retained start:"
  while IFS= read -r line; do
    [[ -n "${line}" ]] || continue
    say "  ${line}"
  done <<< "${LAST_KNOWN_STARTUP_LINES}"
else
  if [[ -n "${ACTIVE_SINCE}" ]]; then
    say "  Current p2app.service start at ${ACTIVE_SINCE} did not emit matching startup banner lines."
  fi
  say "  No retained FPGA startup banner lines were found in p2app.service journal."
fi

say
say "Notes"
say "  - In Trixie the app runs as p2app.service in the background."
say "  - The startup banner above replaces the old terminal window scrollback."
say "  - Die Temp is the value logged during the most recent service start."
