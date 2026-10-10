#!/usr/bin/env bash
set -Eeuo pipefail

# Atomically select one already-installed Saturn application release and
# restart only the services that consume the versioned application payload.
#
# Production activation is intentionally disabled by default until the
# rollback transaction is appliance-tested and an operator explicitly enables
# it. Validation remains available while activation is disabled.

CONFIG_FILE="${SATURN_RELEASE_ACTIVATE_CONFIG:-/etc/default/saturn-release-activate}"
VALIDATE_ONLY=0
APPROVE_ONE_WAY_MIGRATION=0
ACTIVATION_ENABLED=0
SATURN_ROOT="/opt/saturn"
RELEASES_ROOT="/opt/saturn/releases"
CURRENT_LINK="/opt/saturn/current"
TRANSACTION_FILE="/var/lib/saturn-state/deployments/current.json"
LOCK_FILE="/run/lock/saturn-release-activate.lock"
MANIFEST_TOOL="/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py"
COMPONENTS_FILE="/usr/local/lib/saturn-go/release/components-v1.json"
STATE_TOOL="/usr/local/lib/saturn-go/scripts/saturn-state-compatibility.py"
STATE_ROOT="/var/lib/saturn-state"
STATE_BACKUP_ROOT="/var/lib/saturn-state/deployments/state-backups"
SYSTEMD_ROOT="/etc/systemd/system"
SATURN_GO_SERVICE="saturn-go.service"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
SATURN_GO_READY_URL="http://127.0.0.1:8080/readyz"
P23_PERF_URL="http://127.0.0.1:8080/p23_perf"
BACKEND_STATUS_HELPER="/usr/local/lib/saturn-go/scripts/saturn-radio-backend-switch-root.sh"
READY_TIMEOUT_SECONDS=30
P2APP_PANEL_ENABLED=0
TRANSACTION_GROUP="pi"
TRANSACTION_GID=""
PHASE="preflight"
TARGET_COMMIT=""
TARGET_RELEASE=""
PREVIOUS_COMMIT=""
PREVIOUS_RELEASE=""
PREVIOUS_READY_COMMIT=""
TRANSACTION_PREPARED=0
TEMP_POINTER=""
ROLLBACK_DIR=""
ACTIVATION_FAILURE_PHASE=""
ACTIVATION_FAILURE_COMMAND=""
ACTIVATION_FAILURE_STATUS=""
ROLLBACK_STATUS=""
ROLLBACK_MESSAGE=""
SATURN_GO_DROPIN=""
BRIDGE_DROPIN=""
P2APP_DROPIN=""
SATURN_GO_DROPIN_EXISTED=0
BRIDGE_DROPIN_EXISTED=0
P2APP_DROPIN_EXISTED=0
SATURN_GO_WAS_ACTIVE=0
BRIDGE_WAS_ACTIVE=0
P2APP_WAS_ACTIVE=0
STATE_PLAN_JSON=""
STATE_TOOL_ARGS=()
STATE_BACKUP_DIR=""
STATE_MIGRATED=0
SELECTED_BACKEND=""
RECONCILE_ROLLBACK=0
CHECK_RECEPTION=0
VALIDATION_ENABLED=1
VALIDATOR="/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py"
VALIDATOR_GUARD="/usr/local/lib/saturn-go/scripts/saturn-remote-receive-guard.js"
VALIDATION_TIMEOUT_SECONDS=150
VALIDATION_EXPECT_RXC1="any"
VALIDATION_VIEWER_ROLE="preferred"
VALIDATION_LAUNCHER="systemd"
VALIDATION_BROWSER="/usr/bin/chromium"
VALIDATION_URL="https://127.0.0.1:8443/remote-next?validation=receive&transport=split&display_transport=spectrum"
VALIDATION_TLS_CERT="/var/lib/saturn-state/remote-tls/saturn-remote.crt"
VALIDATION_PERF_JSON="/run/saturn-bridge/perf.json"
VALIDATION_LOG_ROOT="/var/lib/saturn-state/deployments/validation"
VALIDATION_MEMORY_MAX="350M"
VALIDATION_NO_SANDBOX=0
VALIDATION_CONTROL_DIR=""
VALIDATION_LABEL=""
VALIDATOR_PID=""
VALIDATION_WAITER_PID=""
VALIDATION_RECORD_JSON="{}"
ROLLBACK_RELEASE_RESTORED=""
ROLLBACK_RECEIVE_VERIFIED=""
ROLLBACK_RECEIVE_UNVERIFIED=0
RECONCILED_FROM=""

log(){ printf '[saturn-release-activate] %s\n' "$*"; }
die(){ printf '[saturn-release-activate] ERROR: %s\n' "$*" >&2; return 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage:
  saturn-release-activate-root.sh <full-commit>
  saturn-release-activate-root.sh --approve-one-way-migration <full-commit>
  saturn-release-activate-root.sh --validate <full-commit>
  saturn-release-activate-root.sh --reconcile-rollback
  saturn-release-activate-root.sh --check-reception

The validation form verifies an installed immutable release without changing
the host. Activation requires ACTIVATION_ENABLED=1 in the root-owned config.
The one-way form is accepted only when the release documents that rollback
compatibility will be lost and records the operator's explicit approval.

Activation proves reception itself: after the target services start, a
temporary receive-only headless browser session is attached to the existing
Bridge (see saturn-remote-validate.py) and held while the readiness gate runs,
so no person has to keep a browser connected. The same proof follows a rollback,
reported separately from the restoration of the previous release.

--reconcile-rollback clears a "rollback_failed" record, and only when the
recorded previous release is verifiably the one running and receiving.

--check-reception proves reception on the release that is running now, with
exactly the launcher an activation uses, and changes nothing: no pointer,
service, drop-in or transaction record.
EOF
}

trim_config_value(){
  local value="$1"
  value="${value#\"}"
  value="${value%\"}"
  value="${value#\'}"
  value="${value%\'}"
  printf '%s' "$value"
}

load_config(){
  [[ -f "$CONFIG_FILE" ]] || return 0
  local owner mode line key value
  if (( EUID == 0 )); then
    owner="$(stat -c '%u' "$CONFIG_FILE")"
    mode="$(stat -c '%a' "$CONFIG_FILE")"
    [[ "$owner" == "0" ]] || die "activation config is not root-owned: $CONFIG_FILE"
    (( (8#$mode & 8#022) == 0 )) || die "activation config is group/world writable: $CONFIG_FILE"
  fi

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    [[ "$line" == *=* ]] || die "invalid activation config line: $line"
    key="${line%%=*}"
    value="$(trim_config_value "${line#*=}")"
    case "$key" in
      ACTIVATION_ENABLED) ACTIVATION_ENABLED="$value" ;;
      SATURN_ROOT) SATURN_ROOT="$value" ;;
      RELEASES_ROOT) RELEASES_ROOT="$value" ;;
      CURRENT_LINK) CURRENT_LINK="$value" ;;
      TRANSACTION_FILE) TRANSACTION_FILE="$value" ;;
      LOCK_FILE) LOCK_FILE="$value" ;;
      MANIFEST_TOOL) MANIFEST_TOOL="$value" ;;
      COMPONENTS_FILE) COMPONENTS_FILE="$value" ;;
      STATE_TOOL) STATE_TOOL="$value" ;;
      STATE_ROOT) STATE_ROOT="$value" ;;
      STATE_BACKUP_ROOT) STATE_BACKUP_ROOT="$value" ;;
      SYSTEMD_ROOT) SYSTEMD_ROOT="$value" ;;
      SATURN_GO_SERVICE) SATURN_GO_SERVICE="$value" ;;
      BRIDGE_SERVICE) BRIDGE_SERVICE="$value" ;;
      P2APP_SERVICE) P2APP_SERVICE="$value" ;;
      SATURN_GO_READY_URL) SATURN_GO_READY_URL="$value" ;;
      P23_PERF_URL) P23_PERF_URL="$value" ;;
      BACKEND_STATUS_HELPER) BACKEND_STATUS_HELPER="$value" ;;
      READY_TIMEOUT_SECONDS) READY_TIMEOUT_SECONDS="$value" ;;
      P2APP_PANEL_ENABLED) P2APP_PANEL_ENABLED="$value" ;;
      TRANSACTION_GROUP) TRANSACTION_GROUP="$value" ;;
      VALIDATION_ENABLED) VALIDATION_ENABLED="$value" ;;
      VALIDATOR) VALIDATOR="$value" ;;
      VALIDATOR_GUARD) VALIDATOR_GUARD="$value" ;;
      VALIDATION_TIMEOUT_SECONDS) VALIDATION_TIMEOUT_SECONDS="$value" ;;
      VALIDATION_EXPECT_RXC1) VALIDATION_EXPECT_RXC1="$value" ;;
      VALIDATION_VIEWER_ROLE) VALIDATION_VIEWER_ROLE="$value" ;;
      VALIDATION_LAUNCHER) VALIDATION_LAUNCHER="$value" ;;
      VALIDATION_BROWSER) VALIDATION_BROWSER="$value" ;;
      VALIDATION_URL) VALIDATION_URL="$value" ;;
      VALIDATION_TLS_CERT) VALIDATION_TLS_CERT="$value" ;;
      VALIDATION_PERF_JSON) VALIDATION_PERF_JSON="$value" ;;
      VALIDATION_LOG_ROOT) VALIDATION_LOG_ROOT="$value" ;;
      VALIDATION_MEMORY_MAX) VALIDATION_MEMORY_MAX="$value" ;;
      VALIDATION_NO_SANDBOX) VALIDATION_NO_SANDBOX="$value" ;;
      *) die "unsupported activation config key: $key" ;;
    esac
  done <"$CONFIG_FILE"
}

validate_configuration(){
  local path service
  [[ "$ACTIVATION_ENABLED" == "0" || "$ACTIVATION_ENABLED" == "1" ]] \
    || die "ACTIVATION_ENABLED must be 0 or 1"
  [[ "$P2APP_PANEL_ENABLED" == "0" || "$P2APP_PANEL_ENABLED" == "1" ]] \
    || die "P2APP_PANEL_ENABLED must be 0 or 1"
  [[ "$READY_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
    || die "READY_TIMEOUT_SECONDS must be a positive integer"
  (( READY_TIMEOUT_SECONDS <= 300 )) || die "READY_TIMEOUT_SECONDS must not exceed 300"
  [[ "$VALIDATION_ENABLED" == "0" || "$VALIDATION_ENABLED" == "1" ]] \
    || die "VALIDATION_ENABLED must be 0 or 1"
  [[ "$VALIDATION_NO_SANDBOX" == "0" || "$VALIDATION_NO_SANDBOX" == "1" ]] \
    || die "VALIDATION_NO_SANDBOX must be 0 or 1"
  [[ "$VALIDATION_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
    || die "VALIDATION_TIMEOUT_SECONDS must be a positive integer"
  (( VALIDATION_TIMEOUT_SECONDS <= 600 )) || die "VALIDATION_TIMEOUT_SECONDS must not exceed 600"
  [[ "$VALIDATION_EXPECT_RXC1" =~ ^(any|valid|unarmed|disabled|unsupported|unavailable|partial)$ ]] \
    || die "VALIDATION_EXPECT_RXC1 must be any, valid, unarmed, disabled, unsupported, unavailable or partial"
  [[ "$VALIDATION_VIEWER_ROLE" == "required" || "$VALIDATION_VIEWER_ROLE" == "preferred" ]] \
    || die "VALIDATION_VIEWER_ROLE must be required or preferred"
  [[ "$VALIDATION_LAUNCHER" == "systemd" || "$VALIDATION_LAUNCHER" == "direct" ]] \
    || die "VALIDATION_LAUNCHER must be systemd or direct"
  if [[ "$VALIDATION_LAUNCHER" == "direct" ]] && (( EUID == 0 )); then
    die "the direct validation launcher would run the browser as root; use systemd"
  fi
  [[ "$VALIDATION_MEMORY_MAX" =~ ^[1-9][0-9]*[MG]$ ]] \
    || die "VALIDATION_MEMORY_MAX must look like 350M"
  local url_re='^https://127\.0\.0\.1:[0-9]+/[A-Za-z0-9_./-]+(\?[A-Za-z0-9_=&.-]*)?$'
  [[ "$VALIDATION_URL" =~ $url_re ]] || die "VALIDATION_URL must be an explicit loopback HTTPS URL"
  for path in \
    "$SATURN_ROOT" "$RELEASES_ROOT" "$CURRENT_LINK" "$TRANSACTION_FILE" "$LOCK_FILE" \
    "$MANIFEST_TOOL" "$COMPONENTS_FILE" "$SYSTEMD_ROOT" \
    "$STATE_TOOL" "$STATE_ROOT" "$STATE_BACKUP_ROOT" "$BACKEND_STATUS_HELPER" \
    "$VALIDATOR" "$VALIDATOR_GUARD" "$VALIDATION_BROWSER" "$VALIDATION_TLS_CERT" \
    "$VALIDATION_PERF_JSON" "$VALIDATION_LOG_ROOT"
  do
    [[ "$path" == /* && "$path" != *[$'\t\r\n ']* ]] || die "unsafe configured path: $path"
  done
  [[ "$(dirname "$RELEASES_ROOT")" == "$SATURN_ROOT" ]] \
    || die "RELEASES_ROOT must be directly beneath SATURN_ROOT"
  [[ "$(dirname "$CURRENT_LINK")" == "$SATURN_ROOT" ]] \
    || die "CURRENT_LINK must be directly beneath SATURN_ROOT"
  [[ "$(dirname "$STATE_BACKUP_ROOT")" == "$(dirname "$TRANSACTION_FILE")" ]] \
    || die "STATE_BACKUP_ROOT must be directly beneath the deployment transaction directory"
  for service in "$SATURN_GO_SERVICE" "$BRIDGE_SERVICE" "$P2APP_SERVICE"; do
    [[ "$service" =~ ^[A-Za-z0-9_.@-]+\.service$ ]] || die "unsafe service name: $service"
  done
  [[ "$TRANSACTION_GROUP" =~ ^[A-Za-z_][A-Za-z0-9_-]*$ ]] \
    || die "unsafe transaction group: $TRANSACTION_GROUP"
  getent group "$TRANSACTION_GROUP" >/dev/null 2>&1 \
    || die "transaction group does not exist: $TRANSACTION_GROUP"
  TRANSACTION_GID="$(getent group "$TRANSACTION_GROUP" | cut -d: -f3)"
  [[ "$TRANSACTION_GID" =~ ^[0-9]+$ ]] || die "cannot resolve transaction group: $TRANSACTION_GROUP"
  [[ "$SATURN_GO_READY_URL" =~ ^http://127\.0\.0\.1:[0-9]+/[A-Za-z0-9_./-]+$ ]] \
    || die "SATURN_GO_READY_URL must be an explicit loopback HTTP endpoint"
  [[ "$P23_PERF_URL" =~ ^http://127\.0\.0\.1:[0-9]+/[A-Za-z0-9_./-]+$ ]] \
    || die "P23_PERF_URL must be an explicit loopback HTTP endpoint"
  [[ -x "$BACKEND_STATUS_HELPER" ]] || die "backend status helper is not executable: $BACKEND_STATUS_HELPER"

  if (( EUID != 0 )); then
    [[ "${SATURN_RELEASE_ACTIVATE_TEST_MODE:-0}" == "1" ]] \
      || die "release activation must run as root"
    [[ "$SATURN_ROOT" != "/opt/saturn" && "$SYSTEMD_ROOT" != "/etc/systemd/system" ]] \
      || die "non-root test mode refuses production paths"
  fi
}

release_identity(){
  python3 - "$1/release-manifest.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    manifest = json.load(handle)
source = manifest.get("source", {})
build = manifest.get("build", {})
print(f'{source.get("commit", "")}\t{build.get("architecture", "")}')
PY
}

validate_release(){
  local commit="$1" release canonical_saturn canonical_root canonical_release actual architecture bad identity
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || { die "target must be one lowercase full Git commit"; return 1; }
  [[ -d "$SATURN_ROOT" && ! -L "$SATURN_ROOT" ]] \
    || { die "Saturn application root is not a real directory: $SATURN_ROOT"; return 1; }
  [[ -d "$RELEASES_ROOT" && ! -L "$RELEASES_ROOT" ]] \
    || { die "release root is not a real directory: $RELEASES_ROOT"; return 1; }
  canonical_saturn="$(realpath -e "$SATURN_ROOT")" || return 1
  canonical_root="$(realpath -e "$RELEASES_ROOT")" || return 1
  [[ "$(dirname "$canonical_root")" == "$canonical_saturn" ]] \
    || { die "release root is not directly beneath the Saturn application root: $canonical_root"; return 1; }
  release="$RELEASES_ROOT/$commit"
  [[ -d "$release" && ! -L "$release" ]] || { die "installed release not found: $release"; return 1; }
  canonical_release="$(realpath -e "$release")" || return 1
  [[ "$(dirname "$canonical_release")" == "$canonical_root" ]] \
    || { die "release is not a direct child of the trusted release root: $canonical_release"; return 1; }
  bad="$(find "$canonical_release" -xdev -type l -print -quit)" || return 1
  [[ -z "$bad" ]] || { die "symbolic links are not permitted inside a release: $bad"; return 1; }
  bad="$(find "$canonical_release" -xdev ! -type d ! -type f -print -quit)" || return 1
  [[ -z "$bad" ]] || { die "non-regular release entry rejected: $bad"; return 1; }
  bad="$(find "$canonical_release" -xdev -perm /022 -print -quit)" || return 1
  [[ -z "$bad" ]] || { die "group/world-writable release entry rejected: $bad"; return 1; }
  bad="$(find "$canonical_release" -xdev -type d ! -perm 0755 -print -quit)" || return 1
  [[ -z "$bad" ]] || { die "release directory must use mode 0755: $bad"; return 1; }
  if (( EUID == 0 )); then
    bad="$(find "$canonical_release" -xdev \( ! -user root -o ! -group root \) -print -quit)" || return 1
    [[ -z "$bad" ]] || { die "installed release ownership mismatch: $bad"; return 1; }
  fi
  python3 "$MANIFEST_TOOL" validate \
    --release-root "$canonical_release" \
    --components "$COMPONENTS_FILE" >/dev/null \
    || { die "release manifest validation failed: $canonical_release"; return 1; }
  # A process substitution can retain FD 9 after this helper exits. Wait for
  # the identity reader before releasing the activation lock.
  identity="$(release_identity "$canonical_release")" || return 1
  IFS=$'\t' read -r actual architecture <<<"$identity"
  [[ "$actual" == "$commit" ]] || { die "installed release commit mismatch: $actual"; return 1; }
  [[ "$architecture" == "$(uname -m)" ]] \
    || { die "installed release architecture $architecture does not match host $(uname -m)"; return 1; }
  printf '%s\n' "$canonical_release"
}

current_release_identity(){
  local canonical_root resolved leaf
  if [[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]; then
    printf '\t\n'
    return 0
  fi
  [[ -L "$CURRENT_LINK" ]] || { die "current release pointer is not a symbolic link: $CURRENT_LINK"; return 1; }
  canonical_root="$(realpath -e "$RELEASES_ROOT")" || return 1
  resolved="$(realpath -e "$CURRENT_LINK")" || return 1
  [[ "$(dirname "$resolved")" == "$canonical_root" ]] \
    || { die "current release points outside the trusted release root: $resolved"; return 1; }
  leaf="$(basename "$resolved")"
  [[ "$leaf" =~ ^[0-9a-f]{40}$ ]] || { die "current release target is not a full commit: $resolved"; return 1; }
  printf '%s\t%s\n' "$leaf" "$resolved"
}

transaction_status(){
  [[ -f "$TRANSACTION_FILE" ]] || return 0
  python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        print(json.load(handle).get("status", ""))
except (OSError, json.JSONDecodeError):
    print("invalid")
PY
}

prepare_transaction_directory(){
  local directory owner mode
  directory="$(dirname "$TRANSACTION_FILE")"
  if (( EUID == 0 )); then
    install -d -m 0750 -o root -g "$TRANSACTION_GROUP" "$directory"
    [[ ! -L "$directory" ]] || die "transaction directory must not be a symbolic link: $directory"
    owner="$(stat -c '%u' "$directory")"
    mode="$(stat -c '%a' "$directory")"
    [[ "$owner" == "0" ]] || die "transaction directory is not root-owned: $directory"
    (( (8#$mode & 8#022) == 0 )) \
      || die "transaction directory is group/world writable: $directory"
  else
    install -d -m 0750 "$directory"
    [[ ! -L "$directory" ]] || die "transaction directory must not be a symbolic link: $directory"
  fi
  if [[ -e "$TRANSACTION_FILE" || -L "$TRANSACTION_FILE" ]]; then
    [[ -f "$TRANSACTION_FILE" && ! -L "$TRANSACTION_FILE" ]] \
      || die "transaction state must be a regular file: $TRANSACTION_FILE"
    if (( EUID == 0 )); then
      owner="$(stat -c '%u' "$TRANSACTION_FILE")"
      mode="$(stat -c '%a' "$TRANSACTION_FILE")"
      [[ "$owner" == "0" ]] || die "transaction state is not root-owned: $TRANSACTION_FILE"
      (( (8#$mode & 8#022) == 0 )) \
        || die "transaction state is group/world writable: $TRANSACTION_FILE"
    fi
  fi
}

write_transaction(){
  local status="$1" phase="$2" message="$3" mode="${4:-update}"
  python3 - \
    "$TRANSACTION_FILE" "$status" "$phase" "$message" "$mode" \
    "$TARGET_COMMIT" "$TARGET_RELEASE" "$PREVIOUS_COMMIT" "$PREVIOUS_RELEASE" \
    "$CURRENT_LINK" "$SATURN_GO_SERVICE" "$BRIDGE_SERVICE" "$P2APP_SERVICE" \
    "$SATURN_GO_DROPIN" "$SATURN_GO_DROPIN_EXISTED" \
    "$BRIDGE_DROPIN" "$BRIDGE_DROPIN_EXISTED" \
    "$P2APP_DROPIN" "$P2APP_DROPIN_EXISTED" "$TRANSACTION_GID" \
    "$PREVIOUS_READY_COMMIT" "$ROLLBACK_DIR" \
    "$SATURN_GO_WAS_ACTIVE" "$BRIDGE_WAS_ACTIVE" "$P2APP_WAS_ACTIVE" "$SELECTED_BACKEND" \
    "$ACTIVATION_FAILURE_PHASE" "$ACTIVATION_FAILURE_COMMAND" \
    "$ACTIVATION_FAILURE_STATUS" "$ROLLBACK_STATUS" "$ROLLBACK_MESSAGE" \
    "$STATE_PLAN_JSON" "$STATE_BACKUP_DIR" "$STATE_MIGRATED" \
    "$APPROVE_ONE_WAY_MIGRATION" \
    "$VALIDATION_RECORD_JSON" "$ROLLBACK_RELEASE_RESTORED" "$ROLLBACK_RECEIVE_VERIFIED" \
    "$RECONCILED_FROM" <<'PY'
import json
import os
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

(
    path_text, status, phase, message, mode, target_commit, target_release,
    previous_commit, previous_release, current_link, saturn_go, bridge, p2app,
    saturn_go_dropin, saturn_go_dropin_existed,
    bridge_dropin, bridge_dropin_existed,
    p2app_dropin, p2app_dropin_existed,
    transaction_gid,
    previous_ready_commit, rollback_directory,
    saturn_go_was_active, bridge_was_active, p2app_was_active, selected_backend,
    failure_phase, failure_command, failure_status,
    rollback_status, rollback_message,
    state_plan_json, state_backup_directory, state_migrated,
    one_way_approved,
    validation_json, release_restored, receive_verified, reconciled_from,
) = sys.argv[1:]
path = Path(path_text)
path.parent.mkdir(parents=True, exist_ok=True)
now = datetime.now(timezone.utc).isoformat()
value = {}
if mode != "prepare" and path.exists():
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
if mode == "prepare":
    value = {
        "format": "saturn-deployment-transaction",
        "schema_version": 1,
        "transaction_id": f"{target_commit}-{os.getpid()}",
        "created_at": now,
        "target_commit": target_commit,
        "target_release": target_release,
        "previous_commit": previous_commit or None,
        "previous_release": previous_release or None,
        "previous_ready_commit": previous_ready_commit,
        "current_link": current_link,
        "rollback_directory": rollback_directory,
        "services": {
            "stop_order": [saturn_go, bridge, p2app],
            "start_order": ([p2app] + ([bridge] if bridge_was_active == "1" else [])
                            if selected_backend == "p2" else [bridge]) + [saturn_go],
            "selected_backend": selected_backend,
            "previously_active": {
                saturn_go: saturn_go_was_active == "1",
                bridge: bridge_was_active == "1",
                p2app: p2app_was_active == "1",
            },
        },
        "service_dropins": {
            saturn_go: {
                "path": saturn_go_dropin,
                "previously_existed": saturn_go_dropin_existed == "1",
                "backup": f"{rollback_directory}/{saturn_go}.conf" if saturn_go_dropin_existed == "1" else None,
            },
            bridge: {
                "path": bridge_dropin,
                "previously_existed": bridge_dropin_existed == "1",
                "backup": f"{rollback_directory}/{bridge}.conf" if bridge_dropin_existed == "1" else None,
            },
            p2app: {
                "path": p2app_dropin,
                "previously_existed": p2app_dropin_existed == "1",
                "backup": f"{rollback_directory}/{p2app}.conf" if p2app_dropin_existed == "1" else None,
            },
        },
    }
value.update({
    "status": status,
    "phase": phase,
    "message": message,
    "updated_at": now,
})
if failure_phase or failure_command:
    value["activation_failure"] = {
        "phase": failure_phase,
        "command": failure_command,
        "exit_status": int(failure_status),
    }
if rollback_status:
    value["rollback"] = {
        "status": rollback_status,
        "message": rollback_message,
        "updated_at": now,
    }
    # Restoring the previous release and proving that it receives are different facts, so they are recorded as different fields.
    if release_restored in ("true", "false"):
        value["rollback"]["release_restored"] = release_restored == "true"
    if receive_verified in ("true", "false"):
        value["rollback"]["receive_verified"] = receive_verified == "true"
    elif receive_verified:
        value["rollback"]["receive_verified"] = receive_verified
validation = json.loads(validation_json or "{}")
if validation:
    value["validation"] = validation
if reconciled_from:
    value["reconciliation"] = {"at": now, "original_record": reconciled_from}
state_plan = json.loads(state_plan_json)
value["state_compatibility"] = {
    "plan": state_plan,
    "backup_directory": state_backup_directory or None,
    "migrated": state_migrated == "1",
    "one_way_operator_approval": one_way_approved == "1",
}
def atomic_json(destination):
    fd, temporary = tempfile.mkstemp(prefix=f".{destination.name}.", dir=destination.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o640)
        if os.geteuid() == 0:
            os.chown(temporary, 0, int(transaction_gid))
        os.replace(temporary, destination)
        directory_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass

atomic_json(path.with_name(f"{path.name}.last-good"))
atomic_json(path)
PY
}

systemd_escape_path(){
  local value="$1"
  value="${value//%/%%}"
  printf '%s' "$value"
}

atomic_install_text(){
  local destination="$1" content="$2" directory temporary
  directory="$(dirname "$destination")"
  install -d -m 0755 "$directory"
  temporary="$(mktemp "$directory/.50-saturn-release.XXXXXX")"
  printf '%s' "$content" >"$temporary"
  chmod 0644 "$temporary"
  if (( EUID == 0 )); then
    chown root:root "$temporary"
  fi
  sync -f "$temporary"
  mv -Tf "$temporary" "$destination"
  sync -f "$directory"
}

atomic_install_file(){
  local source="$1" destination="$2" directory temporary
  if [[ ! -f "$source" || -L "$source" ]]; then
    printf '[saturn-release-activate] ERROR: rollback source is not a regular file: %s\n' "$source" >&2
    return 1
  fi
  directory="$(dirname "$destination")"
  install -d -m 0755 "$directory" || return 1
  temporary="$(mktemp "$directory/.50-saturn-release.XXXXXX")" || return 1
  install -m 0644 "$source" "$temporary" || {
    rm -f -- "$temporary"
    return 1
  }
  if (( EUID == 0 )); then
    chown root:root "$temporary" || {
      rm -f -- "$temporary"
      return 1
    }
  fi
  sync -f "$temporary" || {
    rm -f -- "$temporary"
    return 1
  }
  mv -Tf "$temporary" "$destination" || {
    rm -f -- "$temporary"
    return 1
  }
  sync -f "$directory" || return 1
}

snapshot_dropin(){
  local source="$1" existed="$2" backup="$3" owner mode
  [[ "$existed" == "1" ]] || return 0
  [[ -f "$source" && ! -L "$source" ]] || die "existing service drop-in is unsafe: $source"
  if (( EUID == 0 )); then
    owner="$(stat -c '%u' "$source")"
    mode="$(stat -c '%a' "$source")"
    [[ "$owner" == "0" ]] || die "existing service drop-in is not root-owned: $source"
    (( (8#$mode & 8#022) == 0 )) || die "existing service drop-in is group/world writable: $source"
  fi
  install -m 0644 "$source" "$backup"
  if (( EUID == 0 )); then
    chown root:"$TRANSACTION_GROUP" "$backup"
  fi
}

prepare_rollback_snapshot(){
  local parent
  parent="$(dirname "$TRANSACTION_FILE")"
  ROLLBACK_DIR="$parent/rollback-current"
  [[ "$ROLLBACK_DIR" != "/" ]] || die "unsafe rollback directory"
  rm -rf -- "$ROLLBACK_DIR"
  if (( EUID == 0 )); then
    install -d -m 0750 -o root -g "$TRANSACTION_GROUP" "$ROLLBACK_DIR"
  else
    install -d -m 0750 "$ROLLBACK_DIR"
  fi
  [[ ! -L "$ROLLBACK_DIR" ]] || die "rollback directory must not be a symbolic link: $ROLLBACK_DIR"
  snapshot_dropin "$SATURN_GO_DROPIN" "$SATURN_GO_DROPIN_EXISTED" "$ROLLBACK_DIR/$SATURN_GO_SERVICE.conf"
  snapshot_dropin "$BRIDGE_DROPIN" "$BRIDGE_DROPIN_EXISTED" "$ROLLBACK_DIR/$BRIDGE_SERVICE.conf"
  snapshot_dropin "$P2APP_DROPIN" "$P2APP_DROPIN_EXISTED" "$ROLLBACK_DIR/$P2APP_SERVICE.conf"
  sync -f "$ROLLBACK_DIR"
  sync -f "$parent"
}

install_release_dropins(){
  local current p2_args
  current="$(systemd_escape_path "$CURRENT_LINK")"
  p2_args="-s"
  [[ "$P2APP_PANEL_ENABLED" == "1" ]] && p2_args="-s -p"

  atomic_install_text "$SATURN_GO_DROPIN" "[Service]
ExecStart=
ExecStart=$current/bin/saturn-go
WorkingDirectory=$current
Environment=SATURN_WEBROOT=$current/webroot
"
  atomic_install_text "$BRIDGE_DROPIN" "[Service]
ExecStart=
ExecStart=$current/bin/saturn-bridge
WorkingDirectory=$current
"
  atomic_install_text "$P2APP_DROPIN" "[Service]
ExecStart=
ExecStart=$current/bin/p2app $p2_args
WorkingDirectory=$current
"
}

atomic_switch_pointer(){
  local release="$1" parent attempt
  parent="$(dirname "$CURRENT_LINK")"
  if [[ ! -d "$release" || -L "$release" ]]; then
    printf '[saturn-release-activate] ERROR: release pointer target is invalid: %s\n' "$release" >&2
    return 1
  fi
  if [[ "$(stat -c '%d' "$parent")" != "$(stat -c '%d' "$release")" ]]; then
    printf '[saturn-release-activate] ERROR: current pointer and target release must be on the same filesystem\n' >&2
    return 1
  fi
  if [[ -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]; then
    printf '[saturn-release-activate] ERROR: current release pointer cannot replace a real filesystem entry: %s\n' "$CURRENT_LINK" >&2
    return 1
  fi
  for attempt in {1..20}; do
    TEMP_POINTER="$parent/.current.$$.${RANDOM}.${attempt}"
    if ln -s "$release" "$TEMP_POINTER" 2>/dev/null; then
      break
    fi
    TEMP_POINTER=""
  done
  if [[ -z "$TEMP_POINTER" || ! -L "$TEMP_POINTER" ]]; then
    printf '[saturn-release-activate] ERROR: could not create temporary release pointer in %s\n' "$parent" >&2
    return 1
  fi
  if ! mv -Tf "$TEMP_POINTER" "$CURRENT_LINK"; then
    rm -f -- "$TEMP_POINTER"
    TEMP_POINTER=""
    return 1
  fi
  TEMP_POINTER=""
  if [[ "$(realpath -e "$CURRENT_LINK")" != "$release" ]]; then
    printf '[saturn-release-activate] ERROR: active release pointer did not resolve to target\n' >&2
    return 1
  fi
  sync -f "$parent" || return 1
}

remove_current_pointer(){
  local parent
  parent="$(dirname "$CURRENT_LINK")"
  if [[ -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]; then
    printf '[saturn-release-activate] ERROR: current release pointer is not removable: %s\n' "$CURRENT_LINK" >&2
    return 1
  fi
  rm -f -- "$CURRENT_LINK" || return 1
  sync -f "$parent" || return 1
}

ready_response_matches(){
  local response="$1" expected="$2"
  python3 - "$expected" "$response" <<'PY'
import json
import sys
expected = sys.argv[1]
try:
    value = json.loads(sys.argv[2])
except json.JSONDecodeError:
    raise SystemExit(1)
if value.get("ready") is not True:
    raise SystemExit(1)
if value.get("build_commit") != expected:
    raise SystemExit(1)
if value.get("expected_commit") != expected:
    raise SystemExit(1)
PY
}

probe_running_commit(){
  local response commit
  response="$(curl -fsS --max-time 2 "${SATURN_GO_READY_URL%%\?*}")" || return 1
  commit="$(python3 - "$response" <<'PY'
import json
import re
import sys
try:
    value = json.loads(sys.argv[1])
except json.JSONDecodeError:
    raise SystemExit(1)
commit = value.get("build_commit", "")
if value.get("ready") is not True or not re.fullmatch(r"[0-9a-f]{40}", commit):
    raise SystemExit(1)
print(commit)
PY
)" || return 1
  printf '%s\n' "$commit"
}

wait_for_commit(){
  local expected="$1" elapsed=0 url response
  url="${SATURN_GO_READY_URL%%\?*}?expected_commit=$expected"
  while (( elapsed < READY_TIMEOUT_SECONDS )); do
    if response="$(curl -fsS --max-time 2 "$url" 2>/dev/null)"; then
      if ready_response_matches "$response" "$expected"; then
        return 0
      fi
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  response="$(curl -fsS --max-time 2 "$url")"
  ready_response_matches "$response" "$expected"
}

probe_web_pages(){
  local origin="${SATURN_GO_READY_URL%/*}" response
  response="$(curl -fsS --max-time 2 "$origin/telemetry")" || return 1
  [[ "$response" == *'<title>Saturn G2 Radio Telemetry</title>'* ]] || return 1
  response="$(curl -fsS --max-time 2 "$origin/settings")" || return 1
  [[ "$response" == *'<title>Saturn Go Settings</title>'* ]]
}

selected_backend_status(){
  local response
  response="$("$BACKEND_STATUS_HELPER" status)" || return 1
  python3 - "$response" <<'PY'
import json
import sys
try:
    value = json.loads(sys.argv[1])
except (ValueError, TypeError):
    raise SystemExit(1)
selected = value.get("selected")
if selected not in ("p2", "xdma"):
    raise SystemExit(1)
if value.get("runtime") != selected or value.get("operational_status") != "ready":
    raise SystemExit(1)
if value.get("transaction_status") != "idle" or value.get("mutual_exclusion_ok") is not True:
    raise SystemExit(1)
services = value.get("services") or {}
if selected == "p2" and (services.get("p2app") != "active" or services.get("saturn_bridge") not in ("active", "inactive")):
    raise SystemExit(1)
if selected == "xdma" and (services.get("saturn_bridge") != "active" or services.get("p2app") != "inactive"):
    raise SystemExit(1)
print(selected)
PY
}

selected_owner_receiving(){
  local backend="$1" response status
  status="$(selected_backend_status)" || return 1
  [[ "$status" == "$backend" ]] || return 1
  response="$(curl -fsS --max-time 2 "$P23_PERF_URL")" || return 1
  python3 - "$backend" "$response" <<'PY'
import json
import sys
backend = sys.argv[1]
try:
    root = json.loads(sys.argv[2])
    perf = root["perf"]
    workload = perf["workload"]
    telemetry = perf["app_telemetry"]
    current = telemetry["current"]
    state = current["state"]
    pid = workload["service_main_pid"]
    age = telemetry["age_seconds"]
except (ValueError, TypeError, KeyError):
    raise SystemExit(1)
expected_app = "p2" if backend == "p2" else "saturn-bridge"
if (workload.get("selected_app") != backend or current.get("app") != expected_app
        or not isinstance(pid, int) or pid <= 0 or current.get("pid") != pid
        or telemetry.get("snapshot_readable") is not True
        or telemetry.get("pid_matches_service") is not True
        or not isinstance(age, (int, float)) or not 0 <= age <= 5
        or state.get("sdr_active") is not True
        or state.get("thread_error") is True):
    raise SystemExit(1)
PY
}

wait_for_selected_owner(){
  local backend="$1" elapsed=0
  while (( elapsed < READY_TIMEOUT_SECONDS )); do
    if selected_owner_receiving "$backend"; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  selected_owner_receiving "$backend"
}

validation_enabled(){
  [[ "$VALIDATION_ENABLED" == "1" && "$SELECTED_BACKEND" == "xdma" ]]
}

# Every prerequisite is checked before the first service is touched: an update that cannot prove reception must not start.
validation_preflight(){
  local path mode
  [[ -f "$VALIDATOR" && ! -L "$VALIDATOR" ]] || die "receive validator is not installed: $VALIDATOR (install it with saturn-release-install-activator-root.sh, or set VALIDATION_ENABLED=0)"
  [[ -f "$VALIDATOR_GUARD" && ! -L "$VALIDATOR_GUARD" ]] || die "receive-only guard script is not installed: $VALIDATOR_GUARD"
  if (( EUID == 0 )); then
    for path in "$VALIDATOR" "$VALIDATOR_GUARD"; do
      [[ "$(stat -c '%u' "$path")" == "0" ]] || die "validator component is not root-owned: $path"
      mode="$(stat -c '%a' "$path")"
      (( (8#$mode & 8#022) == 0 )) || die "validator component is group/world writable: $path"
    done
  fi
  python3 -c 'import ast, sys; ast.parse(open(sys.argv[1], encoding="utf-8").read())' "$VALIDATOR" \
    || die "receive validator does not parse: $VALIDATOR"
  [[ -x "$VALIDATION_BROWSER" ]] || die "validation browser is not executable: $VALIDATION_BROWSER"
  [[ -r "$VALIDATION_TLS_CERT" ]] || die "remote proxy certificate is not readable: $VALIDATION_TLS_CERT"
  [[ -r "$VALIDATION_PERF_JSON" ]] || die "the selected owner's telemetry is not readable: $VALIDATION_PERF_JSON"
  if [[ "$VALIDATION_LAUNCHER" == "systemd" ]]; then
    need_cmd systemd-run || return 1
  fi
}

# Start a receive-only session in the background. $1 labels the evidence, $2 is the commit that must be running.
validation_start(){
  local label="$1" commit="$2" stamp dir
  local -a args
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  if (( EUID == 0 )); then
    install -d -m 0750 -o root -g "$TRANSACTION_GROUP" "$VALIDATION_LOG_ROOT" || return 1
  else
    install -d -m 0750 "$VALIDATION_LOG_ROOT" || return 1
  fi
  # Unique for every run: two validations of the same commit in the same second must never share (and read each other's) status files.
  dir="$(mktemp -d "$VALIDATION_LOG_ROOT/$stamp-$label-${commit:0:12}.XXXXXX")" || return 1
  chmod 0750 "$dir" || return 1
  if (( EUID == 0 )); then
    chgrp "$TRANSACTION_GROUP" "$dir" || return 1
  fi
  VALIDATION_CONTROL_DIR="$dir"
  VALIDATION_LABEL="$label"
  args=(
    run --control-dir "$dir" --target-commit "$commit" --backend "$SELECTED_BACKEND"
    --expect-rxc1 "$VALIDATION_EXPECT_RXC1" --viewer-role "$VALIDATION_VIEWER_ROLE" --url "$VALIDATION_URL"
    --ready-url "${SATURN_GO_READY_URL%%\?*}" --perf-url "$P23_PERF_URL" --perf-json "$VALIDATION_PERF_JSON"
    --unit "$BRIDGE_SERVICE" --unit "$SATURN_GO_SERVICE" --owner-unit "$BRIDGE_SERVICE" --auth-unit "$SATURN_GO_SERVICE"
    --release-root "$RELEASES_ROOT" --tls-cert "$VALIDATION_TLS_CERT" --browser "$VALIDATION_BROWSER"
    --launcher "$VALIDATION_LAUNCHER" --memory-max "$VALIDATION_MEMORY_MAX" --guard-script "$VALIDATOR_GUARD"
  )
  (( VALIDATION_NO_SANDBOX )) && args+=(--no-sandbox)
  [[ "$label" != "target" && -z "$PREVIOUS_RELEASE" ]] && args+=(--no-exe-check)
  log "starting receive validation ($label) for $commit; evidence: $dir"
  python3 "$VALIDATOR" "${args[@]}" >"$dir/validator.out" 2>&1 &
  VALIDATOR_PID=$!
}

# Block until the session has proven reception, bounded. The session then stays attached until validation_stop_record.
validation_wait_ready(){
  local output rc=0 out_file="$VALIDATION_CONTROL_DIR/wait.out"
  # Run in the background and use the `wait` builtin: bash defers a trapped signal until a foreground command ends, and this one may take
  # minutes. An interrupted helper must end the session at once.
  python3 "$VALIDATOR" wait --control-dir "$VALIDATION_CONTROL_DIR" --timeout "$VALIDATION_TIMEOUT_SECONDS" >"$out_file" 2>&1 &
  VALIDATION_WAITER_PID=$!
  wait "$VALIDATION_WAITER_PID" || rc=$?
  VALIDATION_WAITER_PID=""
  output="$(cat "$out_file" 2>/dev/null || true)"
  if (( rc == 0 )); then
    log "receive validation ($VALIDATION_LABEL) established reception: $output"
    return 0
  fi
  log "receive validation ($VALIDATION_LABEL) did not establish reception: $output"
  return 1
}

# End the session, wait for its cleanup, and keep what it found in the transaction record. Succeeds only if the validator finished cleanly.
validation_stop_record(){
  local rc=0 output
  [[ -n "$VALIDATION_CONTROL_DIR" ]] || return 0
  output="$(python3 "$VALIDATOR" stop --control-dir "$VALIDATION_CONTROL_DIR" --timeout 60)" || rc=$?
  if [[ -n "$VALIDATOR_PID" ]]; then
    wait "$VALIDATOR_PID" 2>/dev/null || true
  fi
  VALIDATION_RECORD_JSON="$(python3 - "$VALIDATION_RECORD_JSON" "$VALIDATION_LABEL" "$VALIDATION_CONTROL_DIR" <<'PY'
import json
import sys
record = json.loads(sys.argv[1])
label, directory = sys.argv[2], sys.argv[3]
try:
    with open(f"{directory}/status.json", encoding="utf-8") as handle:
        status = json.load(handle)
except (OSError, ValueError):
    status = {"state": "missing"}
record[label] = {
    "control_directory": directory,
    "final_state": status.get("state"),
    "exit_code": status.get("exit_code"),
    "driver": status.get("driver"),
    "failed": status.get("failed"),
    "role": status.get("role"),
    "role_enforced_by_bridge": status.get("role_enforced_by_bridge"),
    "conditions": {key: value.get("ok") for key, value in sorted((status.get("conditions") or {}).items())},
    "started_at": status.get("started_at"),
    "ready_at": status.get("ready_at"),
    "finished_at": status.get("finished_at"),
}
print(json.dumps(record, sort_keys=True))
PY
)" || true
  log "receive validation ($VALIDATION_LABEL) finished: $output"
  VALIDATION_CONTROL_DIR=""
  VALIDATOR_PID=""
  return "$rc"
}

# Last resort for an interrupted helper: never leave a browser session attached.
validation_abort(){
  if [[ -n "$VALIDATION_WAITER_PID" ]]; then
    kill "$VALIDATION_WAITER_PID" 2>/dev/null || true
    VALIDATION_WAITER_PID=""
  fi
  [[ -n "$VALIDATION_CONTROL_DIR" ]] || return 0
  python3 "$VALIDATOR" stop --control-dir "$VALIDATION_CONTROL_DIR" --timeout 60 >/dev/null 2>&1 || true
  if [[ -n "$VALIDATOR_PID" ]]; then
    wait "$VALIDATOR_PID" 2>/dev/null || true
  fi
  VALIDATION_CONTROL_DIR=""
  VALIDATOR_PID=""
}

# Reception must be proven with the validation session attached: the readiness gate counts that session as the receive client.
prove_reception(){
  local label="$1" commit="$2" rc=1
  if validation_enabled; then
    if validation_start "$label" "$commit" && validation_wait_ready && wait_for_selected_owner "$SELECTED_BACKEND"; then
      rc=0
    fi
    validation_stop_record || rc=1
  else
    wait_for_selected_owner "$SELECTED_BACKEND" && rc=0
  fi
  return "$rc"
}

restore_dropin(){
  local destination="$1" existed="$2" backup="$3"
  if [[ "$existed" == "1" ]]; then
    atomic_install_file "$backup" "$destination"
  else
    if [[ -e "$destination" && ! -f "$destination" ]]; then
      printf '[saturn-release-activate] ERROR: generated service drop-in became unsafe: %s\n' "$destination" >&2
      return 1
    fi
    rm -f -- "$destination" || return 1
  fi
}

verify_restored_dropin(){
  local destination="$1" existed="$2" backup="$3"
  if [[ "$existed" == "1" ]]; then
    [[ -f "$destination" && ! -L "$destination" ]] || return 1
    cmp -s "$backup" "$destination"
  else
    [[ ! -e "$destination" && ! -L "$destination" ]]
  fi
}

state_tool_release_args(){
  STATE_TOOL_ARGS=(--state-root "$STATE_ROOT" --target-release "$TARGET_RELEASE")
  if [[ -n "$PREVIOUS_RELEASE" ]]; then
    STATE_TOOL_ARGS+=(--previous-release "$PREVIOUS_RELEASE")
  fi
  if (( APPROVE_ONE_WAY_MIGRATION )); then
    STATE_TOOL_ARGS+=(--approve-one-way)
  fi
}

preflight_state_compatibility(){
  state_tool_release_args
  STATE_PLAN_JSON="$("$STATE_TOOL" preflight "${STATE_TOOL_ARGS[@]}")"
  python3 - "$STATE_PLAN_JSON" <<'PY'
import json
import sys
value = json.loads(sys.argv[1])
required = {
    "current_state_schema_version",
    "target_state_schema_version",
    "migration_required",
    "rollback_safe",
    "managed_paths",
}
if not isinstance(value, dict) or not required.issubset(value):
    raise SystemExit(1)
PY
}

migrate_state_if_required(){
  local result parsed
  state_tool_release_args
  # Keep the command substitution in a tested conditional. With errtrace on,
  # allowing the substitution itself to trigger ERR would invoke automatic
  # rollback once in the subshell and again when this function returned.
  if ! result="$("$STATE_TOOL" migrate "${STATE_TOOL_ARGS[@]}" \
    --backup-root "$STATE_BACKUP_ROOT" \
    --target-commit "$TARGET_COMMIT" \
    --state-group "$TRANSACTION_GROUP")"; then
    return 1
  fi
  parsed="$(python3 - "$result" <<'PY'
import json
import sys
value = json.loads(sys.argv[1])
print(f'{1 if value.get("migrated") is True else 0}\t{value.get("backup_directory") or ""}')
PY
  )" || return 1
  IFS=$'\t' read -r STATE_MIGRATED STATE_BACKUP_DIR <<<"$parsed"
  [[ "$STATE_MIGRATED" == "0" || "$STATE_MIGRATED" == "1" ]] \
    || die "state migration tool returned an invalid result"
}

restore_previous_state(){
  [[ "$STATE_MIGRATED" == "1" ]] || return 0
  [[ -n "$STATE_BACKUP_DIR" ]] || return 1
  "$STATE_TOOL" restore \
    --state-root "$STATE_ROOT" \
    --backup-root "$STATE_BACKUP_ROOT" \
    --backup-directory "$STATE_BACKUP_DIR" >/dev/null
}

stop_affected_services(){
  local rc=0
  systemctl stop "$SATURN_GO_SERVICE" || rc=1
  systemctl stop "$BRIDGE_SERVICE" || rc=1
  systemctl stop "$P2APP_SERVICE" || rc=1
  return "$rc"
}

restore_previous_services(){
  local rc=0
  if [[ "$P2APP_WAS_ACTIVE" == "1" ]]; then
    systemctl start "$P2APP_SERVICE" || rc=1
    systemctl is-active --quiet "$P2APP_SERVICE" || rc=1
  fi
  if [[ "$BRIDGE_WAS_ACTIVE" == "1" ]]; then
    systemctl start "$BRIDGE_SERVICE" || rc=1
    systemctl is-active --quiet "$BRIDGE_SERVICE" || rc=1
  fi
  if [[ "$SATURN_GO_WAS_ACTIVE" == "1" ]]; then
    systemctl start "$SATURN_GO_SERVICE" || rc=1
    systemctl is-active --quiet "$SATURN_GO_SERVICE" || rc=1
  fi
  return "$rc"
}

rollback_activation(){
  local rc=0
  set +e
  ROLLBACK_STATUS="running"
  ROLLBACK_MESSAGE="Restoring the prior release and service configuration"
  write_transaction "rolling_back" "rollback" "$ROLLBACK_MESSAGE" || rc=1
  stop_affected_services || rc=1
  restore_previous_state || rc=1

  if [[ -n "$PREVIOUS_RELEASE" ]]; then
    atomic_switch_pointer "$PREVIOUS_RELEASE" || rc=1
    [[ "$(realpath -e "$CURRENT_LINK" 2>/dev/null)" == "$PREVIOUS_RELEASE" ]] || rc=1
  else
    remove_current_pointer || rc=1
    [[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]] || rc=1
  fi
  restore_dropin "$SATURN_GO_DROPIN" "$SATURN_GO_DROPIN_EXISTED" \
    "$ROLLBACK_DIR/$SATURN_GO_SERVICE.conf" || rc=1
  restore_dropin "$BRIDGE_DROPIN" "$BRIDGE_DROPIN_EXISTED" \
    "$ROLLBACK_DIR/$BRIDGE_SERVICE.conf" || rc=1
  restore_dropin "$P2APP_DROPIN" "$P2APP_DROPIN_EXISTED" \
    "$ROLLBACK_DIR/$P2APP_SERVICE.conf" || rc=1
  verify_restored_dropin "$SATURN_GO_DROPIN" "$SATURN_GO_DROPIN_EXISTED" \
    "$ROLLBACK_DIR/$SATURN_GO_SERVICE.conf" || rc=1
  verify_restored_dropin "$BRIDGE_DROPIN" "$BRIDGE_DROPIN_EXISTED" \
    "$ROLLBACK_DIR/$BRIDGE_SERVICE.conf" || rc=1
  verify_restored_dropin "$P2APP_DROPIN" "$P2APP_DROPIN_EXISTED" \
    "$ROLLBACK_DIR/$P2APP_SERVICE.conf" || rc=1
  systemctl daemon-reload || rc=1
  restore_previous_services || rc=1
  if [[ "$SATURN_GO_WAS_ACTIVE" == "1" ]]; then
    wait_for_commit "$PREVIOUS_READY_COMMIT" || rc=1
  fi

  # "Release restored" (pointer, drop-ins, services and the running commit) and "receive verified" are separate facts. A restored release
  # whose reception could not be proven is reported as exactly that, not as a failed restoration.
  if (( rc == 0 )); then
    ROLLBACK_RELEASE_RESTORED="true"
    if [[ "$SATURN_GO_WAS_ACTIVE" == "1" ]]; then
      if prove_reception rollback "$PREVIOUS_READY_COMMIT"; then
        ROLLBACK_RECEIVE_VERIFIED="true"
      else
        ROLLBACK_RECEIVE_VERIFIED="false"
        ROLLBACK_RECEIVE_UNVERIFIED=1
      fi
    else
      ROLLBACK_RECEIVE_VERIFIED="not_attempted"
    fi
    ROLLBACK_STATUS="succeeded"
    if [[ "$ROLLBACK_RECEIVE_VERIFIED" == "false" ]]; then
      ROLLBACK_MESSAGE="Previous release and services restored after activation failure; reception on the restored release was NOT verified"
    else
      ROLLBACK_MESSAGE="Previous release and services restored after activation failure"
    fi
    write_transaction "rolled_back" "rollback-complete" "$ROLLBACK_MESSAGE" || rc=1
  else
    ROLLBACK_RELEASE_RESTORED="false"
    ROLLBACK_RECEIVE_VERIFIED="not_attempted"
  fi
  if (( rc != 0 )); then
    ROLLBACK_STATUS="failed"
    ROLLBACK_MESSAGE="Automatic rollback did not fully restore the previous release"
    write_transaction "rollback_failed" "rollback-failed" "$ROLLBACK_MESSAGE" || true
  fi
  set -e
  return "$rc"
}

# Modes that attach a validation session without an activation transaction still must not leave it attached if they are interrupted.
install_interrupt_traps(){
  trap validation_abort EXIT
  trap 'exit 143' TERM
  trap 'exit 130' INT
  trap 'exit 129' HUP
}

# Prove reception on the release that is running now, with the launcher an activation uses and nothing else: no pointer, drop-in, service
# or transaction record is touched. It is how a host's browser sandbox, memory cap and credentials are tried out before an update.
check_reception(){
  local running identity
  [[ "$VALIDATION_ENABLED" == "1" ]] || die "receive validation is disabled by root-owned policy (VALIDATION_ENABLED)"
  SELECTED_BACKEND="$(selected_backend_status)" || die "no healthy selected radio owner"
  validation_enabled || die "receive validation supports the direct-XDMA owner only; the selected backend is $SELECTED_BACKEND"
  validation_preflight
  running="$(probe_running_commit)" || die "Saturn Go is not ready"
  identity="$(current_release_identity)" || die "cannot resolve the current release"
  IFS=$'\t' read -r PREVIOUS_COMMIT PREVIOUS_RELEASE <<<"$identity"
  install_interrupt_traps
  log "checking reception on the running release $running (nothing is changed)"
  if prove_reception check "$running"; then
    log "reception verified on $running; evidence is under $VALIDATION_LOG_ROOT"
  else
    die "reception could not be proven on $running; see the evidence under $VALIDATION_LOG_ROOT"
  fi
}

# Clear a "rollback_failed" record the supported way: only if the recorded previous release is verifiably the one running, wired as it was,
# healthy, and receiving, proven with the same validation an activation uses. Anything else leaves the record exactly as it is.
reconcile_rollback(){
  local fields identity cur_commit cur_release running selected backup stamp service
  [[ "$ACTIVATION_ENABLED" == "1" ]] \
    || die "reconciliation is disabled by root-owned policy (ACTIVATION_ENABLED)"
  [[ "$(transaction_status)" == "rollback_failed" ]] \
    || die "nothing to reconcile: the deployment transaction status is '$(transaction_status)', not rollback_failed"
  prepare_transaction_directory
  fields="$(python3 - "$TRANSACTION_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
services = value["services"]
active = services["previously_active"]
names = services["stop_order"]  # saturn-go, bridge, p2app
dropins = value["service_dropins"]
def flag(x):
    return "1" if x else "0"
state = value.get("state_compatibility") or {}
# A non-whitespace delimiter keeps empty fields (a legacy deployment has no previous commit or release); the JSON goes last.
print("|".join([
    value.get("previous_commit") or "", value.get("previous_release") or "", value["previous_ready_commit"],
    services["selected_backend"], value["target_commit"], value["target_release"], value["rollback_directory"],
    flag(active[names[0]]), flag(active[names[1]]), flag(active[names[2]]),
    flag(dropins[names[0]]["previously_existed"]), flag(dropins[names[1]]["previously_existed"]), flag(dropins[names[2]]["previously_existed"]),
    state.get("backup_directory") or "", flag(state.get("migrated")), flag(state.get("one_way_operator_approval")),
    json.dumps(state.get("plan") or {}),
]))
PY
)" || die "the deployment record cannot be read"
  IFS='|' read -r PREVIOUS_COMMIT PREVIOUS_RELEASE PREVIOUS_READY_COMMIT SELECTED_BACKEND TARGET_COMMIT TARGET_RELEASE ROLLBACK_DIR \
    SATURN_GO_WAS_ACTIVE BRIDGE_WAS_ACTIVE P2APP_WAS_ACTIVE \
    SATURN_GO_DROPIN_EXISTED BRIDGE_DROPIN_EXISTED P2APP_DROPIN_EXISTED \
    STATE_BACKUP_DIR STATE_MIGRATED APPROVE_ONE_WAY_MIGRATION STATE_PLAN_JSON <<<"$fields"
  SATURN_GO_DROPIN="$SYSTEMD_ROOT/$SATURN_GO_SERVICE.d/50-saturn-release.conf"
  BRIDGE_DROPIN="$SYSTEMD_ROOT/$BRIDGE_SERVICE.d/50-saturn-release.conf"
  P2APP_DROPIN="$SYSTEMD_ROOT/$P2APP_SERVICE.d/50-saturn-release.conf"
  log "reconciling the failed rollback of $TARGET_COMMIT: the previous release ${PREVIOUS_COMMIT:-<legacy deployment>} must be running and receiving"

  identity="$(current_release_identity)" || die "cannot resolve the current release"
  IFS=$'\t' read -r cur_commit cur_release <<<"$identity"
  [[ "$cur_commit" == "$PREVIOUS_COMMIT" && "$cur_release" == "$PREVIOUS_RELEASE" ]] \
    || die "the active release (${cur_commit:-none}) is not the recorded previous release (${PREVIOUS_COMMIT:-none}); the record is left unchanged"
  if [[ -n "$PREVIOUS_COMMIT" ]]; then
    validate_release "$PREVIOUS_COMMIT" >/dev/null || die "the previous release failed validation; the record is left unchanged"
  fi
  for service in "$SATURN_GO_SERVICE:$SATURN_GO_WAS_ACTIVE" "$BRIDGE_SERVICE:$BRIDGE_WAS_ACTIVE" "$P2APP_SERVICE:$P2APP_WAS_ACTIVE"; do
    if [[ "${service#*:}" == "1" ]]; then
      systemctl is-active --quiet "${service%%:*}" || die "${service%%:*} was active before the activation and is not now; the record is left unchanged"
    else
      ! systemctl is-active --quiet "${service%%:*}" || die "${service%%:*} was inactive before the activation and is active now; the record is left unchanged"
    fi
  done
  verify_restored_dropin "$SATURN_GO_DROPIN" "$SATURN_GO_DROPIN_EXISTED" "$ROLLBACK_DIR/$SATURN_GO_SERVICE.conf" \
    || die "the $SATURN_GO_SERVICE drop-in differs from its rollback backup; the record is left unchanged"
  verify_restored_dropin "$BRIDGE_DROPIN" "$BRIDGE_DROPIN_EXISTED" "$ROLLBACK_DIR/$BRIDGE_SERVICE.conf" \
    || die "the $BRIDGE_SERVICE drop-in differs from its rollback backup; the record is left unchanged"
  verify_restored_dropin "$P2APP_DROPIN" "$P2APP_DROPIN_EXISTED" "$ROLLBACK_DIR/$P2APP_SERVICE.conf" \
    || die "the $P2APP_SERVICE drop-in differs from its rollback backup; the record is left unchanged"
  running="$(probe_running_commit)" || die "Saturn Go is not ready; the record is left unchanged"
  [[ "$running" == "$PREVIOUS_READY_COMMIT" ]] \
    || die "the running commit $running is not the recorded previous commit $PREVIOUS_READY_COMMIT; the record is left unchanged"
  selected="$(selected_backend_status)" || die "no healthy selected radio owner; the record is left unchanged"
  [[ "$selected" == "$SELECTED_BACKEND" ]] || die "the selected backend is $selected, the record says $SELECTED_BACKEND; the record is left unchanged"

  if validation_enabled; then
    validation_preflight
  fi
  install_interrupt_traps
  if ! prove_reception reconcile "$PREVIOUS_READY_COMMIT"; then
    die "reception on the previous release could not be proven; the record is left unchanged (see ${VALIDATION_LOG_ROOT})"
  fi

  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  backup="$TRANSACTION_FILE.rollback-failed.$stamp"
  cp -a -- "$TRANSACTION_FILE" "$backup" || die "cannot preserve the original record"
  RECONCILED_FROM="$backup"
  ROLLBACK_RELEASE_RESTORED="true"
  ROLLBACK_RECEIVE_VERIFIED="true"
  ROLLBACK_STATUS="succeeded"
  ROLLBACK_MESSAGE="Reconciled: the previous release is running, wired as before the activation, and verified receiving"
  write_transaction "rolled_back" "reconciled" "$ROLLBACK_MESSAGE" || die "cannot write the reconciled record"
  log "reconciled: the record is now rolled_back; the original is preserved at $backup"
}

mark_failed(){
  local activation_rc="$?" command="${BASH_COMMAND:-unknown command}" rollback_rc=0
  trap - ERR
  if [[ -n "$TEMP_POINTER" && -L "$TEMP_POINTER" ]]; then
    rm -f -- "$TEMP_POINTER"
  fi
  if (( TRANSACTION_PREPARED )); then
    ACTIVATION_FAILURE_PHASE="$PHASE"
    ACTIVATION_FAILURE_COMMAND="$command"
    ACTIVATION_FAILURE_STATUS="$activation_rc"
    # A validation session still attached when the activation failed is ended first, and what it found is kept as evidence.
    if [[ -n "$VALIDATION_CONTROL_DIR" ]]; then
      validation_stop_record || true
    fi
    rollback_activation || rollback_rc=$?
  fi
  if (( rollback_rc == 0 )); then
    if (( ROLLBACK_RECEIVE_UNVERIFIED )); then
      log "activation failed during $PHASE; the previous release was restored but reception on it was NOT verified"
      exit 71
    fi
    log "activation failed during $PHASE; automatic rollback succeeded"
    exit "$activation_rc"
  fi
  log "activation failed during $PHASE and automatic rollback was incomplete"
  exit 70
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi

while [[ "${1:-}" == --* ]]; do
  case "$1" in
    --validate)
      VALIDATE_ONLY=1
      ;;
    --approve-one-way-migration)
      APPROVE_ONE_WAY_MIGRATION=1
      ;;
    --reconcile-rollback)
      RECONCILE_ROLLBACK=1
      ;;
    --check-reception)
      CHECK_RECEPTION=1
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  shift
done
if (( RECONCILE_ROLLBACK || CHECK_RECEPTION )); then
  [[ $# -eq 0 ]] || { usage >&2; exit 2; }
  (( ! VALIDATE_ONLY && ! APPROVE_ONE_WAY_MIGRATION && ! (RECONCILE_ROLLBACK && CHECK_RECEPTION) )) \
    || die "--reconcile-rollback and --check-reception cannot be combined with other options"
else
  [[ $# -eq 1 ]] || { usage >&2; exit 2; }
  if (( VALIDATE_ONLY && APPROVE_ONE_WAY_MIGRATION )); then
    die "--validate cannot be combined with --approve-one-way-migration"
  fi
  TARGET_COMMIT="$1"
fi

need_cmd curl
need_cmd cmp
need_cmd cut
need_cmd find
need_cmd flock
need_cmd getent
need_cmd install
need_cmd python3
need_cmd realpath
need_cmd systemctl
need_cmd sync
load_config
validate_configuration
install -d -m 0755 "$(dirname "$LOCK_FILE")"
exec 9>"$LOCK_FILE"
flock -n 9 || die "another release activation is already running"
[[ -x "$MANIFEST_TOOL" ]] || die "trusted manifest validator not executable: $MANIFEST_TOOL"
[[ -x "$STATE_TOOL" ]] || die "trusted state compatibility tool not executable: $STATE_TOOL"
[[ -f "$COMPONENTS_FILE" ]] || die "trusted component policy missing: $COMPONENTS_FILE"
if (( RECONCILE_ROLLBACK )); then
  reconcile_rollback
  exit 0
fi
if (( CHECK_RECEPTION )); then
  check_reception
  exit 0
fi
TARGET_RELEASE="$(validate_release "$TARGET_COMMIT")" || die "target release validation failed"
if (( VALIDATE_ONLY )); then
  log "installed release validation passed: $TARGET_RELEASE"
  exit 0
fi

[[ "$ACTIVATION_ENABLED" == "1" ]] \
  || die "activation is disabled by root-owned policy; enable only for an approved appliance rollback test"
existing_status="$(transaction_status)"
case "$existing_status" in
  ""|committed|rolled_back) ;;
  *) die "unresolved deployment transaction has status '$existing_status': $TRANSACTION_FILE" ;;
esac
previous_identity="$(current_release_identity)" || die "current release identity failed"
IFS=$'\t' read -r PREVIOUS_COMMIT PREVIOUS_RELEASE <<<"$previous_identity"
[[ "$PREVIOUS_COMMIT" != "$TARGET_COMMIT" ]] || die "release is already active: $TARGET_COMMIT"
if systemctl is-active --quiet "$SATURN_GO_SERVICE"; then SATURN_GO_WAS_ACTIVE=1; fi
if systemctl is-active --quiet "$BRIDGE_SERVICE"; then BRIDGE_WAS_ACTIVE=1; fi
if systemctl is-active --quiet "$P2APP_SERVICE"; then P2APP_WAS_ACTIVE=1; fi
SELECTED_BACKEND="$(selected_backend_status)" \
  || die "cannot activate without one healthy selected radio owner"
if [[ "$SELECTED_BACKEND" == "p2" ]]; then
  [[ "$P2APP_WAS_ACTIVE" == "1" ]] \
    || die "P2 selection disagrees with active services"
else
  [[ "$BRIDGE_WAS_ACTIVE" == "1" && "$P2APP_WAS_ACTIVE" == "0" ]] \
    || die "direct-XDMA selection disagrees with active services"
fi
[[ "$SATURN_GO_WAS_ACTIVE" == "1" ]] \
  || die "cannot activate from an unhealthy baseline: $SATURN_GO_SERVICE is not active"
if validation_enabled; then
  validation_preflight
elif [[ "$VALIDATION_ENABLED" == "1" ]]; then
  log "receive validation supports the direct-XDMA owner only; $SELECTED_BACKEND keeps the readiness gate that needs an attached receive client"
fi
PREVIOUS_READY_COMMIT="$(probe_running_commit)" \
  || die "cannot resolve the currently ready Saturn Go commit"
if [[ -n "$PREVIOUS_COMMIT" ]]; then
  [[ "$PREVIOUS_READY_COMMIT" == "$PREVIOUS_COMMIT" ]] \
    || die "active pointer and running commit disagree: $PREVIOUS_COMMIT != $PREVIOUS_READY_COMMIT"
  validated_previous="$(validate_release "$PREVIOUS_COMMIT")" \
    || die "previous active release failed validation"
  [[ "$validated_previous" == "$PREVIOUS_RELEASE" ]] \
    || die "previous active release failed validation"
fi
preflight_state_compatibility
prepare_transaction_directory
SATURN_GO_DROPIN="$SYSTEMD_ROOT/$SATURN_GO_SERVICE.d/50-saturn-release.conf"
BRIDGE_DROPIN="$SYSTEMD_ROOT/$BRIDGE_SERVICE.d/50-saturn-release.conf"
P2APP_DROPIN="$SYSTEMD_ROOT/$P2APP_SERVICE.d/50-saturn-release.conf"
[[ -e "$SATURN_GO_DROPIN" || -L "$SATURN_GO_DROPIN" ]] && SATURN_GO_DROPIN_EXISTED=1
[[ -e "$BRIDGE_DROPIN" || -L "$BRIDGE_DROPIN" ]] && BRIDGE_DROPIN_EXISTED=1
[[ -e "$P2APP_DROPIN" || -L "$P2APP_DROPIN" ]] && P2APP_DROPIN_EXISTED=1
prepare_rollback_snapshot

PHASE="prepare"
write_transaction "prepared" "$PHASE" "Validated target and persisted deployment intent" prepare
TRANSACTION_PREPARED=1
# A helper that is interrupted must not leave a validation browser session attached to the radio.
trap validation_abort EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
trap 'exit 129' HUP
trap mark_failed ERR

PHASE="service-stop"
write_transaction "activating" "$PHASE" "Stopping affected services in dependency order"
systemctl stop "$SATURN_GO_SERVICE"
systemctl stop "$BRIDGE_SERVICE"
systemctl stop "$P2APP_SERVICE"

PHASE="state-migration"
write_transaction "activating" "$PHASE" "Backing up and migrating persistent state when required"
migrate_state_if_required
write_transaction "activating" "$PHASE" "Persistent state compatibility is ready for the target"

PHASE="service-wiring"
write_transaction "activating" "$PHASE" "Installing stable-pointer systemd overrides"
install_release_dropins
systemctl daemon-reload

PHASE="pointer-switch"
write_transaction "activating" "$PHASE" "Atomically switching the active release pointer"
atomic_switch_pointer "$TARGET_RELEASE"

PHASE="service-start"
write_transaction "activating" "$PHASE" "Starting affected services in dependency order"
if [[ "$SELECTED_BACKEND" == "p2" ]]; then
  systemctl start "$P2APP_SERVICE"
  systemctl is-active --quiet "$P2APP_SERVICE"
  if [[ "$BRIDGE_WAS_ACTIVE" == "1" ]]; then
    systemctl start "$BRIDGE_SERVICE"
    systemctl is-active --quiet "$BRIDGE_SERVICE"
  else
    if systemctl is-active --quiet "$BRIDGE_SERVICE"; then
      die "P2-only activation unexpectedly started the Bridge"
    fi
  fi
else
  systemctl start "$BRIDGE_SERVICE"
  systemctl is-active --quiet "$BRIDGE_SERVICE"
  if systemctl is-active --quiet "$P2APP_SERVICE"; then
    die "direct-XDMA activation unexpectedly started P2"
  fi
fi
systemctl start "$SATURN_GO_SERVICE"
systemctl is-active --quiet "$SATURN_GO_SERVICE"

PHASE="readiness"
write_transaction "verifying" "$PHASE" "Waiting for target-aware readiness"
if ! wait_for_commit "$TARGET_COMMIT"; then
  false
fi
if ! probe_web_pages; then
  log "activated Saturn Go cannot serve Radio Telemetry and Settings"
  false
fi
if validation_enabled; then
  PHASE="receive-validation"
  write_transaction "verifying" "$PHASE" "Proving reception with a temporary receive-only browser session"
  if ! validation_start target "$TARGET_COMMIT"; then
    validation_stop_record || true
    false
  fi
  if ! validation_wait_ready; then
    validation_stop_record || true
    false
  fi
  # The session stays attached while the helper's own readiness gate runs: it is what makes the owner report an active receive client.
  if ! wait_for_selected_owner "$SELECTED_BACKEND"; then
    validation_stop_record || true
    false
  fi
  if ! validation_stop_record; then
    log "the receive validation session did not end cleanly"
    false
  fi
else
  if ! wait_for_selected_owner "$SELECTED_BACKEND"; then
    false
  fi
fi

PHASE="commit"
write_transaction "committed" "$PHASE" "Target release is active and ready"
trap - ERR
log "activated and verified release: $TARGET_COMMIT"
