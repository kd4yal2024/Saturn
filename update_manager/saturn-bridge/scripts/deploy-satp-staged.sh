#!/usr/bin/env bash
# Copy beside a sealed SATP bundle as deploy.sh. No implicit install action.
set -Eeuo pipefail
case "${1:-}" in
    --check|--install) [[ $# == 1 ]] ;;
    --rollback) [[ $# == 2 ]] ;;
    *) echo "Usage: bash $0 --check | --install | --rollback BACKUP" >&2; exit 2 ;;
esac
cd -- "$(dirname -- "$0")"
sha256sum --check --quiet INSTALLER-SHA256SUMS
exec python3 ./installer.py "$@"
