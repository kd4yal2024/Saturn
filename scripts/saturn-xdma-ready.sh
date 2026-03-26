#!/usr/bin/env bash
set -euo pipefail

SATURN_XDMA_DOCTOR_PATH="${SATURN_XDMA_DOCTOR_PATH:-/usr/local/bin/saturn-xdma-doctor.sh}"
SATURN_XDMA_READY_TIMEOUT_SECONDS="${SATURN_XDMA_READY_TIMEOUT_SECONDS:-20}"
SATURN_XDMA_READY_POLL_SECONDS="${SATURN_XDMA_READY_POLL_SECONDS:-1}"

log() {
  printf '[saturn-xdma-ready] %s\n' "$*" >&2
}

have() {
  command -v "$1" >/dev/null 2>&1
}

if [[ ! -x "$SATURN_XDMA_DOCTOR_PATH" ]]; then
  log "doctor script not found/executable: ${SATURN_XDMA_DOCTOR_PATH}"
  exit 1
fi

if [[ ! "$SATURN_XDMA_READY_TIMEOUT_SECONDS" =~ ^[0-9]+$ ]] || (( SATURN_XDMA_READY_TIMEOUT_SECONDS < 1 )); then
  log "invalid SATURN_XDMA_READY_TIMEOUT_SECONDS=${SATURN_XDMA_READY_TIMEOUT_SECONDS}"
  exit 2
fi
if [[ ! "$SATURN_XDMA_READY_POLL_SECONDS" =~ ^[0-9]+$ ]] || (( SATURN_XDMA_READY_POLL_SECONDS < 1 )); then
  log "invalid SATURN_XDMA_READY_POLL_SECONDS=${SATURN_XDMA_READY_POLL_SECONDS}"
  exit 2
fi

last_stage="UNKNOWN"
deadline=$(( SECONDS + SATURN_XDMA_READY_TIMEOUT_SECONDS ))
doctor_args=(--skip-service-check --stage-only)
try_modprobe_sent=0

while (( SECONDS < deadline )); do
  if [[ "$try_modprobe_sent" -eq 0 && "$(id -u)" -eq 0 ]]; then
    stage="$("$SATURN_XDMA_DOCTOR_PATH" --skip-service-check --stage-only --try-modprobe 2>/dev/null || true)"
    try_modprobe_sent=1
  else
    stage="$("$SATURN_XDMA_DOCTOR_PATH" "${doctor_args[@]}" 2>/dev/null || true)"
  fi

  stage="${stage:-UNKNOWN}"
  last_stage="$stage"

  case "$stage" in
    OK)
      log "XDMA path is ready."
      exit 0
      ;;
    XDMA_UDEV_SYMLINK_MISSING)
      # p2app opens /dev/xdma0_user directly, so the missing friendly symlink
      # does not block app startup.
      log "kernel XDMA node is ready; friendly udev symlink is still missing."
      exit 0
      ;;
  esac

  sleep "$SATURN_XDMA_READY_POLL_SECONDS"
done

log "timed out waiting for XDMA readiness (last stage: ${last_stage})"
"$SATURN_XDMA_DOCTOR_PATH" --skip-service-check || true
exit 1
