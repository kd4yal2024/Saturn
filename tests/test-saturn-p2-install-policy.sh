#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
# shellcheck source=../scripts/saturn-p2-install-policy.sh
source "$REPO_ROOT/scripts/saturn-p2-install-policy.sh"

export SATURN_RADIO_BACKEND_STATE_FILE="$TMP_DIR/selection.json"
[[ "$(saturn_p2_selected_backend)" == p2 ]]
printf '%s\n' '{"active":"p2","status":"ready"}' >"$SATURN_RADIO_BACKEND_STATE_FILE"
[[ "$(saturn_p2_selected_backend)" == p2 ]]
printf '%s\n' '{"active":"xdma","status":"ready"}' >"$SATURN_RADIO_BACKEND_STATE_FILE"
[[ "$(saturn_p2_selected_backend)" == xdma ]]
printf '%s\n' '{"active":"unknown"}' >"$SATURN_RADIO_BACKEND_STATE_FILE"
if saturn_p2_selected_backend >/dev/null 2>&1; then
  printf 'invalid selection was accepted\n' >&2
  exit 1
fi
: >"$SATURN_RADIO_BACKEND_STATE_FILE"
if saturn_p2_selected_backend >/dev/null 2>&1; then
  printf 'empty selection was accepted\n' >&2
  exit 1
fi

grep -Fq 'if [[ "$SELECTED_BACKEND" == "xdma" ]]' "$REPO_ROOT/sw_tools/p2app-control/install.sh"
grep -Fq 'sudo systemctl disable --now "${UNIT_NAME}"' "$REPO_ROOT/sw_tools/p2app-control/install.sh"
printf 'Saturn P2 installation policy tests passed\n'
