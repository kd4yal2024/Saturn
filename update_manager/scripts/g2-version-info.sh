#!/usr/bin/env bash
set -euo pipefail

SCRIPT_VERSION="2.0"
PERF_URL="${SATURN_LOCAL_P23_PERF_URL:-http://127.0.0.1:8080/p23_perf}"
CURRENT_TARGET="$(readlink -f /opt/saturn-go/p23-apps/current 2>/dev/null || true)"
REPO_ROOT="${SATURN_ACTIVE_REPO_ROOT:-${SATURN_REPO_ROOT:-/home/pi/github/Saturn}}"
TMP_PERF_JSON="$(mktemp)"
TMP_BANNER="$(mktemp)"
PERF_FETCH_STATUS=""

cleanup() {
  rm -f "${TMP_PERF_JSON}" "${TMP_BANNER}"
}

trap cleanup EXIT

say() {
  printf '%s\n' "$*"
}

detect_script_path() {
  local rel="$1"
  for base in "${REPO_ROOT}" "/usr/local/lib/saturn-go" "/opt/saturn-go" "/home/pi/github/Saturn"; do
    [[ -n "${base}" ]] || continue
    if [[ -x "${base}/${rel}" ]]; then
      printf '%s\n' "${base}/${rel}"
      return 0
    fi
  done
  return 1
}

kv_value() {
  local key="$1"
  local text="$2"
  awk -F= -v k="$key" '$1 == k {print substr($0, length(k) + 2); exit}' <<<"${text}"
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
import hashlib
import os
import sys
from pathlib import Path
from datetime import datetime

path = sys.argv[1]
current_target = os.environ.get("CURRENT_TARGET", "")
active_app_fallback = os.environ.get("ACTIVE_APP_FALLBACK", "unknown")
app_version_fallback = os.environ.get("APP_VERSION_FALLBACK", "")
binary_modified = os.environ.get("BINARY_MODIFIED", "")

def classify_saturn_hardware(fpga):
    if not fpga.get("available"):
        return {
            "system_role": "unknown",
            "fpga_xdma": "not reported",
            "radio_pcb": "unknown",
            "codec": "unknown",
            "puresignal": "unknown",
        }

    product = str(fpga.get("product") or "").strip().lower()
    try:
        product_version = int(fpga.get("product_version"))
    except Exception:
        product_version = None

    if product == "saturn" and product_version is not None:
        if product_version >= 3:
            return {
                "system_role": "local_saturn_g2",
                "fpga_xdma": "present",
                "radio_pcb": f"G2 PCB V{product_version}+",
                "codec": "TLV320AIC3204",
                "puresignal": "V3 level chain; default SetPk 0.8031",
            }
        return {
            "system_role": "local_saturn_g2",
            "fpga_xdma": "present",
            "radio_pcb": f"G2 PCB V{product_version}",
            "codec": "TLV320AIC23B",
            "puresignal": "legacy Saturn level chain; default SetPk 0.6121",
        }

    return {
        "system_role": "unknown_local_fpga",
        "fpga_xdma": "present",
        "radio_pcb": "unknown",
        "codec": "unknown",
        "puresignal": "unknown",
    }

try:
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
except Exception as exc:
    print(f"  Unable to parse /p23_perf output: {exc}")
    sys.exit(0)

perf = data.get("perf") or {}
telemetry = perf.get("app_telemetry") or {}
current = telemetry.get("current") or {}
if telemetry.get("pid_matches_service") is False:
    current = {}
    print("  Telemetry: rejected (snapshot PID does not match running service)")
workload = perf.get("workload") or {}
service = perf.get("service") or {}

deployment_slot = workload.get("selected_app") or active_app_fallback or "unknown"
app_identity = current.get("app") or deployment_slot or active_app_fallback or "unknown"
active_binary = workload.get("current_target_abs") or workload.get("current_target") or current_target or "unknown"
bridge_metrics = (current.get("gauges") or {}).get("bridge") or {}
component = {}
release_commit = None
# Match the manifest to the executable, not the source checkout or a staged
# release. Reading these files never starts an application or accesses XDMA.
try:
    binary = Path(active_binary).resolve(strict=True)
    release = binary.parent.parent
    manifest = json.loads((release / "release-manifest.json").read_text())
    for candidate in manifest.get("components", []):
        if (release / candidate["path"]).resolve() != binary:
            continue
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        running_digest = service.get("running_exe_sha256")
        if digest == candidate.get("sha256") and (not running_digest or running_digest == digest):
            component = candidate
            release_commit = (manifest.get("source") or {}).get("commit")
            break
except (OSError, ValueError, KeyError, TypeError):
    pass
app_version = current.get("version")
if app_version is None:
    app_version = bridge_metrics.get("app_version") or component.get("version")
if app_version is None:
    app_version = (app_version_fallback + " (checkout source; runtime unverified)") if app_version_fallback and deployment_slot == "p2" else "not published by this runtime"
app_build = current.get("build_git_sha") or bridge_metrics.get("build_git_sha") or component.get("source_commit") or release_commit
app_pid = current.get("pid") or service.get("main_pid") or "unknown"
uptime_sec = current.get("uptime_sec")
startup_mode = workload.get("startup_mode") or "unknown"
panel_mode = workload.get("panel_mode") or "unknown"
if panel_mode == "unknown" and deployment_slot == "xdma":
    panel_mode = "not applicable (Direct XDMA; see panel detection below)"
fpga = current.get("fpga") or {}

print(f"  Service: {service.get('name', 'p2app.service')}")
print(f"  Active binary family: {deployment_slot}")
print(f"  Runtime app identity: {app_identity}")
print(f"  Active binary path: {active_binary}")
print(f"  App version: {app_version}")
if app_build:
    print(f"  App build: {app_build}")
if release_commit:
    print(f"  Installed release: {release_commit} (executable hash verified)")
if active_binary != current_target:
    binary_modified = ""
try:
    binary_modified = datetime.fromtimestamp(Path(active_binary).stat().st_mtime).astimezone().strftime("%Y-%m-%d %H:%M:%S %Z")
except OSError:
    pass
if binary_modified:
    print(f"  Binary modified: {binary_modified}")
print(f"  PID: {app_pid}")
if uptime_sec is not None:
    print(f"  Uptime: {uptime_sec} sec")
print(f"  Startup mode: {startup_mode}")
print(f"  Panel mode: {panel_mode}")
if fpga.get("available"):
    if fpga.get("identity_source") == "acquisition-owner-session-cache":
        print("  FPGA identity source: acquisition owner; captured at session start")
    print(f"  FPGA product: {fpga.get('product', 'unknown')}; Version = {fpga.get('product_version', 'unknown')}")
    print(f"  FPGA image: {fpga.get('firmware_name', 'unknown')}")
    identity_status = fpga.get("build_identity_status")
    if identity_status in ("identified", "unidentified"):
        print(f"  FPGA firmware: {fpga.get('firmware_display')}")
        print(f"  RX filter: {fpga.get('rx_filter') or 'unidentified'}")
        print(f"  Build ID: {fpga.get('build_id_hex')}")
    elif identity_status == "stale":
        print("  FPGA firmware: live build identity unavailable (telemetry stale)")
        print("  RX filter: unidentified")
        print(f"  Build ID (stale telemetry): {fpga.get('build_id_hex')}")
    else:
        print(
            "  FPGA firmware: "
            f"{fpga.get('firmware_major_version', 'unknown')}."
            f"{fpga.get('firmware_version', 'unknown')} — build ID unavailable"
        )
        print("  RX filter: unidentified")
        print("  Build ID: unavailable")
    if fpga.get("date_code_hex") and not fpga.get("build_id_hex"):
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
else:
    print("  FPGA identity: not published by this runtime (hardware presence not determined by this report)")
classification = classify_saturn_hardware(fpga)
print("Hardware classification")
print(f"  System role: {classification['system_role']}")
print(f"  FPGA/XDMA: {classification['fpga_xdma']}")
print(f"  Radio PCB: {classification['radio_pcb']}")
print(f"  Codec family: {classification['codec']}")
print(f"  PureSignal class: {classification['puresignal']}")
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
  say "Hardware classification"
  say "  System role: unknown"
  say "  FPGA/XDMA: not reported"
  say "  Radio PCB: unknown"
  say "  Codec family: unknown"
  say "  PureSignal class: unknown"
fi

say
say "Platform / panel detection"
FRONT_PANEL_SCRIPT="$(detect_script_path "scripts/detect-front-panel.sh" || true)"
LCD_PROFILE_SCRIPT="$(detect_script_path "scripts/detect-lcd-profile.sh" || true)"
FRONT_PANEL_TYPE="unknown"
LCD_INFO=""

if [[ -n "${FRONT_PANEL_SCRIPT}" ]]; then
  FRONT_PANEL_TYPE="$("${FRONT_PANEL_SCRIPT}" --post-udev 2>/dev/null || printf 'unknown\n')"
  FRONT_PANEL_TYPE="${FRONT_PANEL_TYPE:-unknown}"
else
  FRONT_PANEL_TYPE="detector unavailable"
fi

if [[ -n "${LCD_PROFILE_SCRIPT}" ]]; then
  LCD_INFO="$("${LCD_PROFILE_SCRIPT}" 2>/dev/null || true)"
fi

say "  Front panel: ${FRONT_PANEL_TYPE}"
if [[ -n "${LCD_INFO}" ]]; then
  say "  Compute module: $(kv_value "module_family" "${LCD_INFO}")"
  STORAGE_VARIANT="$(kv_value "storage_variant" "${LCD_INFO}")"
  if [[ -z "${STORAGE_VARIANT}" || "${STORAGE_VARIANT}" == "unknown" ]]; then
    STORAGE_VARIANT="not identified (device-tree model does not distinguish Lite/eMMC)"
  fi
  say "  Storage variant: ${STORAGE_VARIANT}"
  say "  Platform vendor: $(kv_value "platform_vendor" "${LCD_INFO}")"
  say "  LCD size: $(kv_value "size" "${LCD_INFO}") ($(kv_value "size_source" "${LCD_INFO}"))"
  say "  LCD profile: $(kv_value "profile" "${LCD_INFO}")"
  say "  UART overlay: $(kv_value "recommended_uart_overlay" "${LCD_INFO}")"
  say "  Panel overlay: $(kv_value "recommended_panel_overlay" "${LCD_INFO}")"
else
  say "  LCD/profile detector: unavailable"
fi

# Boot media is observable in kernel metadata even when the CM4 SKU is not.
# Do not infer Lite/eMMC module identity from the card used to boot.
if command -v findmnt >/dev/null 2>&1 && command -v lsblk >/dev/null 2>&1; then
  ROOT_DEVICE="$(findmnt -nro SOURCE / 2>/dev/null || true)"
  if [[ "${ROOT_DEVICE}" == /dev/* ]]; then
    ROOT_PARENT="$(lsblk -dnro PKNAME -- "${ROOT_DEVICE}" 2>/dev/null || true)"
    ROOT_PARENT="${ROOT_PARENT:-$(basename "${ROOT_DEVICE}")}"
    if [[ "${ROOT_PARENT}" =~ ^[[:alnum:]_-]+$ && -r "/sys/class/block/${ROOT_PARENT}/device/type" ]]; then
      BOOT_MEDIA="$(cat "/sys/class/block/${ROOT_PARENT}/device/type")"
      say "  Boot medium: ${BOOT_MEDIA} (${ROOT_DEVICE}; kernel-reported)"
    fi
  fi
fi

say
say "P2 startup banner"
P2_SERVICE_STATE="$(systemctl is-active p2app.service 2>/dev/null || true)"
if [[ "${P2_SERVICE_STATE}" != "active" ]]; then
  say "  p2app.service is ${P2_SERVICE_STATE:-unknown}; historical P2 startup banner omitted."
  say "  Current FPGA identity and live-telemetry status are shown under Runtime above."
else
ACTIVE_SINCE="$(systemctl show -p ActiveEnterTimestamp --value p2app.service 2>/dev/null || true)"
CURRENT_STARTUP_LINES=""
LAST_KNOWN_STARTUP_LINES=""
if [[ -n "${ACTIVE_SINCE}" ]]; then
  CURRENT_STARTUP_LINES="$(
    journalctl -u p2app.service --since "${ACTIVE_SINCE}" --no-pager -o cat 2>/dev/null \
      | grep -E 'FPGA BIT file data code|FPGA Build ID| Product:| FPGA Firmware loaded:|All clocks present|Die Temp =' || true
  )"
fi
LAST_KNOWN_STARTUP_LINES="$(
  journalctl -u p2app.service --no-pager -o cat 2>/dev/null \
    | grep -E 'FPGA BIT file data code|FPGA Build ID| Product:| FPGA Firmware loaded:|All clocks present|Die Temp =' \
    | tail -n 6 || true
)"
printf '%s\n' "${LAST_KNOWN_STARTUP_LINES}" > "${TMP_BANNER}"

if [[ -n "${CURRENT_STARTUP_LINES}" ]]; then
  while IFS= read -r line; do
    [[ -n "${line}" ]] || continue
    say "  ${line}"
  done <<< "${CURRENT_STARTUP_LINES}"
elif [[ -n "${LAST_KNOWN_STARTUP_LINES}" ]]; then
  RETAINED_IDENTITY_STATE="$(python3 - "${TMP_PERF_JSON}" "${TMP_BANNER}" <<'PY'
import json
import re
import sys

try:
    with open(sys.argv[1], "r", encoding="utf-8") as handle:
        root = json.load(handle)
    live = (((root.get("perf") or {}).get("app_telemetry") or {}).get("current") or {}).get("fpga") or {}
except Exception:
    live = {}

try:
    with open(sys.argv[2], "r", encoding="utf-8") as handle:
        banner = handle.read()
except Exception:
    banner = ""

if not live.get("available"):
    print("unverified")
    raise SystemExit

patterns = {
    "firmware_version": r"FW Version\s*=\s*(\d+)",
    "product_version": r"Product:.*?Version\s*=\s*(\d+)",
    "date_code_hex": r"FPGA BIT file data code\s*=\s*([0-9A-Fa-f]+)",
}
mismatches = []
for field, pattern in patterns.items():
    match = re.search(pattern, banner)
    current = live.get(field)
    if not match or current is None:
        continue
    retained = match.group(1)
    if field == "date_code_hex":
        equal = retained.upper().zfill(8) == str(current).upper().zfill(8)
    else:
        try:
            equal = int(retained) == int(current)
        except (TypeError, ValueError):
            equal = False
    if not equal:
        mismatches.append(f"{field} retained={retained} live={current}")

print("stale:" + "; ".join(mismatches) if mismatches else "match")
PY
)"
  say "  Current p2app.service start at ${ACTIVE_SINCE:-unknown time} did not emit matching startup banner lines."
  if [[ "${RETAINED_IDENTITY_STATE}" == stale:* ]]; then
    say "  STALE retained startup banner: ${RETAINED_IDENTITY_STATE#stale:}"
    say "  Historical only; this banner is not current FPGA identity evidence:"
    BANNER_PREFIX="[stale]"
  else
    say "  Retained startup banner is historical and not current FPGA identity evidence (${RETAINED_IDENTITY_STATE})."
    BANNER_PREFIX="[retained]"
  fi
  while IFS= read -r line; do
    [[ -n "${line}" ]] || continue
    say "  ${BANNER_PREFIX} ${line}"
  done <<< "${LAST_KNOWN_STARTUP_LINES}"
else
  if [[ -n "${ACTIVE_SINCE}" ]]; then
    say "  Current p2app.service start at ${ACTIVE_SINCE} did not emit matching startup banner lines."
  fi
  say "  No retained FPGA startup banner lines were found in p2app.service journal."
fi
fi

say
say "Notes"
say "  - P2 uses p2app.service; Direct XDMA uses saturn-bridge.service."
say "  - A P2 startup banner is shown only while P2 is active."
say "  - A banner Die Temp is the value logged during that P2 service start."
