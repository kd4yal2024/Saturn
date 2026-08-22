#!/usr/bin/env bash
set -Eeuo pipefail

SYSTEMD_RUN="/usr/bin/systemd-run"
SYSTEMCTL="/usr/bin/systemctl"
DELAY_SECONDS="${SATURN_APPLIANCE_POWEROFF_DELAY_SECONDS:-3}"

log() { printf '[saturn-appliance-power] %s\n' "$*"; }
die() { printf '[saturn-appliance-power] ERROR: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: saturn-appliance-power-root.sh schedule-poweroff

Schedules a systemd-owned G2 poweroff after a short delay. Radio services must
already have been stopped through the appliance radio ownership broker.
EOF
}

[[ ${EUID:-$(id -u)} -eq 0 ]] || die "run as root"
[[ "$DELAY_SECONDS" =~ ^[1-9][0-9]*$ ]] || die "invalid poweroff delay"
(( DELAY_SECONDS <= 15 )) || die "poweroff delay must not exceed 15 seconds"
[[ -x "$SYSTEMD_RUN" ]] || die "systemd-run is unavailable"
[[ -x "$SYSTEMCTL" ]] || die "systemctl is unavailable"

case "${1:-}" in
  schedule-poweroff)
    [[ $# == 1 ]] || {
      usage >&2
      exit 2
    }
    unit="saturn-g2-poweroff-$(date +%s)-$$"
    "$SYSTEMD_RUN" \
      --unit "$unit" \
      --collect \
      --on-active "${DELAY_SECONDS}s" \
      --property Type=oneshot \
      "$SYSTEMCTL" poweroff
    log "G2 poweroff scheduled in ${DELAY_SECONDS} seconds"
    ;;
  -h|--help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
