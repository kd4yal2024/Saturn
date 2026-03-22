#!/usr/bin/env bash
set -euo pipefail

# Installs Saturn shutdown waiter as a systemd service and migrates away from
# legacy desktop-autostart startup.
#
# Environment knobs:
#   SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT (default: auto)
#     -> written only when /etc/default/saturn-shutdown-waiter does not exist
#   SATURN_USER (default: pi)
#
# Optional flags:
#   --enabled-default <mode>
#   --saturn-user <user>

SCRIPT_SELF="$(readlink -f "${BASH_SOURCE[0]:-$0}" 2>/dev/null || printf '%s\n' "${BASH_SOURCE[0]:-$0}")"
PRIVILEGED_SCRIPT_PATH="${SATURN_PRIVILEGED_SCRIPT_PATH:-/usr/local/lib/saturn-go/scripts/$(basename "$SCRIPT_SELF")}"
HERE="$(cd "$(dirname "$SCRIPT_SELF")" && pwd)"
REPO_ROOT="$(cd "$HERE/.." && pwd)"

SRC_SCRIPT="${REPO_ROOT}/scripts/shutdown-waiter.sh"
DEST_SCRIPT="/usr/local/sbin/saturn-shutdown-waiter.sh"
UNIT_PATH="/etc/systemd/system/saturn-shutdown-waiter.service"
CONF_PATH="/etc/default/saturn-shutdown-waiter"

SATURN_USER="${SATURN_USER:-pi}"
DEFAULT_ENABLED="${SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT:-auto}"

log() { printf '[shutdown-waiter-install] %s\n' "$*"; }
die() { printf '[shutdown-waiter-install] ERROR: %s\n' "$*" >&2; exit 1; }
has_tty() { [[ -t 0 || -t 1 ]]; }

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --enabled-default)
        shift
        [[ $# -gt 0 ]] || die "--enabled-default requires a value"
        DEFAULT_ENABLED="$1"
        shift
        ;;
      --saturn-user)
        shift
        [[ $# -gt 0 ]] || die "--saturn-user requires a value"
        SATURN_USER="$1"
        shift
        ;;
      -h|--help)
        cat <<'EOF'
Usage: install-shutdown-waiter-service.sh [options]
  --enabled-default <mode>  Set SATURN_SHUTDOWN_WAITER_ENABLED default (auto|true|false)
  --saturn-user <user>      User whose legacy autostart entry should be removed
EOF
        exit 0
        ;;
      *)
        die "Unknown argument: $1"
        ;;
    esac
  done
}

require_root() {
  [[ "$(id -u)" -eq 0 ]] && return 0
  command -v sudo >/dev/null 2>&1 || die "run as root or install sudo"

  local target="$PRIVILEGED_SCRIPT_PATH"
  if [[ ! -x "$target" ]]; then
    if has_tty; then
      target="$SCRIPT_SELF"
    else
      die "installed privileged copy not found at $PRIVILEGED_SCRIPT_PATH"
    fi
  fi

  if has_tty; then
    exec sudo "$target" --enabled-default "$DEFAULT_ENABLED" --saturn-user "$SATURN_USER"
  fi
  exec sudo -n "$target" --enabled-default "$DEFAULT_ENABLED" --saturn-user "$SATURN_USER"
}

write_unit_file() {
  cat > "$UNIT_PATH" <<'EOF'
[Unit]
Description=Saturn shutdown waiter
After=multi-user.target

[Service]
Type=simple
ExecStart=/usr/local/sbin/saturn-shutdown-waiter.sh
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
EOF
  chmod 0644 "$UNIT_PATH"
}

write_default_config_if_missing() {
  if [[ -f "$CONF_PATH" ]]; then
    log "Keeping existing config: $CONF_PATH"
    return 0
  fi

  cat > "$CONF_PATH" <<EOF
# Saturn shutdown waiter configuration.
# Modes: true|false|auto
SATURN_SHUTDOWN_WAITER_ENABLED=${DEFAULT_ENABLED}
SATURN_SHUTDOWN_WAITER_GPIO_CHIP=gpiochip0
SATURN_SHUTDOWN_WAITER_GPIO_LINE=26
SATURN_SHUTDOWN_WAITER_ARM_DELAY_SEC=20
SATURN_SHUTDOWN_WAITER_POLL_SEC=1
SATURN_SHUTDOWN_WAITER_LOW_CONFIRM_COUNT=3
SATURN_SHUTDOWN_WAITER_REQUIRE_HIGH_BEFORE_ARM=1
SATURN_SHUTDOWN_WAITER_I2C_BUS=1
SATURN_SHUTDOWN_WAITER_I2C_ADDR=0x20
EOF
  chmod 0644 "$CONF_PATH"
  log "Wrote default config: $CONF_PATH (ENABLED=${DEFAULT_ENABLED})"
}

remove_legacy_autostart_entry() {
  local autostart_dir="/home/${SATURN_USER}/.config/autostart"
  local legacy_entry="${autostart_dir}/g2-shutdown.desktop"
  if [[ -f "$legacy_entry" ]]; then
    rm -f "$legacy_entry"
    log "Removed legacy autostart entry: $legacy_entry"
  fi
}

main() {
  parse_args "$@"
  require_root
  [[ -f "$SRC_SCRIPT" ]] || die "missing script: $SRC_SCRIPT"

  install -D -m 0755 "$SRC_SCRIPT" "$DEST_SCRIPT"
  log "Installed script: $DEST_SCRIPT"

  write_unit_file
  write_default_config_if_missing
  remove_legacy_autostart_entry

  systemctl daemon-reload

  if [[ "$(systemctl is-enabled saturn-shutdown-waiter.service 2>/dev/null || true)" == "masked" ]]; then
    log "Service is masked; leaving masked and skipping enable/start"
    exit 0
  fi

  systemctl enable saturn-shutdown-waiter.service >/dev/null
  if systemctl is-active --quiet saturn-shutdown-waiter.service; then
    systemctl restart saturn-shutdown-waiter.service
  else
    systemctl start saturn-shutdown-waiter.service
  fi

  log "Installed and started saturn-shutdown-waiter.service"
}

main "$@"
