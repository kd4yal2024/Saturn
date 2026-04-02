#!/usr/bin/env bash
# flash_fpga.sh - Web-manager wrapper for the privileged Saturn FPGA flash helper

set -euo pipefail

PRIVILEGED_DIR="${SATURN_SATURNGO_PRIVILEGED_SCRIPTS_DIR:-/usr/local/lib/saturn-go/scripts}"
HELPER="${PRIVILEGED_DIR}/saturn-flash-fpga.sh"

if [[ ! -x "$HELPER" ]]; then
  echo "ERR: FPGA flash helper not found: $HELPER" >&2
  exit 1
fi

echo "Running Saturn FPGA flash..."
echo "Helper: $HELPER"
echo

if ! command -v sudo >/dev/null 2>&1; then
  echo "ERR: sudo is required to run the privileged FPGA flash helper." >&2
  exit 1
fi

exec sudo -n "$HELPER" "$@"
