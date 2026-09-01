#!/usr/bin/env bash
# saturn-tci-bind.sh
# Sets the Saturn Bridge TCI server's bind host/port via a systemd drop-in.
# If the bridge is already active, it is restarted through the appliance radio
# ownership broker. An inactive bridge remains inactive so a settings change
# cannot take ownership away from a clean-boot Protocol 2 session.
# Invoked via NOPASSWD sudo from rust-server
# (Saturn Go's Settings page). Every argument is strictly validated here as
# well as by the caller (defense in depth) - this script never interpolates
# unvalidated input into the systemd unit file.
#
# Subcommands:
#   show                 Print the currently configured host/port.
#   set <host> <port>    Validate and apply a new host/port. Restart the bridge
#                        through the ownership broker only when already active.

set -euo pipefail

TEST_MODE="${SATURN_TCI_BIND_TEST_MODE:-0}"
SERVICE_NAME="saturn-bridge.service"
SYSTEMD_ROOT="/etc/systemd/system"
BACKEND_HELPER="/usr/local/lib/saturn-go/scripts/saturn-radio-backend-switch-root.sh"
if [[ "$TEST_MODE" == "1" ]]; then
  SYSTEMD_ROOT="${SATURN_TCI_BIND_SYSTEMD_ROOT:-}"
  BACKEND_HELPER="${SATURN_TCI_BIND_BACKEND_HELPER:-}"
  [[ "$SYSTEMD_ROOT" == /tmp/* ]] \
    || { echo "ERR: test systemd root must be below /tmp" >&2; exit 1; }
  [[ "$BACKEND_HELPER" == /tmp/* ]] \
    || { echo "ERR: test backend helper must be below /tmp" >&2; exit 1; }
elif [[ "$TEST_MODE" != "0" ]]; then
  echo "ERR: SATURN_TCI_BIND_TEST_MODE must be 0 or 1" >&2
  exit 1
elif (( EUID != 0 )); then
  echo "ERR: TCI bind helper must run as root" >&2
  exit 1
fi
DROPIN_DIR="${SYSTEMD_ROOT}/${SERVICE_NAME}.d"
DROPIN_FILE="$DROPIN_DIR/tci-bind.conf"

die() { echo "ERR: $*" >&2; exit 1; }

valid_ipv4() {
  local ip="$1" IFS='.'
  local -a octets=($ip)
  [[ "$ip" =~ ^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$ ]] || return 1
  for octet in "${octets[@]}"; do
    # Reject leading zeros (e.g. "010") to avoid octal-parsing ambiguity
    # in downstream consumers.
    [[ "$octet" =~ ^0[0-9]+$ ]] && return 1
    (( octet >= 0 && octet <= 255 )) || return 1
  done
  return 0
}

valid_port() {
  [[ "$1" =~ ^[0-9]{1,5}$ ]] || return 1
  (( 10#$1 >= 1 && 10#$1 <= 65535 ))
}

cmd="${1:-}"
case "$cmd" in
  show)
    systemctl show "$SERVICE_NAME" -p Environment --value \
      | tr ' ' '\n' | grep '^SATURN_BRIDGE_TCI_' || true
    ;;
  set)
    host="${2:-}"
    port="${3:-}"
    valid_ipv4 "$host" || die "invalid host (must be a plain IPv4 address): $host"
    valid_port "$port" || die "invalid port (must be 1-65535): $port"

    [[ -x "$BACKEND_HELPER" ]] \
      || die "radio ownership broker is missing or not executable: $BACKEND_HELPER"

    was_active=0
    systemctl is-active --quiet "$SERVICE_NAME" && was_active=1
    backup_dir="$(mktemp -d)"
    trap 'rm -rf "$backup_dir"' EXIT
    dropin_existed=0
    if [[ -f "$DROPIN_FILE" ]]; then
      dropin_existed=1
      install -m 0644 "$DROPIN_FILE" "$backup_dir/tci-bind.conf"
    fi
    restore_dropin() {
      if (( dropin_existed == 1 )); then
        install -m 0644 "$backup_dir/tci-bind.conf" "$DROPIN_FILE"
      else
        rm -f "$DROPIN_FILE"
      fi
      systemctl daemon-reload || true
    }

    install -d -m 0755 "$DROPIN_DIR"
    tmp_file="$(mktemp "$DROPIN_DIR/.tci-bind.XXXXXX")"
    trap '[[ -z "${tmp_file:-}" ]] || rm -f "$tmp_file"; rm -rf "$backup_dir"' EXIT
    printf '%s\n' \
      '[Service]' \
      "Environment=SATURN_BRIDGE_TCI_HOST=$host" \
      "Environment=SATURN_BRIDGE_TCI_PORT=$port" >"$tmp_file"
    chmod 0644 "$tmp_file"
    mv -f "$tmp_file" "$DROPIN_FILE"
    tmp_file=""
    if ! systemctl daemon-reload; then
      restore_dropin
      die "systemd reload failed; restored the previous TCI bind configuration"
    fi

    if (( was_active == 0 )); then
      echo "TCI bind saved as ${host}:${port}; ${SERVICE_NAME} remains inactive until manually started"
      exit 0
    fi

    restart_rc=0
    "$BACKEND_HELPER" restart bridge || restart_rc=$?
    if (( restart_rc == 0 )); then
      echo "TCI bind set to ${host}:${port}; ${SERVICE_NAME} restarted through the radio ownership broker"
      exit 0
    fi

    restore_dropin
    "$BACKEND_HELPER" restart bridge || true
    die "bridge restart failed (status $restart_rc); restored the previous TCI bind configuration"
    ;;
  *)
    die "usage: $0 {show|set <host> <port>}"
    ;;
esac
