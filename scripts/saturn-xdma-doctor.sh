#!/usr/bin/env bash
set -euo pipefail

SATURN_XDMA_VENDOR="${SATURN_XDMA_VENDOR:-0x10ee}"
SATURN_XDMA_DEVICE="${SATURN_XDMA_DEVICE:-0x7024}"
SATURN_XDMA_MODULE="${SATURN_XDMA_MODULE:-xdma}"
SATURN_XDMA_DEVICE_NODE="${SATURN_XDMA_DEVICE_NODE:-/dev/xdma0_user}"
SATURN_XDMA_UDEV_NODE="${SATURN_XDMA_UDEV_NODE:-/dev/xdma/card0/user}"
SATURN_XDMA_SERVICE="${SATURN_XDMA_SERVICE:-p2app.service}"
SATURN_XDMA_POSTINST_HOOK="${SATURN_XDMA_POSTINST_HOOK:-/etc/kernel/postinst.d/saturn-xdma}"
SATURN_XDMA_POSTINST_HELPER="${SATURN_XDMA_POSTINST_HELPER:-/usr/local/bin/saturn-xdma-kernel-postinst.sh}"

TRY_MODPROBE=0
JSON_OUTPUT=0
MARK_MODPROBE_FAILED=0
SKIP_SERVICE_CHECK=0
STAGE_ONLY=0
ADVISORY=""
ADVISORY_SUMMARY=""
SATURN_XDMA_TRANSIENT_MARKER="${SATURN_XDMA_TRANSIENT_MARKER:-0}"

usage() {
  cat <<'EOF'
Usage:
  saturn-xdma-doctor.sh [--json] [--stage-only] [--skip-service-check] [--try-modprobe] [--mark-modprobe-failed]

Diagnose the Saturn XDMA readiness chain:

  FPGA endpoint on PCIe -> xdma module -> /dev/xdma* -> p2app.service

Options:
  --json                 Emit structured JSON output
  --stage-only           Emit only the classified stage name
  --skip-service-check   Do not classify p2app.service state; useful before app startup
  --try-modprobe         If running as root and xdma is not loaded, try `modprobe xdma`
  --mark-modprobe-failed Force stage `XDMA_MODPROBE_FAILED` when xdma is still not loaded
                         (intended for callers such as fix-xdma.sh after a failed modprobe)
  -h, --help             Show this help text
EOF
}

have() {
  command -v "$1" >/dev/null 2>&1
}

join_by() {
  local delim="$1"
  shift || true
  local first=1
  local item
  for item in "$@"; do
    if [[ "$first" -eq 1 ]]; then
      printf '%s' "$item"
      first=0
    else
      printf '%s%s' "$delim" "$item"
    fi
  done
}

running_under_saturngo_service() {
  grep -q 'saturn-go\.service' /proc/self/cgroup 2>/dev/null
}

maybe_reexec_via_systemd_run() {
  [[ "$SATURN_XDMA_TRANSIENT_MARKER" == "1" ]] && return 0
  running_under_saturngo_service || return 0
  command -v systemd-run >/dev/null 2>&1 || return 0
  systemd-run --help 2>&1 | grep -q -- '--pipe' || return 0

  local unit="saturn-xdma-doctor-$(date +%Y%m%d%H%M%S)-$$"
  exec systemd-run --quiet --wait --collect --pipe --service-type=exec \
    --unit "$unit" \
    --setenv=SATURN_XDMA_TRANSIENT_MARKER=1 \
    "$0" "$@"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --json) JSON_OUTPUT=1 ;;
    --stage-only) STAGE_ONLY=1 ;;
    --skip-service-check) SKIP_SERVICE_CHECK=1 ;;
    --try-modprobe) TRY_MODPROBE=1 ;;
    --mark-modprobe-failed) MARK_MODPROBE_FAILED=1 ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'ERROR: Unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

maybe_reexec_via_systemd_run "$@"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

RUNNING_KERNEL="$(uname -r)"
MODPROBE_OUTPUT_FILE="${TMP_DIR}/modprobe-output.txt"
KERNEL_LOG_FILE="${TMP_DIR}/kernel-log.txt"
LSPCI_NN_FILE="${TMP_DIR}/lspci-nn.txt"
LSPCI_K_FILE="${TMP_DIR}/lspci-k.txt"
LSMOD_FILE="${TMP_DIR}/lsmod.txt"
P2APP_STATUS_FILE="${TMP_DIR}/p2app-status.txt"

MODPROBE_ATTEMPTED=0
MODPROBE_EXIT_CODE=0

collect_matching_endpoints() {
  ENDPOINT_PATHS=()
  ENDPOINT_BDFS=()
  ENDPOINT_DRIVERS=()

  local dev_path vendor device driver_name
  for dev_path in /sys/bus/pci/devices/*; do
    [[ -e "$dev_path" ]] || continue
    [[ -r "$dev_path/vendor" && -r "$dev_path/device" ]] || continue

    vendor="$(tr -d '\n' < "$dev_path/vendor" 2>/dev/null || true)"
    device="$(tr -d '\n' < "$dev_path/device" 2>/dev/null || true)"
    if [[ "$vendor" != "$SATURN_XDMA_VENDOR" || "$device" != "$SATURN_XDMA_DEVICE" ]]; then
      continue
    fi

    ENDPOINT_PATHS+=("$dev_path")
    ENDPOINT_BDFS+=("$(basename "$dev_path")")
    if [[ -L "$dev_path/driver" ]]; then
      driver_name="$(basename "$(readlink -f "$dev_path/driver")")"
    else
      driver_name="none"
    fi
    ENDPOINT_DRIVERS+=("$driver_name")
  done
}

find_module_path_on_disk() {
  local modules_root="/lib/modules/${RUNNING_KERNEL}"
  local candidate=""

  [[ -d "$modules_root" ]] || return 0

  while IFS= read -r candidate; do
    printf '%s' "$candidate"
    return 0
  done < <(
    find "$modules_root" -type f \
      \( -name "${SATURN_XDMA_MODULE}.ko" -o -name "${SATURN_XDMA_MODULE}.ko.xz" \) \
      -print 2>/dev/null | sort | head -n1
  )
}

detect_module_state() {
  MODULE_PATH="$(modinfo -k "$RUNNING_KERNEL" -n "$SATURN_XDMA_MODULE" 2>/dev/null || true)"
  if [[ -z "$MODULE_PATH" || ! -e "$MODULE_PATH" ]]; then
    MODULE_PATH="$(find_module_path_on_disk)"
  fi
  if [[ -n "$MODULE_PATH" && -e "$MODULE_PATH" ]]; then
    MODULE_INSTALLED_FOR_KERNEL=1
  else
    MODULE_INSTALLED_FOR_KERNEL=0
  fi

  if [[ -d "/sys/module/${SATURN_XDMA_MODULE}" ]]; then
    MODULE_LOADED=1
  else
    MODULE_LOADED=0
  fi
}

detect_binding_state() {
  BOUND_TO_XDMA=0
  local driver_name
  for driver_name in "${ENDPOINT_DRIVERS[@]:-}"; do
    if [[ "$driver_name" == "$SATURN_XDMA_MODULE" ]]; then
      BOUND_TO_XDMA=1
      break
    fi
  done
}

try_load_module_if_requested() {
  : >"$MODPROBE_OUTPUT_FILE"

  if [[ "$TRY_MODPROBE" -ne 1 ]]; then
    return 0
  fi
  if [[ "$(id -u)" -ne 0 ]]; then
    printf 'Not root; skipping modprobe attempt.\n' >"$MODPROBE_OUTPUT_FILE"
    return 0
  fi
  if [[ "$MODULE_LOADED" -eq 1 || "$MODULE_INSTALLED_FOR_KERNEL" -ne 1 || "${#ENDPOINT_PATHS[@]}" -eq 0 ]]; then
    return 0
  fi
  if ! have modprobe; then
    printf 'modprobe not found.\n' >"$MODPROBE_OUTPUT_FILE"
    return 0
  fi

  MODPROBE_ATTEMPTED=1
  if modprobe "$SATURN_XDMA_MODULE" >"$MODPROBE_OUTPUT_FILE" 2>&1; then
    MODPROBE_EXIT_CODE=0
    if have udevadm; then
      udevadm settle >/dev/null 2>&1 || true
    fi
  else
    MODPROBE_EXIT_CODE=$?
  fi
}

collect_supporting_output() {
  if have lspci; then
    lspci -nn -d "${SATURN_XDMA_VENDOR#0x}:${SATURN_XDMA_DEVICE#0x}" >"$LSPCI_NN_FILE" 2>&1 || true
    lspci -k -d "${SATURN_XDMA_VENDOR#0x}:${SATURN_XDMA_DEVICE#0x}" >"$LSPCI_K_FILE" 2>&1 || true
  else
    printf 'lspci not installed.\n' >"$LSPCI_NN_FILE"
    printf 'lspci not installed.\n' >"$LSPCI_K_FILE"
  fi

  if have lsmod; then
    lsmod | grep -E "^${SATURN_XDMA_MODULE}[[:space:]]" >"$LSMOD_FILE" 2>&1 || true
  else
    printf 'lsmod not installed.\n' >"$LSMOD_FILE"
  fi

  if have journalctl; then
    journalctl -k -b --no-pager 2>&1 | grep -iE 'pcie|brcm-pcie|xilinx|xdma|10ee:7024' >"$KERNEL_LOG_FILE" || true
  elif have dmesg; then
    dmesg 2>&1 | grep -iE 'pcie|brcm-pcie|xilinx|xdma|10ee:7024' >"$KERNEL_LOG_FILE" || true
  else
    printf 'Neither journalctl nor dmesg is available.\n' >"$KERNEL_LOG_FILE"
  fi

  if have systemctl; then
    systemctl status "$SATURN_XDMA_SERVICE" --no-pager >"$P2APP_STATUS_FILE" 2>&1 || true
  else
    printf 'systemctl not installed.\n' >"$P2APP_STATUS_FILE"
  fi
}

detect_service_state() {
  SERVICE_EXISTS=0
  SERVICE_RUNNING=0
  SERVICE_ACTIVE_STATE="unknown"
  SERVICE_SUB_STATE="unknown"

  if ! have systemctl; then
    return 0
  fi

  local load_state
  load_state="$(systemctl show -p LoadState --value "$SATURN_XDMA_SERVICE" 2>/dev/null || true)"
  if [[ "$load_state" == "loaded" ]]; then
    SERVICE_EXISTS=1
  fi

  if [[ "$SERVICE_EXISTS" -eq 1 ]]; then
    SERVICE_ACTIVE_STATE="$(systemctl show -p ActiveState --value "$SATURN_XDMA_SERVICE" 2>/dev/null || true)"
    SERVICE_SUB_STATE="$(systemctl show -p SubState --value "$SATURN_XDMA_SERVICE" 2>/dev/null || true)"
    if [[ "$SERVICE_ACTIVE_STATE" == "active" && "$SERVICE_SUB_STATE" == "running" ]]; then
      SERVICE_RUNNING=1
    fi
  fi
}

detect_postinst_state() {
  POSTINST_HOOK_EXISTS=0
  POSTINST_HOOK_EXECUTABLE=0
  POSTINST_HELPER_EXISTS=0
  POSTINST_HELPER_EXECUTABLE=0
  POSTINST_HOOK_REFERENCES_HELPER=0

  [[ -e "$SATURN_XDMA_POSTINST_HOOK" ]] && POSTINST_HOOK_EXISTS=1
  [[ -x "$SATURN_XDMA_POSTINST_HOOK" ]] && POSTINST_HOOK_EXECUTABLE=1
  [[ -e "$SATURN_XDMA_POSTINST_HELPER" ]] && POSTINST_HELPER_EXISTS=1
  [[ -x "$SATURN_XDMA_POSTINST_HELPER" ]] && POSTINST_HELPER_EXECUTABLE=1
  if [[ -r "$SATURN_XDMA_POSTINST_HOOK" ]] && grep -Fq "$SATURN_XDMA_POSTINST_HELPER" "$SATURN_XDMA_POSTINST_HOOK"; then
    POSTINST_HOOK_REFERENCES_HELPER=1
  fi
}

determine_stage() {
  if [[ "${#ENDPOINT_PATHS[@]}" -eq 0 ]]; then
    STAGE="PCIE_ENDPOINT_MISSING"
    STAGE_SUMMARY="No Saturn FPGA PCIe endpoint (10ee:7024) is visible. This is upstream of XDMA."
  elif [[ "$MARK_MODPROBE_FAILED" -eq 1 && "$MODULE_LOADED" -ne 1 ]]; then
    STAGE="XDMA_MODPROBE_FAILED"
    STAGE_SUMMARY="A prior modprobe attempt failed and xdma is still not loaded."
  elif [[ "$MODPROBE_ATTEMPTED" -eq 1 && "$MODPROBE_EXIT_CODE" -ne 0 ]]; then
    STAGE="XDMA_MODPROBE_FAILED"
    STAGE_SUMMARY="modprobe xdma failed."
  elif [[ "$MODULE_INSTALLED_FOR_KERNEL" -ne 1 && "$MODULE_LOADED" -ne 1 ]]; then
    STAGE="XDMA_MODULE_NOT_INSTALLED_FOR_KERNEL"
    STAGE_SUMMARY="The xdma module is not installed for the running kernel."
  elif [[ "$MODULE_LOADED" -ne 1 ]]; then
    STAGE="XDMA_MODULE_NOT_LOADED"
    STAGE_SUMMARY="The xdma module is installed for this kernel but is not currently loaded."
  elif [[ "$BOUND_TO_XDMA" -ne 1 ]]; then
    STAGE="XDMA_NOT_BOUND"
    STAGE_SUMMARY="The Saturn PCIe endpoint is present, but it is not bound to the xdma driver."
  elif [[ ! -e "$SATURN_XDMA_DEVICE_NODE" ]]; then
    STAGE="XDMA_DEVNODE_MISSING"
    STAGE_SUMMARY="xdma is bound, but ${SATURN_XDMA_DEVICE_NODE} is missing."
  elif [[ ! -e "$SATURN_XDMA_UDEV_NODE" ]]; then
    STAGE="XDMA_UDEV_SYMLINK_MISSING"
    STAGE_SUMMARY="The kernel-created XDMA node exists, but the friendly udev symlink is missing."
  elif [[ "$SKIP_SERVICE_CHECK" -ne 1 && "$SERVICE_EXISTS" -eq 1 && "$SERVICE_RUNNING" -ne 1 ]]; then
    STAGE="P2APP_START_FAILED"
    STAGE_SUMMARY="${SATURN_XDMA_SERVICE} is not active/running even though the XDMA path is ready."
  else
    STAGE="OK"
    STAGE_SUMMARY="PCIe endpoint, XDMA driver, device nodes, and service state all look ready."
  fi
}

determine_advisory() {
  ADVISORY=""
  ADVISORY_SUMMARY=""

  if [[ "$MODULE_LOADED" -eq 1 && "$MODULE_INSTALLED_FOR_KERNEL" -ne 1 ]]; then
    ADVISORY="XDMA_MODULE_NOT_STAGED_FOR_RUNNING_KERNEL"
    ADVISORY_SUMMARY="XDMA is working now because the module is already loaded, but it is not installed on disk for the running kernel. Reboot, modprobe reload, or recovery after a service stop may fail until the module is staged for $(uname -r)."
  fi
}

print_human_output() {
  printf 'Saturn XDMA Doctor\n'
  printf 'stage=%s\n' "$STAGE"
  printf 'summary=%s\n' "$STAGE_SUMMARY"
  if [[ -n "$ADVISORY" ]]; then
    printf 'advisory=%s\n' "$ADVISORY"
    printf 'advisory_summary=%s\n' "$ADVISORY_SUMMARY"
  fi
  printf 'running_kernel=%s\n' "$RUNNING_KERNEL"
  printf 'endpoint_present=%s\n' "$( [[ "${#ENDPOINT_PATHS[@]}" -gt 0 ]] && printf true || printf false )"
  printf 'endpoint_bdfs=%s\n' "$(join_by "," "${ENDPOINT_BDFS[@]:-}")"
  printf 'endpoint_drivers=%s\n' "$(join_by "," "${ENDPOINT_DRIVERS[@]:-}")"
  printf 'module_installed_for_kernel=%s\n' "$( [[ "$MODULE_INSTALLED_FOR_KERNEL" -eq 1 ]] && printf true || printf false )"
  printf 'module_path=%s\n' "${MODULE_PATH:-}"
  printf 'module_loaded=%s\n' "$( [[ "$MODULE_LOADED" -eq 1 ]] && printf true || printf false )"
  printf 'bound_to_xdma=%s\n' "$( [[ "$BOUND_TO_XDMA" -eq 1 ]] && printf true || printf false )"
  printf 'devnode_exists=%s\n' "$( [[ -e "$SATURN_XDMA_DEVICE_NODE" ]] && printf true || printf false )"
  printf 'udev_symlink_exists=%s\n' "$( [[ -e "$SATURN_XDMA_UDEV_NODE" ]] && printf true || printf false )"
  printf 'service_exists=%s\n' "$( [[ "$SERVICE_EXISTS" -eq 1 ]] && printf true || printf false )"
  printf 'service_active_state=%s\n' "$SERVICE_ACTIVE_STATE"
  printf 'service_sub_state=%s\n' "$SERVICE_SUB_STATE"
  printf 'postinst_hook_path=%s\n' "$SATURN_XDMA_POSTINST_HOOK"
  printf 'postinst_hook_exists=%s\n' "$( [[ "$POSTINST_HOOK_EXISTS" -eq 1 ]] && printf true || printf false )"
  printf 'postinst_hook_executable=%s\n' "$( [[ "$POSTINST_HOOK_EXECUTABLE" -eq 1 ]] && printf true || printf false )"
  printf 'postinst_helper_path=%s\n' "$SATURN_XDMA_POSTINST_HELPER"
  printf 'postinst_helper_exists=%s\n' "$( [[ "$POSTINST_HELPER_EXISTS" -eq 1 ]] && printf true || printf false )"
  printf 'postinst_helper_executable=%s\n' "$( [[ "$POSTINST_HELPER_EXECUTABLE" -eq 1 ]] && printf true || printf false )"
  printf 'postinst_hook_references_helper=%s\n' "$( [[ "$POSTINST_HOOK_REFERENCES_HELPER" -eq 1 ]] && printf true || printf false )"
  printf 'modprobe_attempted=%s\n' "$( [[ "$MODPROBE_ATTEMPTED" -eq 1 ]] && printf true || printf false )"
  printf 'modprobe_exit_code=%s\n' "$MODPROBE_EXIT_CODE"

  if [[ -s "$MODPROBE_OUTPUT_FILE" ]]; then
    printf 'modprobe_output<<EOF\n'
    cat "$MODPROBE_OUTPUT_FILE"
    printf 'EOF\n'
  fi

  printf 'lspci_nn<<EOF\n'
  cat "$LSPCI_NN_FILE"
  printf 'EOF\n'
  printf 'lspci_k<<EOF\n'
  cat "$LSPCI_K_FILE"
  printf 'EOF\n'
  printf 'lsmod_xdma<<EOF\n'
  cat "$LSMOD_FILE"
  printf 'EOF\n'
  printf 'kernel_log<<EOF\n'
  cat "$KERNEL_LOG_FILE"
  printf 'EOF\n'
  printf 'p2app_status<<EOF\n'
  cat "$P2APP_STATUS_FILE"
  printf 'EOF\n'
}

print_json_output() {
  local endpoint_present_text module_installed_text module_loaded_text bound_to_xdma_text
  local devnode_exists_text udev_symlink_exists_text service_exists_text service_running_text modprobe_attempted_text
  local postinst_hook_exists_text postinst_hook_executable_text postinst_helper_exists_text
  local postinst_helper_executable_text postinst_hook_references_helper_text

  endpoint_present_text=false
  module_installed_text=false
  module_loaded_text=false
  bound_to_xdma_text=false
  devnode_exists_text=false
  udev_symlink_exists_text=false
  service_exists_text=false
  service_running_text=false
  modprobe_attempted_text=false
  postinst_hook_exists_text=false
  postinst_hook_executable_text=false
  postinst_helper_exists_text=false
  postinst_helper_executable_text=false
  postinst_hook_references_helper_text=false

  [[ "${#ENDPOINT_PATHS[@]}" -gt 0 ]] && endpoint_present_text=true
  [[ "$MODULE_INSTALLED_FOR_KERNEL" -eq 1 ]] && module_installed_text=true
  [[ "$MODULE_LOADED" -eq 1 ]] && module_loaded_text=true
  [[ "$BOUND_TO_XDMA" -eq 1 ]] && bound_to_xdma_text=true
  [[ -e "$SATURN_XDMA_DEVICE_NODE" ]] && devnode_exists_text=true
  [[ -e "$SATURN_XDMA_UDEV_NODE" ]] && udev_symlink_exists_text=true
  [[ "$SERVICE_EXISTS" -eq 1 ]] && service_exists_text=true
  [[ "$SERVICE_RUNNING" -eq 1 ]] && service_running_text=true
  [[ "$MODPROBE_ATTEMPTED" -eq 1 ]] && modprobe_attempted_text=true
  [[ "$POSTINST_HOOK_EXISTS" -eq 1 ]] && postinst_hook_exists_text=true
  [[ "$POSTINST_HOOK_EXECUTABLE" -eq 1 ]] && postinst_hook_executable_text=true
  [[ "$POSTINST_HELPER_EXISTS" -eq 1 ]] && postinst_helper_exists_text=true
  [[ "$POSTINST_HELPER_EXECUTABLE" -eq 1 ]] && postinst_helper_executable_text=true
  [[ "$POSTINST_HOOK_REFERENCES_HELPER" -eq 1 ]] && postinst_hook_references_helper_text=true

  ENDPOINT_BDFS_TEXT="$(printf '%s\n' "${ENDPOINT_BDFS[@]:-}")"
  ENDPOINT_DRIVERS_TEXT="$(printf '%s\n' "${ENDPOINT_DRIVERS[@]:-}")"

  export STAGE STAGE_SUMMARY ADVISORY ADVISORY_SUMMARY RUNNING_KERNEL MODULE_PATH SERVICE_ACTIVE_STATE SERVICE_SUB_STATE
  export SATURN_XDMA_DEVICE_NODE SATURN_XDMA_UDEV_NODE SATURN_XDMA_SERVICE
  export SATURN_XDMA_POSTINST_HOOK SATURN_XDMA_POSTINST_HELPER
  export endpoint_present_text module_installed_text module_loaded_text bound_to_xdma_text
  export devnode_exists_text udev_symlink_exists_text service_exists_text service_running_text modprobe_attempted_text
  export postinst_hook_exists_text postinst_hook_executable_text postinst_helper_exists_text
  export postinst_helper_executable_text postinst_hook_references_helper_text
  export MODPROBE_EXIT_CODE ENDPOINT_BDFS_TEXT ENDPOINT_DRIVERS_TEXT
  export MODPROBE_OUTPUT_FILE LSPCI_NN_FILE LSPCI_K_FILE LSMOD_FILE KERNEL_LOG_FILE P2APP_STATUS_FILE

  python3 - <<'PY'
import json
import os

def read_file(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""

def read_lines_from_env(name):
    raw = os.environ.get(name, "")
    if not raw:
        return []
    return [line for line in raw.splitlines() if line]

data = {
    "stage": os.environ.get("STAGE", ""),
    "summary": os.environ.get("STAGE_SUMMARY", ""),
    "advisory": os.environ.get("ADVISORY", ""),
    "advisory_summary": os.environ.get("ADVISORY_SUMMARY", ""),
    "running_kernel": os.environ.get("RUNNING_KERNEL", ""),
    "endpoint_present": os.environ.get("endpoint_present_text") == "true",
    "endpoint_bdfs": read_lines_from_env("ENDPOINT_BDFS_TEXT"),
    "endpoint_drivers": read_lines_from_env("ENDPOINT_DRIVERS_TEXT"),
    "module_installed_for_kernel": os.environ.get("module_installed_text") == "true",
    "module_path": os.environ.get("MODULE_PATH", ""),
    "module_loaded": os.environ.get("module_loaded_text") == "true",
    "bound_to_xdma": os.environ.get("bound_to_xdma_text") == "true",
    "devnode_path": os.environ.get("SATURN_XDMA_DEVICE_NODE", ""),
    "devnode_exists": os.environ.get("devnode_exists_text") == "true",
    "udev_symlink_path": os.environ.get("SATURN_XDMA_UDEV_NODE", ""),
    "udev_symlink_exists": os.environ.get("udev_symlink_exists_text") == "true",
    "service_name": os.environ.get("SATURN_XDMA_SERVICE", ""),
    "service_exists": os.environ.get("service_exists_text") == "true",
    "service_running": os.environ.get("service_running_text") == "true",
    "service_active_state": os.environ.get("SERVICE_ACTIVE_STATE", ""),
    "service_sub_state": os.environ.get("SERVICE_SUB_STATE", ""),
    "postinst_hook_path": os.environ.get("SATURN_XDMA_POSTINST_HOOK", ""),
    "postinst_hook_exists": os.environ.get("postinst_hook_exists_text") == "true",
    "postinst_hook_executable": os.environ.get("postinst_hook_executable_text") == "true",
    "postinst_helper_path": os.environ.get("SATURN_XDMA_POSTINST_HELPER", ""),
    "postinst_helper_exists": os.environ.get("postinst_helper_exists_text") == "true",
    "postinst_helper_executable": os.environ.get("postinst_helper_executable_text") == "true",
    "postinst_hook_references_helper": os.environ.get("postinst_hook_references_helper_text") == "true",
    "modprobe_attempted": os.environ.get("modprobe_attempted_text") == "true",
    "modprobe_exit_code": int(os.environ.get("MODPROBE_EXIT_CODE", "0") or "0"),
    "modprobe_output": read_file(os.environ.get("MODPROBE_OUTPUT_FILE", "")),
    "lspci_nn": read_file(os.environ.get("LSPCI_NN_FILE", "")),
    "lspci_k": read_file(os.environ.get("LSPCI_K_FILE", "")),
    "lsmod_xdma": read_file(os.environ.get("LSMOD_FILE", "")),
    "kernel_log": read_file(os.environ.get("KERNEL_LOG_FILE", "")),
    "p2app_status": read_file(os.environ.get("P2APP_STATUS_FILE", "")),
}

print(json.dumps(data, indent=2))
PY
}

collect_matching_endpoints
detect_module_state
try_load_module_if_requested
collect_matching_endpoints
detect_module_state
detect_binding_state
detect_service_state
detect_postinst_state
collect_supporting_output
determine_stage
determine_advisory

if [[ "$STAGE_ONLY" -eq 1 ]]; then
  printf '%s\n' "$STAGE"
elif [[ "$JSON_OUTPUT" -eq 1 ]]; then
  print_json_output
else
  print_human_output
fi
