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
# Native gpio-keys policy:
#   - a live pwr_button input disables the polling fallback
#   - a per-user XDG override retires Raspberry Pi OS's desktop power-key
#     inhibitor after the next login/reboot, allowing logind to power off
#   - duplicate gpio-shutdown overlays are reported but never edited silently
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
SYSTEM_POWER_KEY_AUTOSTART="${SATURN_SYSTEM_POWER_KEY_AUTOSTART:-/etc/xdg/autostart/pwrkey.desktop}"
POWER_KEY_OVERRIDE_NAME="pwrkey.desktop"
POWER_KEY_OVERRIDE_MARKER="X-Saturn-Native-Power-Button=true"

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

validate_args() {
  case "$DEFAULT_ENABLED" in
    auto|true|false) ;;
    *) die "invalid --enabled-default: $DEFAULT_ENABLED" ;;
  esac
  [[ "$SATURN_USER" =~ ^[a-z_][a-z0-9_-]{0,31}$ ]] || die "invalid --saturn-user: $SATURN_USER"
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
  local saturn_home autostart_dir
  saturn_home="$(getent passwd "$SATURN_USER" | cut -d: -f6 || true)"
  [[ -n "$saturn_home" ]] || {
    log "No home directory found for $SATURN_USER; legacy autostart cleanup skipped"
    return 0
  }
  autostart_dir="${saturn_home}/.config/autostart"
  local legacy_entry="${autostart_dir}/g2-shutdown.desktop"
  if [[ -f "$legacy_entry" ]]; then
    rm -f "$legacy_entry"
    log "Removed legacy autostart entry: $legacy_entry"
  fi
}

saturn_user_home() {
  getent passwd "$SATURN_USER" | cut -d: -f6 || true
}

write_native_power_key_override() {
  local saturn_home="${1:-}" autostart_dir override group
  [[ -n "$saturn_home" ]] || return 0
  [[ -f "$SYSTEM_POWER_KEY_AUTOSTART" ]] || {
    log "No desktop power-key inhibitor entry found at $SYSTEM_POWER_KEY_AUTOSTART"
    return 0
  }

  autostart_dir="${saturn_home}/.config/autostart"
  override="${autostart_dir}/${POWER_KEY_OVERRIDE_NAME}"
  if [[ -e "$override" ]] && ! grep -Fq "$POWER_KEY_OVERRIDE_MARKER" "$override"; then
    log "Existing operator-owned power-key override preserved: $override"
    return 0
  fi
  group="$(id -gn "$SATURN_USER" 2>/dev/null || printf '%s' "$SATURN_USER")"
  install -d -m 0755 -o "$SATURN_USER" -g "$group" "$autostart_dir"
  cat > "$override" <<EOF
[Desktop Entry]
Type=Application
Name=Saturn native power-key handling
Hidden=true
$POWER_KEY_OVERRIDE_MARKER
EOF
  chmod 0644 "$override"
  chown "$SATURN_USER:$group" "$override"
  log "Disabled the desktop handle-power-key inhibitor for $SATURN_USER: $override"
}

remove_native_power_key_override() {
  local saturn_home="${1:-}" override
  [[ -n "$saturn_home" ]] || return 0
  override="${saturn_home}/.config/autostart/${POWER_KEY_OVERRIDE_NAME}"
  if [[ -f "$override" ]] && grep -Fq "$POWER_KEY_OVERRIDE_MARKER" "$override"; then
    rm -f "$override"
    log "Removed obsolete Saturn-managed power-key inhibitor override: $override"
  fi
}

apply_power_button_policy() {
  local saturn_home diagnostics overlay line
  saturn_home="$(saturn_user_home)"
  diagnostics="$("$DEST_SCRIPT" --diagnose 2>/dev/null || true)"
  while IFS= read -r line; do
    [[ -n "$line" ]] && log "Power-button diagnostic: $line"
  done <<< "$diagnostics"

  if "$DEST_SCRIPT" --probe-native-power-button; then
    overlay="$(sed -n 's/^gpio_shutdown_overlay=//p' <<< "$diagnostics" | tail -n 1)"
    if [[ -n "$overlay" && "$overlay" != "none" ]]; then
      log "WARNING: native pwr_button and gpio-shutdown both claim the configured shutdown GPIO; boot config was left unchanged"
      log "WARNING: inspect $overlay in /boot/firmware/config.txt and remove only the verified duplicate"
    fi
    write_native_power_key_override "$saturn_home"
    systemctl disable --now saturn-shutdown-waiter.service >/dev/null 2>&1 || true
    log "Native gpio-keys KEY_POWER input detected; polling waiter disabled"
    log "A reboot or desktop logout/login is required to retire any currently running power-key inhibitor"
    return 0
  fi

  remove_native_power_key_override "$saturn_home"
  return 1
}

main() {
  parse_args "$@"
  validate_args
  require_root
  [[ -f "$SRC_SCRIPT" ]] || die "missing script: $SRC_SCRIPT"

  install -D -m 0755 "$SRC_SCRIPT" "$DEST_SCRIPT"
  log "Installed script: $DEST_SCRIPT"

  write_unit_file
  write_default_config_if_missing
  remove_legacy_autostart_entry

  systemctl daemon-reload

  if apply_power_button_policy; then
    log "Installed native power-button policy; saturn-shutdown-waiter.service remains available as a disabled fallback"
    exit 0
  fi

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

if [[ "${SATURN_SHUTDOWN_WAITER_SOURCE_ONLY:-0}" != "1" ]]; then
  main "$@"
fi
