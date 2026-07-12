#!/usr/bin/env bash
set -Eeuo pipefail

# Root-owned deployment broker for Saturn Go self-updates. The unprivileged
# service may prepare a payload, but this broker never executes staged code and
# never installs staged privileged helpers or systemd unit contents.

CONFIG_FILE="${SATURN_GO_DEPLOY_CONFIG:-/etc/default/saturn-go-deploy}"
VALIDATE_ONLY=0

die(){ printf '[saturn-go-deploy] ERROR: %s\n' "$*" >&2; exit 1; }
info(){ printf '[saturn-go-deploy] %s\n' "$*"; }

usage(){
  cat <<'EOF'
Usage:
  saturn-go-deploy-root.sh <stage-directory>
  saturn-go-deploy-root.sh --validate <stage-directory>

The deployment form must run as root. --validate performs payload structure,
mode, and checksum checks without changing the system.
EOF
}

if [[ "${1:-}" == "--validate" ]]; then
  VALIDATE_ONLY=1
  shift
fi
[[ $# -eq 1 ]] || { usage >&2; exit 2; }
STAGE_INPUT="$1"

RUN_USER="pi"
RUN_GROUP="pi"
STAGING_ROOT="/var/lib/saturn-state/repo-staging"
STATUS_FILE="/var/lib/saturn-state/saturngo_deploy_status.json"
SATURN_ROOT="/opt/saturn-go"
SATURN_GO_BIN="/opt/saturn-go/bin/saturn-go"
SATURN_GO_SERVICE="saturn-go.service"
BRIDGE_BIN="/opt/saturn-go/bin/saturn-bridge"
BRIDGE_SERVICE="saturn-bridge.service"
BRIDGE_SERVICE_FILE="/etc/systemd/system/saturn-bridge.service"
WEB_ROOT="/var/lib/saturn-web"
SCRIPTS_DIR="/opt/saturn-go/scripts"
BRIDGE_MAX_RATE_KHZ="192"
BRIDGE_OPUS_ENABLED="1"
BRIDGE_RF_TX_ENABLED="1"
SATURN_GO_HEALTH_URL="http://127.0.0.1:8080/healthz"

load_root_config(){
  [[ -f "$CONFIG_FILE" ]] || return 0
  local owner mode
  owner="$(stat -c '%u' "$CONFIG_FILE")"
  mode="$(stat -c '%a' "$CONFIG_FILE")"
  [[ "$owner" == "0" ]] || die "deployment config is not root-owned: $CONFIG_FILE"
  (( (8#$mode & 8#022) == 0 )) || die "deployment config is group/world writable: $CONFIG_FILE"
  # shellcheck disable=SC1090
  source "$CONFIG_FILE"
}

safe_leaf_name(){
  [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]
}

validate_stage(){
  local stage="$1" enforce_root="$2" run_uid bad path listed expected actual
  [[ -d "$stage" ]] || die "stage directory not found: $stage"
  stage="$(realpath -e "$stage")"

  if (( enforce_root )); then
    local root
    root="$(realpath -e "$STAGING_ROOT")"
    [[ "$stage" == "$root/"* ]] || die "stage is outside trusted staging root: $stage"
    run_uid="$(id -u "$RUN_USER")"
    bad="$(find "$stage" -xdev ! -uid "$run_uid" -print -quit)"
    [[ -z "$bad" ]] || die "stage entry is not owned by $RUN_USER: $bad"
  fi

  bad="$(find "$stage" -xdev -type l -print -quit)"
  [[ -z "$bad" ]] || die "symbolic links are not allowed in deploy payloads: $bad"
  bad="$(find "$stage" -xdev -perm /022 -print -quit)"
  [[ -z "$bad" ]] || die "group/world-writable stage entry rejected: $bad"

  [[ -f "$stage/saturn-go" && -x "$stage/saturn-go" ]] || die "missing executable saturn-go payload"
  [[ -d "$stage/webroot" ]] || die "missing webroot payload directory"
  [[ -d "$stage/scripts" ]] || die "missing scripts payload directory"
  [[ -f "$stage/SHA256SUMS" ]] || die "missing SHA256SUMS payload manifest"

  while IFS= read -r path; do
    safe_leaf_name "$(basename "$path")" || die "unsafe payload filename: $path"
    [[ -f "$path" ]] || die "payload entries must be regular files: $path"
  done < <(find "$stage/webroot" "$stage/scripts" -mindepth 1 -maxdepth 1 -print)

  while IFS= read -r listed; do
    [[ "$listed" != /* && "$listed" != *".."* ]] || die "unsafe checksum path: $listed"
    case "$listed" in
      saturn-go|saturn-bridge|webroot/*|scripts/*) ;;
      *) die "checksum manifest names an unsupported payload path: $listed" ;;
    esac
  done < <(awk '{ sub(/^\*/, "", $2); print $2 }' "$stage/SHA256SUMS")

  expected="$(find "$stage" -type f ! -name SHA256SUMS -printf '%P\n' | sort)"
  actual="$(awk '{ sub(/^\*/, "", $2); print $2 }' "$stage/SHA256SUMS" | sort)"
  [[ "$actual" == "$expected" ]] || die "checksum manifest does not exactly cover the payload"

  (cd "$stage" && sha256sum -c SHA256SUMS >/dev/null) || die "payload checksum verification failed"
  printf '%s\n' "$stage"
}

write_status(){
  local status="$1" message="$2" exit_code="$3" now uid gid tmp
  now="$(date -Is)"
  uid="$(id -u "$RUN_USER")"
  gid="$(id -g "$RUN_USER")"
  install -d -m 0750 -o "$uid" -g "$gid" "$(dirname "$STATUS_FILE")"
  tmp="$(mktemp "$(dirname "$STATUS_FILE")/.saturngo-deploy.XXXXXX")"
  printf '{\n  "status": "%s",\n  "phase": "root-deploy",\n  "message": "%s",\n  "updated_at": "%s",\n  "exit_code": %s\n}\n' \
    "$status" "$message" "$now" "$exit_code" >"$tmp"
  chown "$uid:$gid" "$tmp"
  chmod 0640 "$tmp"
  mv -f "$tmp" "$STATUS_FILE"
}

declare -a ROLLBACK_DESTS=()
declare -a ROLLBACK_KEYS=()
declare -a ROLLBACK_EXISTED=()
BACKUP_DIR=""
SNAPSHOT_DIR=""
SATURN_GO_WAS_ACTIVE=0
BRIDGE_WAS_ACTIVE=0

cleanup_temporary_state(){
  if [[ -n "$BACKUP_DIR" && -d "$BACKUP_DIR" ]]; then
    rm -rf "$BACKUP_DIR"
  fi
  if [[ -n "$SNAPSHOT_DIR" && -d "$SNAPSHOT_DIR" ]]; then
    rm -rf "$SNAPSHOT_DIR"
  fi
}
trap cleanup_temporary_state EXIT

backup_destination(){
  local dest="$1" key="$2" existed=0
  install -d -m 0700 "$BACKUP_DIR/$(dirname "$key")"
  if [[ -e "$dest" || -L "$dest" ]]; then
    cp -a --no-dereference "$dest" "$BACKUP_DIR/$key"
    existed=1
  fi
  ROLLBACK_DESTS+=("$dest")
  ROLLBACK_KEYS+=("$key")
  ROLLBACK_EXISTED+=("$existed")
}

install_payload_file(){
  local src="$1" dest="$2" mode="$3" owner="$4" group="$5" key="$6"
  backup_destination "$dest" "$key"
  install -D -m "$mode" -o "$owner" -g "$group" "$src" "$dest"
}

restore_payload(){
  local i dest key existed
  set +e
  for ((i=${#ROLLBACK_DESTS[@]}-1; i>=0; i--)); do
    dest="${ROLLBACK_DESTS[$i]}"
    key="${ROLLBACK_KEYS[$i]}"
    existed="${ROLLBACK_EXISTED[$i]}"
    rm -f -- "$dest"
    if (( existed )); then
      install -d -m 0755 "$(dirname "$dest")"
      cp -a --no-dereference "$BACKUP_DIR/$key" "$dest"
    fi
  done
  systemctl daemon-reload >/dev/null 2>&1 || true
  (( BRIDGE_WAS_ACTIVE )) && systemctl start "$BRIDGE_SERVICE" >/dev/null 2>&1 || true
  (( SATURN_GO_WAS_ACTIVE )) && systemctl start "$SATURN_GO_SERVICE" >/dev/null 2>&1 || true
}

on_error(){
  local rc="$?"
  trap - ERR
  systemctl stop "$BRIDGE_SERVICE" >/dev/null 2>&1 || true
  systemctl stop "$SATURN_GO_SERVICE" >/dev/null 2>&1 || true
  restore_payload
  write_status "error" "Root deploy failed; complete payload restored" "$rc" || true
  [[ -n "$BACKUP_DIR" ]] && rm -rf "$BACKUP_DIR"
  [[ -n "$SNAPSHOT_DIR" ]] && rm -rf "$SNAPSHOT_DIR"
  exit "$rc"
}

write_bridge_unit(){
  local tmp="$1"
  cat >"$tmp" <<EOF
[Unit]
Description=Saturn Bridge (WDSP 2.00)
After=network-online.target p2app.service
Wants=network-online.target p2app.service

[Service]
Type=simple
User=${RUN_USER}
Group=${RUN_GROUP}
WorkingDirectory=${SATURN_ROOT}
ExecStart=${BRIDGE_BIN}
Restart=on-failure
RestartSec=2
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
RestrictSUIDSGID=yes
LockPersonality=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
Environment=SATURN_BRIDGE_RADIO_HOST=127.0.0.1
Environment=SATURN_BRIDGE_RADIO_PORT=1024
Environment=SATURN_BRIDGE_CLIENT_HOST=127.0.0.1
Environment=SATURN_BRIDGE_CLIENT_PORT=12000
Environment=SATURN_BRIDGE_TCI_HOST=127.0.0.1
Environment=SATURN_BRIDGE_TCI_PORT=50001
Environment=SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ=${BRIDGE_MAX_RATE_KHZ}
Environment=SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED=${BRIDGE_OPUS_ENABLED}
Environment=SATURN_REMOTE_TX_RF_ENABLED=${BRIDGE_RF_TX_ENABLED}

[Install]
WantedBy=multi-user.target
EOF
}

listener_owned_by_service(){
  local service="$1" port="$2" pid
  pid="$(systemctl show -p MainPID --value "$service")"
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
  ss -ltnp "sport = :$port" 2>/dev/null | grep -Fq "pid=$pid,"
}

load_root_config
STAGE_DIR="$(validate_stage "$STAGE_INPUT" "$(( ! VALIDATE_ONLY ))")"
if (( VALIDATE_ONLY )); then
  info "payload validation passed: $STAGE_DIR"
  exit 0
fi

[[ $(id -u) -eq 0 ]] || die "deployment must run as root"
id "$RUN_USER" >/dev/null 2>&1 || die "configured run user does not exist: $RUN_USER"
getent group "$RUN_GROUP" >/dev/null 2>&1 || die "configured run group does not exist: $RUN_GROUP"

install -d -m 0755 /var/lib/saturn-state
SNAPSHOT_DIR="$(mktemp -d /var/lib/saturn-state/root-deploy-payload.XXXXXX)"
chmod 0700 "$SNAPSHOT_DIR"
cp -a --no-dereference "$STAGE_DIR/." "$SNAPSHOT_DIR/"
chown -R root:root "$SNAPSHOT_DIR"
STAGE_DIR="$(validate_stage "$SNAPSHOT_DIR" 0)"
BACKUP_DIR="$(mktemp -d /var/lib/saturn-state/root-deploy-rollback.XXXXXX)"
chmod 0700 "$BACKUP_DIR"
systemctl is-active --quiet "$SATURN_GO_SERVICE" && SATURN_GO_WAS_ACTIVE=1 || true
systemctl is-active --quiet "$BRIDGE_SERVICE" && BRIDGE_WAS_ACTIVE=1 || true
trap on_error ERR
write_status "running" "Installing validated Saturn Go payload" "null"

systemctl stop "$SATURN_GO_SERVICE"
if [[ -f "$STAGE_DIR/saturn-bridge" ]]; then
  systemctl stop "$BRIDGE_SERVICE" >/dev/null 2>&1 || true
fi

install_payload_file "$STAGE_DIR/saturn-go" "$SATURN_GO_BIN" 0755 root root bin/saturn-go
if [[ -f "$STAGE_DIR/saturn-bridge" ]]; then
  install_payload_file "$STAGE_DIR/saturn-bridge" "$BRIDGE_BIN" 0755 root root bin/saturn-bridge
  unit_tmp="$(mktemp)"
  write_bridge_unit "$unit_tmp"
  install_payload_file "$unit_tmp" "$BRIDGE_SERVICE_FILE" 0644 root root systemd/saturn-bridge.service
  rm -f "$unit_tmp"
fi

while IFS= read -r src; do
  name="$(basename "$src")"
  install_payload_file "$src" "$WEB_ROOT/$name" 0644 root root "webroot/$name"
done < <(find "$STAGE_DIR/webroot" -mindepth 1 -maxdepth 1 -type f | sort)

while IFS= read -r src; do
  name="$(basename "$src")"
  mode=0644
  case "$name" in *.sh|*.py) mode=0755 ;; esac
  install_payload_file "$src" "$SCRIPTS_DIR/$name" "$mode" "$RUN_USER" "$RUN_GROUP" "scripts/$name"
done < <(find "$STAGE_DIR/scripts" -mindepth 1 -maxdepth 1 -type f | sort)

systemctl daemon-reload
if [[ -f "$STAGE_DIR/saturn-bridge" ]]; then
  systemctl enable "$BRIDGE_SERVICE" >/dev/null
  systemctl start "$BRIDGE_SERVICE"
  systemctl is-active --quiet "$BRIDGE_SERVICE"
  for _ in {1..20}; do
    listener_owned_by_service "$BRIDGE_SERVICE" 50001 && break
    sleep 1
  done
  listener_owned_by_service "$BRIDGE_SERVICE" 50001
fi

systemctl start "$SATURN_GO_SERVICE"
systemctl is-active --quiet "$SATURN_GO_SERVICE"
for _ in {1..20}; do
  curl -fsS --max-time 2 "$SATURN_GO_HEALTH_URL" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS --max-time 2 "$SATURN_GO_HEALTH_URL" >/dev/null

trap - ERR
write_status "success" "Validated payload installed" "0"
rm -rf "$BACKUP_DIR"
rm -rf "$SNAPSHOT_DIR"
info "deployment completed"
