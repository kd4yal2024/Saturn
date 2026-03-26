#!/usr/bin/env bash
# saturn-xdma-kernel-postinst.sh
# Kernel post-install hook helper that pre-stages the XDMA module for a newly
# installed kernel without disturbing the currently running module/service.

set -euo pipefail

TARGET_KERNEL="${1:-}"
TARGET_IMAGE="${2:-}"
LOG_FILE="${SATURN_XDMA_POSTINST_LOG:-/var/log/saturn-xdma-postinst.log}"
FIX_XDMA_SCRIPT="${SATURN_XDMA_FIX_SCRIPT:-/usr/local/bin/saturn-fix-xdma.sh}"
LOG_TO_FILE=0

timestamp(){ date '+%Y-%m-%d %H:%M:%S'; }

emit_log_line(){
  local level="$1"
  shift
  local line
  line="$(printf '[%s] [%s] %s' "$(timestamp)" "$level" "$*")"

  if [[ "$level" == "WARN" ]]; then
    printf '%s\n' "$line" >&2
  else
    printf '%s\n' "$line"
  fi

  if [[ "$LOG_TO_FILE" -eq 1 ]]; then
    printf '%s\n' "$line" >&3 2>/dev/null || true
  fi
}

info(){ emit_log_line INFO "$*"; }
warn(){ emit_log_line WARN "$*"; }

setup_logging(){
  mkdir -p "$(dirname "$LOG_FILE")" 2>/dev/null || true
  if ! exec 3>>"$LOG_FILE" 2>/dev/null; then
    warn "Unable to write ${LOG_FILE}; continuing without file logging."
    return 0
  fi
  if ! printf '' >&3 2>/dev/null; then
    exec 3>&-
    warn "Unable to write ${LOG_FILE}; continuing without file logging."
    return 0
  fi
  LOG_TO_FILE=1
}

main(){
  if [[ -z "$TARGET_KERNEL" ]]; then
    warn "No kernel release argument supplied; skipping XDMA staging."
    exit 0
  fi

  setup_logging

  info "Kernel postinst hook invoked for kernel=${TARGET_KERNEL} image=${TARGET_IMAGE:-unknown}"

  if [[ ! -x "$FIX_XDMA_SCRIPT" ]]; then
    warn "XDMA staging helper not found: $FIX_XDMA_SCRIPT"
    exit 0
  fi

  if ! "$FIX_XDMA_SCRIPT" --stage-kernel "$TARGET_KERNEL"; then
    warn "XDMA staging failed for ${TARGET_KERNEL}; kernel install will continue."
    exit 0
  fi

  info "XDMA staged successfully for ${TARGET_KERNEL}"
}

main "$@"
