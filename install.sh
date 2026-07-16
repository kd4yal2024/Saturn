#!/usr/bin/env bash
# Stable one-command entry point for Saturn appliance installation.

set -Eeuo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export SATURN_REPO_DIR="${SATURN_REPO_DIR:-$REPO_ROOT}"
exec bash "$REPO_ROOT/scripts/install-saturn-appliance.sh" "$@"
