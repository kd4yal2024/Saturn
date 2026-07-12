#!/usr/bin/env bash
set -euo pipefail

# Compatibility entry point. Service creation is centralized in the
# p2app-control installer so legacy callers cannot recreate the old root service
# that executed a user-owned binary from the Git checkout.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
INSTALLER="$REPO_ROOT/sw_tools/p2app-control/install.sh"

[[ -x "$INSTALLER" ]] || {
  printf 'ERROR: secure p2app installer not found: %s\n' "$INSTALLER" >&2
  exit 1
}

exec "$INSTALLER" "$@"
