#!/usr/bin/env bash
set -Eeuo pipefail

# Transactionally select the one appliance-wide owner of the Saturn FPGA.
#
# Direct XDMA remains probe-only. The installed configuration therefore keeps
# XDMA_OPERATIONAL_ENABLED=0, causing a request for the XDMA backend to fail
# before any service or state mutation. Test mode can exercise the completed
# transaction and rollback paths without touching production paths.

CONFIG_FILE="${SATURN_RADIO_BACKEND_CONFIG:-/etc/default/saturn-radio-backend}"
XDMA_OPERATIONAL_ENABLED=0
STATE_FILE="/var/lib/saturn-radio-backend/selection.json"
TRANSACTION_FILE="/run/saturn-radio-backend/transaction.json"
LOCK_FILE="/run/lock/saturn-maintenance/radio.lock"
SYSTEMD_ROOT="/etc/systemd/system"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
BRIDGE_DROPIN_NAME="20-radio-backend.conf"
READY_TIMEOUT_SECONDS=15
STATE_GROUP="pi"
TEST_MODE="${SATURN_RADIO_BACKEND_TEST_MODE:-0}"

TARGET_BACKEND=""
PREVIOUS_BACKEND="p2"
P2APP_WAS_ACTIVE=0
BRIDGE_WAS_ACTIVE=0
DROPIN_EXISTED=0
STATE_EXISTED=0
TRANSACTION_ACTIVE=0
ROLLBACK_DIR=""
BRIDGE_DROPIN=""

log() { printf '[saturn-radio-backend] %s\n' "$*"; }
die() { printf '[saturn-radio-backend] ERROR: %s\n' "$*" >&2; return 1; }
need_cmd() { command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage() {
  cat <<'EOF'
Usage:
  saturn-radio-backend-switch-root.sh status
  saturn-radio-backend-switch-root.sh switch p2
  saturn-radio-backend-switch-root.sh switch xdma

The selection is appliance-wide. Direct XDMA activation remains disabled until
the production backend is implemented and explicitly enabled by root-owned
configuration.
EOF
}

trim_config_value() {
  local value="$1"
  value="${value#\"}"
  value="${value%\"}"
  value="${value#\'}"
  value="${value%\'}"
  printf '%s' "$value"
}

load_config() {
  [[ -f "$CONFIG_FILE" ]] || return 0
  local owner mode line key value
  if (( EUID == 0 )); then
    owner="$(stat -c '%u' "$CONFIG_FILE")"
    mode="$(stat -c '%a' "$CONFIG_FILE")"
    [[ "$owner" == "0" ]] || die "backend config is not root-owned: $CONFIG_FILE"
    (( (8#$mode & 8#022) == 0 )) \
      || die "backend config is group/world writable: $CONFIG_FILE"
  fi

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    [[ "$line" == *=* ]] || die "invalid backend config line: $line"
    key="${line%%=*}"
    value="$(trim_config_value "${line#*=}")"
    case "$key" in
      XDMA_OPERATIONAL_ENABLED) XDMA_OPERATIONAL_ENABLED="$value" ;;
      STATE_FILE) STATE_FILE="$value" ;;
      TRANSACTION_FILE) TRANSACTION_FILE="$value" ;;
      LOCK_FILE) LOCK_FILE="$value" ;;
      SYSTEMD_ROOT) SYSTEMD_ROOT="$value" ;;
      BRIDGE_SERVICE) BRIDGE_SERVICE="$value" ;;
      P2APP_SERVICE) P2APP_SERVICE="$value" ;;
      BRIDGE_DROPIN_NAME) BRIDGE_DROPIN_NAME="$value" ;;
      READY_TIMEOUT_SECONDS) READY_TIMEOUT_SECONDS="$value" ;;
      STATE_GROUP) STATE_GROUP="$value" ;;
      *) die "unsupported backend config key: $key" ;;
    esac
  done <"$CONFIG_FILE"
}

validate_configuration() {
  local path service
  [[ "$XDMA_OPERATIONAL_ENABLED" == "0" || "$XDMA_OPERATIONAL_ENABLED" == "1" ]] \
    || die "XDMA_OPERATIONAL_ENABLED must be 0 or 1"
  [[ "$READY_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
    || die "READY_TIMEOUT_SECONDS must be a positive integer"
  (( READY_TIMEOUT_SECONDS <= 120 )) \
    || die "READY_TIMEOUT_SECONDS must not exceed 120"
  [[ "$TEST_MODE" == "0" || "$TEST_MODE" == "1" ]] \
    || die "SATURN_RADIO_BACKEND_TEST_MODE must be 0 or 1"

  for path in "$STATE_FILE" "$TRANSACTION_FILE" "$LOCK_FILE" "$SYSTEMD_ROOT"; do
    [[ "$path" == /* && "$path" != *[$'\t\r\n ']* ]] \
      || die "unsafe configured path: $path"
  done
  [[ "$BRIDGE_DROPIN_NAME" =~ ^[A-Za-z0-9_.-]+\.conf$ ]] \
    || die "unsafe bridge drop-in name: $BRIDGE_DROPIN_NAME"
  for service in "$BRIDGE_SERVICE" "$P2APP_SERVICE"; do
    [[ "$service" =~ ^[A-Za-z0-9_.@-]+\.service$ ]] \
      || die "unsafe service name: $service"
  done
  [[ "$STATE_GROUP" =~ ^[A-Za-z_][A-Za-z0-9_-]*$ ]] \
    || die "unsafe state group: $STATE_GROUP"

  BRIDGE_DROPIN="$SYSTEMD_ROOT/${BRIDGE_SERVICE}.d/$BRIDGE_DROPIN_NAME"
  if (( EUID != 0 )); then
    [[ "$TEST_MODE" == "1" ]] || die "backend switch must run as root"
    [[ "$SYSTEMD_ROOT" != "/etc/systemd/system" ]] \
      || die "non-root test mode refuses the production systemd root"
    [[ "$STATE_FILE" != /var/lib/* && "$LOCK_FILE" != /run/* ]] \
      || die "non-root test mode refuses production state and lock paths"
  fi
  if [[ "$XDMA_OPERATIONAL_ENABLED" == "1" && "$TEST_MODE" != "1" ]]; then
    die "direct XDMA is still probe-only; production activation cannot be enabled"
  fi
  if (( EUID == 0 )); then
    getent group "$STATE_GROUP" >/dev/null 2>&1 \
      || die "state group does not exist: $STATE_GROUP"
  fi
}

ensure_state_directories() {
  local directory owner mode
  for directory in "$(dirname "$STATE_FILE")" "$(dirname "$TRANSACTION_FILE")"; do
    if [[ -e "$directory" && (! -d "$directory" || -L "$directory") ]]; then
      die "refusing unsafe state directory: $directory"
    fi
    if (( EUID == 0 )); then
      install -d -m 0750 -o root -g "$STATE_GROUP" "$directory"
      owner="$(stat -c '%u' "$directory")"
      mode="$(stat -c '%a' "$directory")"
      [[ "$owner" == "0" ]] || die "state directory is not root-owned: $directory"
      (( (8#$mode & 8#022) == 0 )) \
        || die "state directory is group/world writable: $directory"
    else
      install -d -m 0750 "$directory"
    fi
  done
}

ensure_lock_directory() {
  local directory owner mode
  directory="$(dirname "$LOCK_FILE")"
  [[ -d "$directory" && ! -L "$directory" ]] \
    || die "radio lock directory is missing or unsafe: $directory"
  if (( EUID == 0 )); then
    owner="$(stat -c '%u' "$directory")"
    mode="$(stat -c '%a' "$directory")"
    [[ "$owner" == "0" ]] || die "radio lock directory is not root-owned: $directory"
    (( (8#$mode & 8#022) == 0 )) \
      || die "radio lock directory is group/world writable: $directory"
  fi
}

service_is_active() {
  systemctl is-active --quiet "$1"
}

wait_for_service_state() {
  local service="$1" expected="$2" elapsed=0
  while (( elapsed < READY_TIMEOUT_SECONDS )); do
    if [[ "$expected" == "active" ]]; then
      service_is_active "$service" && return 0
    else
      ! service_is_active "$service" && return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  die "$service did not become $expected within ${READY_TIMEOUT_SECONDS}s"
}

read_selected_backend() {
  [[ -f "$STATE_FILE" ]] || {
    printf 'p2\n'
    return 0
  }
  python3 - "$STATE_FILE" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        value = json.load(handle).get("active", "p2")
except (OSError, ValueError, TypeError):
    value = "p2"
print(value if value in {"p2", "xdma"} else "p2")
PY
}

write_json_state() {
  local path="$1" requested="$2" active="$3" status="$4"
  install -d -m 0750 "$(dirname "$path")"
  python3 - "$path" "$requested" "$active" "$status" <<'PY'
import json
import os
import sys
import tempfile
import time

path, requested, active, status = sys.argv[1:]
directory = os.path.dirname(path)
fd, temporary = tempfile.mkstemp(prefix=".radio-backend-", dir=directory)
try:
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump(
            {
                "schema_version": 1,
                "updated_at_ms": int(time.time() * 1000),
                "requested": requested,
                "active": active,
                "status": status,
            },
            handle,
            indent=2,
            sort_keys=True,
        )
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    os.chmod(temporary, 0o640)
    os.replace(temporary, path)
    directory_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)
finally:
    try:
        os.unlink(temporary)
    except FileNotFoundError:
        pass
PY
  if (( EUID == 0 )); then
    chown "root:$STATE_GROUP" "$path"
  fi
}

remove_transaction_file() {
  rm -f "$TRANSACTION_FILE"
  rmdir "$(dirname "$TRANSACTION_FILE")" 2>/dev/null || true
}

write_bridge_dropin() {
  local backend="$1" directory temporary
  directory="$(dirname "$BRIDGE_DROPIN")"
  install -d -m 0755 "$directory"
  temporary="$(mktemp "$directory/.${BRIDGE_DROPIN_NAME}.XXXXXX")"
  if [[ "$backend" == "p2" ]]; then
    cat >"$temporary" <<EOF
[Service]
Environment=SATURN_BRIDGE_RADIO_BACKEND=p2
EOF
  else
    cat >"$temporary" <<'EOF'
[Service]
Environment=SATURN_BRIDGE_RADIO_BACKEND=xdma
EOF
  fi
  chmod 0644 "$temporary"
  (( EUID != 0 )) || chown root:root "$temporary"
  mv -f -- "$temporary" "$BRIDGE_DROPIN"
}

bridge_runtime_backend() {
  local environment
  environment="$(systemctl show --property=Environment --value "$BRIDGE_SERVICE")"
  case " $environment " in
    *" SATURN_BRIDGE_RADIO_BACKEND=p2 "*) printf 'p2\n' ;;
    *" SATURN_BRIDGE_RADIO_BACKEND=xdma "*) printf 'xdma\n' ;;
    *) printf 'unknown\n' ;;
  esac
}

verify_target_ready() {
  local backend="$1" runtime_backend
  wait_for_service_state "$BRIDGE_SERVICE" active
  runtime_backend="$(bridge_runtime_backend)"
  [[ "$runtime_backend" == "$backend" ]] \
    || die "$BRIDGE_SERVICE started with backend '$runtime_backend', expected '$backend'"

  if [[ "$backend" == "p2" ]]; then
    wait_for_service_state "$P2APP_SERVICE" active
  else
    wait_for_service_state "$P2APP_SERVICE" inactive
  fi
}

restore_file_snapshot() {
  local existed="$1" snapshot="$2" destination="$3" directory_mode="$4"
  if [[ "$existed" == "1" ]]; then
    install -d -m "$directory_mode" "$(dirname "$destination")"
    cp -p "$snapshot" "$destination"
  else
    rm -f "$destination"
  fi
}

rollback() {
  local original_status="${1:-1}" rollback_failed=0
  (( TRANSACTION_ACTIVE == 1 )) || return "$original_status"
  TRANSACTION_ACTIVE=0
  log "Switch failed; restoring the previous backend transaction"

  systemctl stop "$BRIDGE_SERVICE" >/dev/null 2>&1 || rollback_failed=1
  restore_file_snapshot "$DROPIN_EXISTED" "$ROLLBACK_DIR/bridge-dropin" \
    "$BRIDGE_DROPIN" 0755 || rollback_failed=1
  systemctl daemon-reload >/dev/null 2>&1 || rollback_failed=1

  if (( P2APP_WAS_ACTIVE == 1 )); then
    systemctl start "$P2APP_SERVICE" >/dev/null 2>&1 || rollback_failed=1
  else
    systemctl stop "$P2APP_SERVICE" >/dev/null 2>&1 || rollback_failed=1
  fi
  if (( BRIDGE_WAS_ACTIVE == 1 )); then
    systemctl start "$BRIDGE_SERVICE" >/dev/null 2>&1 || rollback_failed=1
  else
    systemctl stop "$BRIDGE_SERVICE" >/dev/null 2>&1 || rollback_failed=1
  fi

  restore_file_snapshot "$STATE_EXISTED" "$ROLLBACK_DIR/state" "$STATE_FILE" \
    0750 || rollback_failed=1
  remove_transaction_file
  if (( rollback_failed == 1 )); then
    printf '[saturn-radio-backend] ERROR: automatic rollback was incomplete\n' >&2
  else
    log "Rollback restored backend '$PREVIOUS_BACKEND'"
  fi
  return "$original_status"
}

on_error() {
  local status="$1" line="$2"
  trap - ERR INT TERM
  printf '[saturn-radio-backend] ERROR: transaction failed at line %s\n' "$line" >&2
  rollback "$status"
  exit "$status"
}

prepare_transaction() {
  ROLLBACK_DIR="$(mktemp -d)"
  PREVIOUS_BACKEND="$(read_selected_backend)"
  service_is_active "$P2APP_SERVICE" && P2APP_WAS_ACTIVE=1
  service_is_active "$BRIDGE_SERVICE" && BRIDGE_WAS_ACTIVE=1
  if [[ -f "$BRIDGE_DROPIN" ]]; then
    DROPIN_EXISTED=1
    cp -p "$BRIDGE_DROPIN" "$ROLLBACK_DIR/bridge-dropin"
  fi
  if [[ -f "$STATE_FILE" ]]; then
    STATE_EXISTED=1
    cp -p "$STATE_FILE" "$ROLLBACK_DIR/state"
  fi
  write_json_state "$TRANSACTION_FILE" "$TARGET_BACKEND" "$PREVIOUS_BACKEND" "switching"
  TRANSACTION_ACTIVE=1
}

switch_backend() {
  if [[ "$TARGET_BACKEND" == "xdma" && "$XDMA_OPERATIONAL_ENABLED" != "1" ]]; then
    die "direct XDMA is still probe-only; no service or persistent state was changed"
  fi

  prepare_transaction
  trap 'on_error "$?" "$LINENO"' ERR
  trap 'on_error 130 "$LINENO"' INT TERM

  # Stopping the bridge is the current global force-RX/release boundary. The
  # production direct backend will add its explicit RX-safe shutdown handshake
  # before XDMA_OPERATIONAL_ENABLED can be enabled outside fixture tests.
  systemctl stop "$BRIDGE_SERVICE"
  wait_for_service_state "$BRIDGE_SERVICE" inactive

  if [[ "$TARGET_BACKEND" == "xdma" ]]; then
    systemctl stop "$P2APP_SERVICE"
    wait_for_service_state "$P2APP_SERVICE" inactive
    write_bridge_dropin xdma
    systemctl daemon-reload
    systemctl start "$BRIDGE_SERVICE"
  else
    write_bridge_dropin p2
    systemctl daemon-reload
    systemctl start "$P2APP_SERVICE"
    wait_for_service_state "$P2APP_SERVICE" active
    systemctl start "$BRIDGE_SERVICE"
  fi

  verify_target_ready "$TARGET_BACKEND"
  write_json_state "$STATE_FILE" "$TARGET_BACKEND" "$TARGET_BACKEND" "ready"
  TRANSACTION_ACTIVE=0
  trap - ERR INT TERM
  remove_transaction_file
  rm -rf "$ROLLBACK_DIR"
  ROLLBACK_DIR=""
  log "Backend '$TARGET_BACKEND' is ready and persisted"
}

print_status() {
  local selected p2_status bridge_status runtime_backend
  selected="$(read_selected_backend)"
  p2_status="inactive"
  bridge_status="inactive"
  service_is_active "$P2APP_SERVICE" && p2_status="active"
  service_is_active "$BRIDGE_SERVICE" && bridge_status="active"
  runtime_backend="$(bridge_runtime_backend)"
  python3 - "$selected" "$runtime_backend" "$p2_status" "$bridge_status" \
    "$XDMA_OPERATIONAL_ENABLED" "$TRANSACTION_FILE" <<'PY'
import json
import sys

selected, runtime, p2, bridge, xdma_enabled, transaction_path = sys.argv[1:]
requested = selected
transaction_status = "idle"
try:
    with open(transaction_path, encoding="utf-8") as handle:
        transaction = json.load(handle)
    requested = transaction.get("requested", selected)
    transaction_status = transaction.get("status", "switching")
except (OSError, ValueError, TypeError):
    pass
print(json.dumps({
    "schema_version": 1,
    "requested": requested,
    "selected": selected,
    "runtime": runtime,
    "transaction_status": transaction_status,
    "services": {"p2app": p2, "saturn_bridge": bridge},
    "mutual_exclusion_ok": not (runtime == "xdma" and p2 == "active"),
    "xdma_operational_enabled": xdma_enabled == "1",
}, indent=2, sort_keys=True))
PY
}

main() {
  local command="${1:-}"
  case "$command" in
    status)
      [[ $# == 1 ]] || {
        usage >&2
        return 2
      }
      ;;
    switch)
      [[ $# == 2 && ("$2" == "p2" || "$2" == "xdma") ]] || {
        usage >&2
        return 2
      }
      TARGET_BACKEND="$2"
      ;;
    *)
      usage >&2
      return 2
      ;;
  esac

  load_config
  validate_configuration
  need_cmd systemctl
  need_cmd python3
  need_cmd flock
  need_cmd install
  need_cmd mktemp

  if [[ "$TARGET_BACKEND" == "xdma" && "$XDMA_OPERATIONAL_ENABLED" != "1" ]]; then
    die "direct XDMA is still probe-only; no service or persistent state was changed"
  fi
  if [[ "$command" == "status" ]]; then
    print_status
    return 0
  fi

  ensure_state_directories
  ensure_lock_directory
  exec 9>"$LOCK_FILE"
  flock -w 30 9 || die "timed out waiting for the radio ownership lock"
  switch_backend
}

main "$@"
