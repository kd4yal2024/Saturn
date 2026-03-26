#!/usr/bin/env bash
# xdma-doctor.sh - Web-manager wrapper for the privileged Saturn XDMA doctor

set -euo pipefail

PRIVILEGED_DIR="${SATURN_SATURNGO_PRIVILEGED_SCRIPTS_DIR:-/usr/local/lib/saturn-go/scripts}"
HELPER="${PRIVILEGED_DIR}/saturn-xdma-doctor.sh"

if [[ ! -x "$HELPER" ]]; then
  echo "ERR: XDMA doctor helper not found: $HELPER" >&2
  exit 1
fi

echo "Running Saturn XDMA Doctor..."
echo "Helper: $HELPER"
echo

if ! command -v sudo >/dev/null 2>&1; then
  echo "ERR: sudo is required to run the privileged XDMA doctor helper." >&2
  exit 1
fi

exec sudo -n "$HELPER" "$@"
