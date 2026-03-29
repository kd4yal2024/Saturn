#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
Usage: saturn-provision-powerctl.sh <reboot|poweroff|shutdown>
EOF
}

action="${1:-}"
case "$action" in
  reboot)
    exec /usr/bin/systemctl reboot
    ;;
  poweroff|shutdown)
    exec /usr/bin/systemctl poweroff
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
