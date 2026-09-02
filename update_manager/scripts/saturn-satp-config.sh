#!/usr/bin/env bash
# Configure the Phase 0E SATP v1 UDP receiver. This helper only changes the
# bridge subsystem environment; it never selects a radio backend or starts an
# inactive bridge. Active bridge restarts go through the ownership broker.

set -euo pipefail

TEST_MODE="${SATURN_SATP_CONFIG_TEST_MODE:-0}"
SERVICE_NAME="saturn-bridge.service"
SYSTEMD_ROOT="/etc/systemd/system"
BACKEND_HELPER="/usr/local/lib/saturn-go/scripts/saturn-radio-backend-switch-root.sh"
if [[ "$TEST_MODE" == "1" ]]; then
  SYSTEMD_ROOT="${SATURN_SATP_CONFIG_SYSTEMD_ROOT:-}"
  BACKEND_HELPER="${SATURN_SATP_CONFIG_BACKEND_HELPER:-}"
  [[ "$SYSTEMD_ROOT" == /tmp/* ]] || { echo "ERR: test systemd root must be below /tmp" >&2; exit 1; }
  [[ "$BACKEND_HELPER" == /tmp/* ]] || { echo "ERR: test backend helper must be below /tmp" >&2; exit 1; }
elif [[ "$TEST_MODE" != "0" ]]; then
  echo "ERR: SATURN_SATP_CONFIG_TEST_MODE must be 0 or 1" >&2
  exit 1
elif (( EUID != 0 )); then
  echo "ERR: SATP config helper must run as root" >&2
  exit 1
fi

DROPIN_DIR="${SYSTEMD_ROOT}/${SERVICE_NAME}.d"
DROPIN_FILE="$DROPIN_DIR/satp.conf"

die() { echo "ERR: $*" >&2; exit 1; }

valid_ipv4() {
  local ip="$1" IFS='.'
  local -a octets=($ip)
  [[ "$ip" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]] || return 1
  for octet in "${octets[@]}"; do
    [[ "$octet" =~ ^0[0-9]+$ ]] && return 1
    (( octet >= 0 && octet <= 255 )) || return 1
  done
}

valid_uint() { [[ "$1" =~ ^[0-9]+$ ]]; }
valid_port() { valid_uint "$1" && (( 10#$1 >= 1 && 10#$1 <= 65535 )); }

cmd="${1:-}"
case "$cmd" in
  show)
    systemctl show "$SERVICE_NAME" -p Environment --value \
      | tr ' ' '\n' | grep '^SATURN_BRIDGE_SATP_' || true
    ;;
  set)
    enabled="${2:-}"
    host="${3:-}"
    port="${4:-}"
    allowed_source="${5:--}"
    target="${6:-512}"
    capacity="${7:-4096}"
    timeout_ms="${8:-250}"
    [[ "$enabled" == "0" || "$enabled" == "1" ]] || die "enabled must be 0 or 1"
    valid_ipv4 "$host" || die "host must be a plain IPv4 address: $host"
    valid_port "$port" || die "port must be 1-65535: $port"
    [[ "$allowed_source" == "-" ]] || valid_ipv4 "$allowed_source" \
      || die "allowed source must be '-' or a plain IPv4 address"
    valid_uint "$target" && valid_uint "$capacity" || die "jitter sizes must be integers"
    (( target >= 128 && target <= capacity && capacity >= 512 && capacity <= 65536 )) \
      || die "jitter target/capacity are outside safe bounds"
    (( target % 128 == 0 && capacity % 128 == 0 )) \
      || die "jitter target/capacity must be multiples of 128 frames"
    valid_uint "$timeout_ms" && (( timeout_ms >= 100 && timeout_ms <= 5000 )) \
      || die "audio loss timeout must be 100-5000 ms"
    [[ -x "$BACKEND_HELPER" ]] || die "radio ownership broker is missing: $BACKEND_HELPER"

    was_active=0
    systemctl is-active --quiet "$SERVICE_NAME" && was_active=1
    backup_dir="$(mktemp -d)"
    trap 'rm -rf "$backup_dir"' EXIT
    existed=0
    if [[ -f "$DROPIN_FILE" ]]; then
      existed=1
      install -m 0644 "$DROPIN_FILE" "$backup_dir/satp.conf"
    fi
    restore_dropin() {
      if (( existed == 1 )); then
        install -m 0644 "$backup_dir/satp.conf" "$DROPIN_FILE"
      else
        rm -f "$DROPIN_FILE"
      fi
      systemctl daemon-reload || true
    }

    install -d -m 0755 "$DROPIN_DIR"
    tmp_file="$(mktemp "$DROPIN_DIR/.satp.XXXXXX")"
    trap '[[ -z "${tmp_file:-}" ]] || rm -f "$tmp_file"; rm -rf "$backup_dir"' EXIT
    {
      printf '%s\n' '[Service]'
      printf 'Environment=SATURN_BRIDGE_SATP_ENABLED=%s\n' "$enabled"
      printf 'Environment=SATURN_BRIDGE_SATP_HOST=%s\n' "$host"
      printf 'Environment=SATURN_BRIDGE_SATP_PORT=%s\n' "$port"
      printf 'Environment=SATURN_BRIDGE_SATP_JITTER_TARGET_FRAMES=%s\n' "$target"
      printf 'Environment=SATURN_BRIDGE_SATP_JITTER_CAPACITY_FRAMES=%s\n' "$capacity"
      printf 'Environment=SATURN_BRIDGE_SATP_AUDIO_LOSS_TIMEOUT_MS=%s\n' "$timeout_ms"
      if [[ "$allowed_source" != "-" ]]; then
        printf 'Environment=SATURN_BRIDGE_SATP_ALLOWED_SOURCE_IP=%s\n' "$allowed_source"
      fi
    } >"$tmp_file"
    chmod 0644 "$tmp_file"
    mv -f "$tmp_file" "$DROPIN_FILE"
    tmp_file=""
    if ! systemctl daemon-reload; then
      restore_dropin
      die "systemd reload failed; restored previous SATP configuration"
    fi
    if (( was_active == 0 )); then
      echo "SATP configuration saved; ${SERVICE_NAME} remains inactive until manually started"
      exit 0
    fi
    if "$BACKEND_HELPER" restart bridge; then
      echo "SATP receiver configured at ${host}:${port}; bridge restarted through ownership broker"
      exit 0
    fi
    restore_dropin
    "$BACKEND_HELPER" restart bridge || true
    die "bridge restart failed; restored previous SATP configuration"
    ;;
  *) die "usage: $0 {show|set <0|1> <host> <port> <allowed-ip|-> <target> <capacity> <timeout-ms>}" ;;
esac
