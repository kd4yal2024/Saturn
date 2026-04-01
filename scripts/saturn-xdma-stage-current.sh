#!/usr/bin/env bash
set -euo pipefail

FIX_XDMA_SCRIPT="${SATURN_XDMA_FIX_SCRIPT:-/usr/local/bin/saturn-fix-xdma.sh}"
RUNNING_KERNEL="$(uname -r)"
SATURN_XDMA_TRANSIENT_MARKER="${SATURN_XDMA_TRANSIENT_MARKER:-0}"

running_under_saturngo_service() {
  grep -q 'saturn-go\.service' /proc/self/cgroup 2>/dev/null
}

maybe_reexec_via_systemd_run() {
  [[ "$SATURN_XDMA_TRANSIENT_MARKER" == "1" ]] && return 0
  running_under_saturngo_service || return 0
  command -v systemd-run >/dev/null 2>&1 || return 0
  systemd-run --help 2>&1 | grep -q -- '--pipe' || return 0

  local unit="saturn-xdma-stage-$(date +%Y%m%d%H%M%S)-$$"
  exec systemd-run --quiet --wait --collect --pipe --service-type=exec \
    --unit "$unit" \
    --setenv=SATURN_XDMA_TRANSIENT_MARKER=1 \
    "$0" "$@"
}

if [[ $# -ne 0 ]]; then
  printf 'ERROR: saturn-xdma-stage-current.sh does not accept arguments.\n' >&2
  exit 2
fi

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  printf 'ERROR: Run as root.\n' >&2
  exit 1
fi

if [[ ! -x "$FIX_XDMA_SCRIPT" ]]; then
  printf 'ERROR: XDMA staging helper not found: %s\n' "$FIX_XDMA_SCRIPT" >&2
  exit 1
fi

maybe_reexec_via_systemd_run "$@"

printf 'Staging XDMA for running kernel: %s\n' "$RUNNING_KERNEL"
printf 'This does not stop p2app.service or unload the live xdma module.\n'
printf '\n'

exec "$FIX_XDMA_SCRIPT" --stage-kernel "$RUNNING_KERNEL"
