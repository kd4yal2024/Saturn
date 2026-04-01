#!/usr/bin/env bash
# xdma-stage-current.sh - Web-manager wrapper for safe XDMA staging on the running kernel

set -euo pipefail

PRIVILEGED_DIR="${SATURN_SATURNGO_PRIVILEGED_SCRIPTS_DIR:-/usr/local/lib/saturn-go/scripts}"
HELPER="${PRIVILEGED_DIR}/saturn-xdma-stage-current.sh"

if [[ ! -x "$HELPER" ]]; then
  echo "ERR: XDMA stage helper not found: $HELPER" >&2
  exit 1
fi

echo "Staging XDMA for the running kernel..."
echo "Helper: $HELPER"
echo

if ! command -v sudo >/dev/null 2>&1; then
  echo "ERR: sudo is required to run the privileged XDMA stage helper." >&2
  exit 1
fi

exec sudo -n "$HELPER"
