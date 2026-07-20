#!/usr/bin/env bash
set -Eeuo pipefail

# Atomically select one already-installed Saturn application release and
# restart only the services that consume the versioned application payload.
#
# Production activation is intentionally disabled by default until REM-0204
# automatic rollback is implemented and appliance-tested. Validation remains
# available while activation is disabled.

CONFIG_FILE="${SATURN_RELEASE_ACTIVATE_CONFIG:-/etc/default/saturn-release-activate}"
VALIDATE_ONLY=0
ACTIVATION_ENABLED=0
SATURN_ROOT="/opt/saturn"
RELEASES_ROOT="/opt/saturn/releases"
CURRENT_LINK="/opt/saturn/current"
TRANSACTION_FILE="/var/lib/saturn-state/deployments/current.json"
LOCK_FILE="/run/lock/saturn-release-activate.lock"
MANIFEST_TOOL="/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py"
COMPONENTS_FILE="/usr/local/lib/saturn-go/release/components-v1.json"
SYSTEMD_ROOT="/etc/systemd/system"
SATURN_GO_SERVICE="saturn-go.service"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
SATURN_GO_READY_URL="http://127.0.0.1:8080/readyz"
READY_TIMEOUT_SECONDS=30
P2APP_PANEL_ENABLED=0
TRANSACTION_GROUP="pi"
TRANSACTION_GID=""
PHASE="preflight"
TARGET_COMMIT=""
TARGET_RELEASE=""
PREVIOUS_COMMIT=""
PREVIOUS_RELEASE=""
TRANSACTION_PREPARED=0
TEMP_POINTER=""
SATURN_GO_DROPIN=""
BRIDGE_DROPIN=""
P2APP_DROPIN=""
SATURN_GO_DROPIN_EXISTED=0
BRIDGE_DROPIN_EXISTED=0
P2APP_DROPIN_EXISTED=0

log(){ printf '[saturn-release-activate] %s\n' "$*"; }
die(){ printf '[saturn-release-activate] ERROR: %s\n' "$*" >&2; return 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage:
  saturn-release-activate-root.sh <full-commit>
  saturn-release-activate-root.sh --validate <full-commit>

The validation form verifies an installed immutable release without changing
the host. Activation requires ACTIVATION_ENABLED=1 in the root-owned config.
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
      SYSTEMD_ROOT) SYSTEMD_ROOT="$value" ;;
      SATURN_GO_SERVICE) SATURN_GO_SERVICE="$value" ;;
      BRIDGE_SERVICE) BRIDGE_SERVICE="$value" ;;
      P2APP_SERVICE) P2APP_SERVICE="$value" ;;
      SATURN_GO_READY_URL) SATURN_GO_READY_URL="$value" ;;
      READY_TIMEOUT_SECONDS) READY_TIMEOUT_SECONDS="$value" ;;
      P2APP_PANEL_ENABLED) P2APP_PANEL_ENABLED="$value" ;;
      TRANSACTION_GROUP) TRANSACTION_GROUP="$value" ;;
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
  for path in \
    "$SATURN_ROOT" "$RELEASES_ROOT" "$CURRENT_LINK" "$TRANSACTION_FILE" "$LOCK_FILE" \
    "$MANIFEST_TOOL" "$COMPONENTS_FILE" "$SYSTEMD_ROOT"
  do
    [[ "$path" == /* && "$path" != *[$'\t\r\n ']* ]] || die "unsafe configured path: $path"
  done
  [[ "$(dirname "$RELEASES_ROOT")" == "$SATURN_ROOT" ]] \
    || die "RELEASES_ROOT must be directly beneath SATURN_ROOT"
  [[ "$(dirname "$CURRENT_LINK")" == "$SATURN_ROOT" ]] \
    || die "CURRENT_LINK must be directly beneath SATURN_ROOT"
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
  local commit="$1" release canonical_saturn canonical_root canonical_release actual architecture bad
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "target must be one lowercase full Git commit"
  [[ -d "$SATURN_ROOT" && ! -L "$SATURN_ROOT" ]] \
    || die "Saturn application root is not a real directory: $SATURN_ROOT"
  [[ -d "$RELEASES_ROOT" && ! -L "$RELEASES_ROOT" ]] \
    || die "release root is not a real directory: $RELEASES_ROOT"
  canonical_saturn="$(realpath -e "$SATURN_ROOT")"
  canonical_root="$(realpath -e "$RELEASES_ROOT")"
  [[ "$(dirname "$canonical_root")" == "$canonical_saturn" ]] \
    || die "release root is not directly beneath the Saturn application root: $canonical_root"
  release="$RELEASES_ROOT/$commit"
  [[ -d "$release" && ! -L "$release" ]] || die "installed release not found: $release"
  canonical_release="$(realpath -e "$release")"
  [[ "$(dirname "$canonical_release")" == "$canonical_root" ]] \
    || die "release is not a direct child of the trusted release root: $canonical_release"
  bad="$(find "$canonical_release" -xdev -type l -print -quit)"
  [[ -z "$bad" ]] || die "symbolic links are not permitted inside a release: $bad"
  bad="$(find "$canonical_release" -xdev ! -type d ! -type f -print -quit)"
  [[ -z "$bad" ]] || die "non-regular release entry rejected: $bad"
  bad="$(find "$canonical_release" -xdev -perm /022 -print -quit)"
  [[ -z "$bad" ]] || die "group/world-writable release entry rejected: $bad"
  bad="$(find "$canonical_release" -xdev -type d ! -perm 0755 -print -quit)"
  [[ -z "$bad" ]] || die "release directory must use mode 0755: $bad"
  if (( EUID == 0 )); then
    bad="$(find "$canonical_release" -xdev \( ! -user root -o ! -group root \) -print -quit)"
    [[ -z "$bad" ]] || die "installed release ownership mismatch: $bad"
  fi
  python3 "$MANIFEST_TOOL" validate \
    --release-root "$canonical_release" \
    --components "$COMPONENTS_FILE" >/dev/null
  IFS=$'\t' read -r actual architecture < <(release_identity "$canonical_release")
  [[ "$actual" == "$commit" ]] || die "installed release commit mismatch: $actual"
  [[ "$architecture" == "$(uname -m)" ]] \
    || die "installed release architecture $architecture does not match host $(uname -m)"
  printf '%s\n' "$canonical_release"
}

current_release_identity(){
  local canonical_root resolved leaf
  if [[ ! -e "$CURRENT_LINK" && ! -L "$CURRENT_LINK" ]]; then
    printf '\t\n'
    return 0
  fi
  [[ -L "$CURRENT_LINK" ]] || die "current release pointer is not a symbolic link: $CURRENT_LINK"
  canonical_root="$(realpath -e "$RELEASES_ROOT")"
  resolved="$(realpath -e "$CURRENT_LINK")"
  [[ "$(dirname "$resolved")" == "$canonical_root" ]] \
    || die "current release points outside the trusted release root: $resolved"
  leaf="$(basename "$resolved")"
  [[ "$leaf" =~ ^[0-9a-f]{40}$ ]] || die "current release target is not a full commit: $resolved"
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
    "$P2APP_DROPIN" "$P2APP_DROPIN_EXISTED" "$TRANSACTION_GID" <<'PY'
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
        "current_link": current_link,
        "services": {
            "stop_order": [saturn_go, bridge, p2app],
            "start_order": [p2app, bridge, saturn_go],
        },
        "service_dropins": {
            saturn_go: {
                "path": saturn_go_dropin,
                "previously_existed": saturn_go_dropin_existed == "1",
            },
            bridge: {
                "path": bridge_dropin,
                "previously_existed": bridge_dropin_existed == "1",
            },
            p2app: {
                "path": p2app_dropin,
                "previously_existed": p2app_dropin_existed == "1",
            },
        },
    }
value.update({
    "status": status,
    "phase": phase,
    "message": message,
    "updated_at": now,
})
fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
try:
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    os.chmod(temporary, 0o640)
    if os.geteuid() == 0:
        os.chown(temporary, 0, int(transaction_gid))
    os.replace(temporary, path)
    directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
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

switch_current_pointer(){
  local parent attempt
  parent="$(dirname "$CURRENT_LINK")"
  [[ "$(stat -c '%d' "$parent")" == "$(stat -c '%d' "$TARGET_RELEASE")" ]] \
    || die "current pointer and target release must be on the same filesystem"
  for attempt in {1..20}; do
    TEMP_POINTER="$parent/.current.$$.${RANDOM}.${attempt}"
    if ln -s "$TARGET_RELEASE" "$TEMP_POINTER" 2>/dev/null; then
      break
    fi
    TEMP_POINTER=""
  done
  [[ -n "$TEMP_POINTER" && -L "$TEMP_POINTER" ]] \
    || die "could not create temporary release pointer in $parent"
  mv -Tf "$TEMP_POINTER" "$CURRENT_LINK"
  TEMP_POINTER=""
  [[ "$(realpath -e "$CURRENT_LINK")" == "$TARGET_RELEASE" ]] \
    || die "active release pointer did not resolve to the target release"
  sync -f "$parent"
}

wait_for_ready(){
  local elapsed=0 url
  url="${SATURN_GO_READY_URL%%\?*}?expected_commit=$TARGET_COMMIT"
  while (( elapsed < READY_TIMEOUT_SECONDS )); do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  curl -fsS --max-time 2 "$url" >/dev/null
}

mark_failed(){
  local rc="$?" command="${BASH_COMMAND:-unknown command}"
  trap - ERR
  if [[ -n "$TEMP_POINTER" && -L "$TEMP_POINTER" ]]; then
    rm -f -- "$TEMP_POINTER"
  fi
  if (( TRANSACTION_PREPARED )); then
    write_transaction "failed" "$PHASE" "Activation failed before commit: $command" || true
  fi
  log "activation failed during $PHASE; automatic rollback is not enabled until REM-0204"
  exit "$rc"
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi

if [[ "${1:-}" == "--validate" ]]; then
  VALIDATE_ONLY=1
  shift
fi
[[ $# -eq 1 ]] || { usage >&2; exit 2; }
TARGET_COMMIT="$1"

need_cmd curl
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
[[ -f "$COMPONENTS_FILE" ]] || die "trusted component policy missing: $COMPONENTS_FILE"
TARGET_RELEASE="$(validate_release "$TARGET_COMMIT")"
if (( VALIDATE_ONLY )); then
  log "installed release validation passed: $TARGET_RELEASE"
  exit 0
fi

[[ "$ACTIVATION_ENABLED" == "1" ]] \
  || die "activation is disabled; REM-0204 automatic rollback must be completed before production enablement"
prepare_transaction_directory
existing_status="$(transaction_status)"
case "$existing_status" in
  ""|committed) ;;
  *) die "unresolved deployment transaction has status '$existing_status': $TRANSACTION_FILE" ;;
esac
IFS=$'\t' read -r PREVIOUS_COMMIT PREVIOUS_RELEASE < <(current_release_identity)
[[ "$PREVIOUS_COMMIT" != "$TARGET_COMMIT" ]] || die "release is already active: $TARGET_COMMIT"
SATURN_GO_DROPIN="$SYSTEMD_ROOT/$SATURN_GO_SERVICE.d/50-saturn-release.conf"
BRIDGE_DROPIN="$SYSTEMD_ROOT/$BRIDGE_SERVICE.d/50-saturn-release.conf"
P2APP_DROPIN="$SYSTEMD_ROOT/$P2APP_SERVICE.d/50-saturn-release.conf"
[[ -e "$SATURN_GO_DROPIN" || -L "$SATURN_GO_DROPIN" ]] && SATURN_GO_DROPIN_EXISTED=1
[[ -e "$BRIDGE_DROPIN" || -L "$BRIDGE_DROPIN" ]] && BRIDGE_DROPIN_EXISTED=1
[[ -e "$P2APP_DROPIN" || -L "$P2APP_DROPIN" ]] && P2APP_DROPIN_EXISTED=1

PHASE="prepare"
write_transaction "prepared" "$PHASE" "Validated target and persisted deployment intent" prepare
TRANSACTION_PREPARED=1
trap mark_failed ERR

PHASE="service-wiring"
write_transaction "activating" "$PHASE" "Installing stable-pointer systemd overrides"
install_release_dropins
systemctl daemon-reload

PHASE="pointer-switch"
write_transaction "activating" "$PHASE" "Atomically switching the active release pointer"
switch_current_pointer

PHASE="service-stop"
write_transaction "activating" "$PHASE" "Stopping affected services in dependency order"
systemctl stop "$SATURN_GO_SERVICE"
systemctl stop "$BRIDGE_SERVICE"
systemctl stop "$P2APP_SERVICE"

PHASE="service-start"
write_transaction "activating" "$PHASE" "Starting affected services in dependency order"
systemctl start "$P2APP_SERVICE"
systemctl is-active --quiet "$P2APP_SERVICE"
systemctl start "$BRIDGE_SERVICE"
systemctl is-active --quiet "$BRIDGE_SERVICE"
systemctl start "$SATURN_GO_SERVICE"
systemctl is-active --quiet "$SATURN_GO_SERVICE"

PHASE="readiness"
write_transaction "verifying" "$PHASE" "Waiting for target-aware readiness"
wait_for_ready

PHASE="commit"
write_transaction "committed" "$PHASE" "Target release is active and ready"
trap - ERR
log "activated and verified release: $TARGET_COMMIT"
