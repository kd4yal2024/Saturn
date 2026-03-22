#!/usr/bin/env bash
set -euo pipefail

# update-saturn-go.sh
# Rebuild and redeploy Saturn Update Manager (saturn-go) from the active Saturn repo.
# Intended to be run from the web UI via /run (SSE terminal output).

SCRIPT_VERSION="1.1.0"
SCRIPT_NAME="$(basename "$0")"

SKIP_GIT=0
SKIP_BUILD=0
SKIP_DEPLOY=0
DRY_RUN=0
VERBOSE=0
STATUS_PHASE="init"

progress(){ echo "Progress: $1%"; }
info(){ echo "$@"; }
warn(){ echo "WARN: $*" >&2; }
die(){ echo "ERR: $*" >&2; exit 1; }
json_bool(){ (( $1 )) && printf 'true' || printf 'false'; }
json_escape(){
  local s="${1-}"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  s="${s//$'\n'/\\n}"
  s="${s//$'\r'/\\r}"
  s="${s//$'\t'/\\t}"
  printf '%s' "$s"
}
write_status(){
  local status="${1:-running}"
  local phase="${2:-$STATUS_PHASE}"
  local message="${3:-}"
  local finished="${4:-0}"
  local exit_code="${5:-null}"
  local now finished_at unit_json
  now="$(date -Is)"
  finished_at='null'
  if (( finished )); then
    finished_at="\"$(json_escape "$now")\""
  fi
  if [[ -n "${UNIT_NAME:-}" ]]; then
    unit_json="\"$(json_escape "$UNIT_NAME")\""
  else
    unit_json='null'
  fi
  mkdir -p "$(dirname "$STATUS_FILE")" >/dev/null 2>&1 || true
  cat > "$STATUS_FILE" <<EOF
{
  "status": "$(json_escape "$status")",
  "phase": "$(json_escape "$phase")",
  "message": "$(json_escape "$message")",
  "updated_at": "$(json_escape "$now")",
  "started_at": "$(json_escape "$RUN_STARTED_AT")",
  "finished_at": $finished_at,
  "exit_code": $exit_code,
  "unit_name": $unit_json,
  "service_name": "$(json_escape "$SERVICE_NAME")",
  "repo_root": "$(json_escape "$REPO_ROOT")",
  "ref": "$(json_escape "$SATURNGO_REF")",
  "remote": "$(json_escape "$SATURNGO_REMOTE")",
  "flags": {
    "verbose": $(json_bool "$VERBOSE"),
    "dry_run": $(json_bool "$DRY_RUN"),
    "skip_git": $(json_bool "$SKIP_GIT"),
    "skip_build": $(json_bool "$SKIP_BUILD"),
    "skip_deploy": $(json_bool "$SKIP_DEPLOY")
  }
}
EOF
  chmod 666 "$STATUS_FILE" >/dev/null 2>&1 || true
}

run_cmd(){
  if (( DRY_RUN )); then
    info "[dry-run] $*"
    return 0
  fi
  if (( VERBOSE )); then
    info "+ $*"
  fi
  "$@"
}

need_cmd(){
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-git) SKIP_GIT=1; shift ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --skip-deploy) SKIP_DEPLOY=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --verbose) VERBOSE=1; shift ;;
    -h|--help)
      cat <<'EOF'
Usage: update-saturn-go.sh [flags]
  --skip-git       Skip git fetch/reset
  --skip-build     Skip cargo build --release
  --skip-deploy    Skip template/binary deployment dispatch
  --dry-run        Print actions only
  --verbose        Echo executed commands
EOF
      exit 0
      ;;
    *)
      die "Unknown argument: $1"
      ;;
  esac
done

progress 2
info "Saturn Go self-update ${SCRIPT_VERSION}"

need_cmd git
need_cmd cargo
need_cmd systemd-run

RUN_STARTED_AT="$(date -Is)"
REPO_ROOT="${SATURN_ACTIVE_REPO_ROOT:-${SATURN_REPO_ROOT:-}}"
[[ -n "$REPO_ROOT" ]] || die "SATURN_ACTIVE_REPO_ROOT is not set"
[[ -d "$REPO_ROOT/.git" ]] || die "Repo root is not a git checkout: $REPO_ROOT"
[[ -d "$REPO_ROOT/update_manager" ]] || die "Repo root does not contain update_manager/: $REPO_ROOT"
EXTRA_PACKAGED_SCRIPTS=(
  "$REPO_ROOT/scripts/fix-LED-power-button.sh"
  "$REPO_ROOT/scripts/install-shutdown-waiter-service.sh"
  "$REPO_ROOT/scripts/shutdown-waiter.sh"
  "$REPO_ROOT/scripts/setup-eth-fallback.sh"
)
PRIVILEGED_HELPER_SCRIPTS=(
  "$REPO_ROOT/scripts/fix-LED-power-button.sh"
  "$REPO_ROOT/scripts/install-shutdown-waiter-service.sh"
  "$REPO_ROOT/scripts/shutdown-waiter.sh"
  "$REPO_ROOT/scripts/setup-eth-fallback.sh"
)

SATURNGO_URL="${SATURN_SATURNGO_POLICY_URL:-}"
SATURNGO_REMOTE="${SATURN_SATURNGO_POLICY_REMOTE:-origin}"
SATURNGO_REF="${SATURN_SATURNGO_POLICY_REF:-main}"

RUST_DIR="$REPO_ROOT/update_manager/rust-server"
TEMPLATES_DIR="$REPO_ROOT/update_manager/templates"
SCRIPTS_SRC_DIR="$REPO_ROOT/update_manager/scripts"
BUILD_TMP_DIR="$RUST_DIR/.tmp"
BUILD_TARGET_DIR="$RUST_DIR/target-local"

SERVICE_NAME="${SATURN_SATURNGO_SERVICE_NAME:-saturn-go.service}"
DEPLOY_RUN_USER="${SATURN_SATURNGO_RUN_USER:-$(id -un)}"
DEPLOY_BIN="${SATURN_SATURNGO_BIN_DEST:-/opt/saturn-go/bin/saturn-go}"
DEPLOY_WEBROOT="${SATURN_SATURNGO_WEBROOT:-/var/lib/saturn-web}"
DEPLOY_SCRIPTS_DIR="${SATURN_SATURNGO_SCRIPTS_DIR:-/opt/saturn-go/scripts}"
DEPLOY_PRIVILEGED_SCRIPTS_DIR="${SATURN_SATURNGO_PRIVILEGED_SCRIPTS_DIR:-/usr/local/lib/saturn-go/scripts}"
DEPLOY_SUDOERS_FILE="${SATURN_SATURNGO_SUDOERS_FILE:-/etc/sudoers.d/saturn-go-maintenance}"
STATUS_FILE="${SATURN_SATURNGO_DEPLOY_STATUS_FILE:-/var/lib/saturn-state/saturngo_deploy_status.json}"

trap 'rc=$?; if (( rc != 0 )); then write_status "error" "$STATUS_PHASE" "Saturn Go self-update failed" 1 "$rc"; fi' EXIT

[[ -f "$RUST_DIR/Cargo.toml" ]] || die "Rust server Cargo.toml not found: $RUST_DIR/Cargo.toml"
[[ -d "$TEMPLATES_DIR" ]] || die "Templates dir not found: $TEMPLATES_DIR"
[[ -d "$SCRIPTS_SRC_DIR" ]] || die "Scripts dir not found: $SCRIPTS_SRC_DIR"
[[ -f "$SCRIPTS_SRC_DIR/config.json" ]] || die "Missing config.json in $SCRIPTS_SRC_DIR"
[[ -f "$SCRIPTS_SRC_DIR/themes.json" ]] || die "Missing themes.json in $SCRIPTS_SRC_DIR"
for extra_script in "${EXTRA_PACKAGED_SCRIPTS[@]}"; do
  [[ -f "$extra_script" ]] || die "Extra packaged script not found: $extra_script"
done
for privileged_script in "${PRIVILEGED_HELPER_SCRIPTS[@]}"; do
  [[ -f "$privileged_script" ]] || die "Privileged helper script not found: $privileged_script"
done

write_status "running" "init" "Starting Saturn Go self-update"

progress 5
info "Repo root: $REPO_ROOT"
if [[ -n "$SATURNGO_URL" ]]; then
  info "Remote: ${SATURNGO_REMOTE} -> ${SATURNGO_URL}"
else
  info "Remote: ${SATURNGO_REMOTE} (git update skipped; using active repo root)"
fi
info "Ref: ${SATURNGO_REF}"
info "Service: ${SERVICE_NAME}"

if (( ! SKIP_GIT )); then
  [[ -n "$SATURNGO_URL" ]] || die "Saturn Go policy repo URL not configured. Save Saturn Go page settings first."
  STATUS_PHASE="git"
  write_status "running" "$STATUS_PHASE" "Updating git checkout"
  progress 10
  info "Checking repo status..."
  if ! git -C "$REPO_ROOT" diff --quiet --ignore-submodules --; then
    die "Repo has uncommitted working-tree changes. Commit/stash them or use --skip-git."
  fi
  if ! git -C "$REPO_ROOT" diff --quiet --cached --ignore-submodules --; then
    die "Repo has staged changes. Commit/stash them or use --skip-git."
  fi

  info "Updating git remote and fetching target ref..."
  run_cmd git -C "$REPO_ROOT" remote set-url "$SATURNGO_REMOTE" "$SATURNGO_URL"
  run_cmd git -C "$REPO_ROOT" fetch --prune "$SATURNGO_REMOTE" "$SATURNGO_REF"
  if (( ! DRY_RUN )); then
    # Reset only after clean-tree checks to avoid clobbering local work unexpectedly.
    run_cmd git -C "$REPO_ROOT" reset --hard FETCH_HEAD
  else
    info "[dry-run] git -C \"$REPO_ROOT\" reset --hard FETCH_HEAD"
  fi
else
  STATUS_PHASE="git"
  write_status "running" "$STATUS_PHASE" "Skipping git update"
  info "Skipping git update (--skip-git)"
fi

progress 30

if (( ! SKIP_BUILD )); then
  STATUS_PHASE="build"
  write_status "running" "$STATUS_PHASE" "Building saturn-go release binary"
  info "Building saturn-go release binary..."
  run_cmd mkdir -p "$BUILD_TMP_DIR" "$BUILD_TARGET_DIR"
  if (( DRY_RUN )); then
    info "[dry-run] TMPDIR=$BUILD_TMP_DIR CARGO_TARGET_DIR=$BUILD_TARGET_DIR cargo build --release -j1 (cwd=$RUST_DIR)"
  else
    (
      cd "$RUST_DIR"
      TMPDIR="$BUILD_TMP_DIR" \
      CARGO_TARGET_DIR="$BUILD_TARGET_DIR" \
      cargo build --release -j1
    )
  fi
else
  STATUS_PHASE="build"
  write_status "running" "$STATUS_PHASE" "Skipping build"
  info "Skipping build (--skip-build)"
fi

BIN_SRC="$BUILD_TARGET_DIR/release/saturn-go"
if (( ! DRY_RUN )) && (( ! SKIP_BUILD )) && [[ ! -x "$BIN_SRC" ]]; then
  die "Built binary not found: $BIN_SRC"
fi

progress 65

if (( SKIP_DEPLOY )); then
  STATUS_PHASE="deploy"
  write_status "success" "$STATUS_PHASE" "Skipped deploy (--skip-deploy); build/update steps completed" 1 0
  info "Skipping deploy (--skip-deploy)"
  progress 100
  info "Done"
  exit 0
fi

STAGE_BASE="${SATURN_SATURNGO_STAGE_BASE:-/var/tmp/saturngo-deploy-stage}"
STAMP="$(date +%Y%m%d%H%M%S)"
STAGE_DIR="$STAGE_BASE/$STAMP"
STAGE_WEB_DIR="$STAGE_DIR/webroot"
STAGE_SCRIPTS_DIR="$STAGE_DIR/scripts"
STAGE_PRIVILEGED_SCRIPTS_DIR="$STAGE_DIR/privileged-scripts"
UNIT_NAME="saturn-go-self-deploy-${STAMP}"

info "Preparing staged deploy payload..."
STATUS_PHASE="stage"
write_status "running" "$STATUS_PHASE" "Preparing staged deploy payload"
run_cmd mkdir -p "$STAGE_WEB_DIR" "$STAGE_SCRIPTS_DIR" "$STAGE_PRIVILEGED_SCRIPTS_DIR"
if (( ! DRY_RUN )); then
  cp "$BIN_SRC" "$STAGE_DIR/saturn-go"
  cp "$TEMPLATES_DIR"/*.html "$STAGE_WEB_DIR/"
  cp "$SCRIPTS_SRC_DIR/config.json" "$STAGE_WEB_DIR/config.json"
  cp "$SCRIPTS_SRC_DIR/themes.json" "$STAGE_WEB_DIR/themes.json"
  find "$SCRIPTS_SRC_DIR" -maxdepth 1 -type f -exec cp "{}" "$STAGE_SCRIPTS_DIR/" \;
  for extra_script in "${EXTRA_PACKAGED_SCRIPTS[@]}"; do
    cp "$extra_script" "$STAGE_SCRIPTS_DIR/"
  done
  for privileged_script in "${PRIVILEGED_HELPER_SCRIPTS[@]}"; do
    cp "$privileged_script" "$STAGE_PRIVILEGED_SCRIPTS_DIR/"
  done
  chmod 755 "$STAGE_DIR/saturn-go"
  find "$STAGE_SCRIPTS_DIR" -maxdepth 1 -type f \( -name '*.sh' -o -name '*.py' \) -print0 | xargs -0 -r chmod 755
  find "$STAGE_SCRIPTS_DIR" -maxdepth 1 -type f ! \( -name '*.sh' -o -name '*.py' \) -print0 | xargs -0 -r chmod 644
  find "$STAGE_PRIVILEGED_SCRIPTS_DIR" -maxdepth 1 -type f -print0 | xargs -0 -r chmod 755
  find "$STAGE_WEB_DIR" -maxdepth 1 -type f -print0 | xargs -0 -r chmod 644
else
  info "[dry-run] stage binary, web assets, packaged scripts, and privileged helpers under $STAGE_DIR"
fi

# Copy web assets immediately so the UI updates even before the service restart.
info "Deploying web assets..."
STATUS_PHASE="deploy"
write_status "running" "$STATUS_PHASE" "Copying web assets to webroot"
run_cmd sudo -n cp "$STAGE_WEB_DIR/"* "$DEPLOY_WEBROOT/"

HELPER="$STAGE_DIR/deploy-root-helper.sh"
if (( ! DRY_RUN )); then
  cat > "$HELPER" <<EOF
#!/usr/bin/env bash
set -euo pipefail
STATUS_FILE="$(printf '%s' "$STATUS_FILE")"
UNIT_NAME="$(printf '%s' "$UNIT_NAME")"
SERVICE_NAME="$(printf '%s' "$SERVICE_NAME")"
DEPLOY_RUN_USER="$(printf '%s' "$DEPLOY_RUN_USER")"
DEPLOY_PRIVILEGED_SCRIPTS_DIR="$(printf '%s' "$DEPLOY_PRIVILEGED_SCRIPTS_DIR")"
DEPLOY_SUDOERS_FILE="$(printf '%s' "$DEPLOY_SUDOERS_FILE")"
STARTED_AT="$(printf '%s' "$RUN_STARTED_AT")"
json_escape(){
  local s="\${1-}"
  s="\${s//\\\\/\\\\\\\\}"
  s="\${s//\\\"/\\\\\\\"}"
  printf '%s' "\$s"
}
write_status(){
  local status="\$1"
  local phase="\$2"
  local message="\$3"
  local finished="\${4:-0}"
  local exit_code="\${5:-null}"
  local now finished_at
  now="\$(date -Is)"
  finished_at='null'
  if (( finished )); then
    finished_at="\"\$(json_escape "\$now")\""
  fi
  mkdir -p "\$(dirname "\$STATUS_FILE")" >/dev/null 2>&1 || true
  cat > "\$STATUS_FILE" <<JSON
{
  "status": "\$(json_escape "\$status")",
  "phase": "\$(json_escape "\$phase")",
  "message": "\$(json_escape "\$message")",
  "updated_at": "\$(json_escape "\$now")",
  "started_at": "\$(json_escape "\$STARTED_AT")",
  "finished_at": \$finished_at,
  "exit_code": \$exit_code,
  "unit_name": "\$(json_escape "\$UNIT_NAME")",
  "service_name": "\$(json_escape "\$SERVICE_NAME")"
}
JSON
  chmod 666 "\$STATUS_FILE" >/dev/null 2>&1 || true
}
trap 'rc=\$?; write_status "error" "root-deploy" "Root deploy helper failed" 1 "\$rc"; exit "\$rc"' ERR
write_status "running" "root-deploy" "Root deploy helper started"
SCRIPT_UID="\$(id -u "$DEPLOY_RUN_USER")"
SCRIPT_GID="\$(id -g "$DEPLOY_RUN_USER")"
install -d -m 0755 "$DEPLOY_WEBROOT"
install -d -m 0775 -o "\$SCRIPT_UID" -g "\$SCRIPT_GID" "$DEPLOY_SCRIPTS_DIR"
install -d -m 0755 -o root -g root "$DEPLOY_PRIVILEGED_SCRIPTS_DIR"
systemctl stop "$SERVICE_NAME"
install -m 0755 "$STAGE_DIR/saturn-go" "$DEPLOY_BIN"
for src in "$STAGE_WEB_DIR/"*; do
  [[ -f "\$src" ]] || continue
  install -m 0644 "\$src" "$DEPLOY_WEBROOT/\$(basename "\$src")"
done
for src in "$STAGE_SCRIPTS_DIR/"*; do
  [[ -f "\$src" ]] || continue
  mode=0644
  case "\$src" in
    *.sh|*.py) mode=0755 ;;
  esac
  install -m "\$mode" -o "\$SCRIPT_UID" -g "\$SCRIPT_GID" "\$src" "$DEPLOY_SCRIPTS_DIR/\$(basename "\$src")"
done
for src in "$STAGE_PRIVILEGED_SCRIPTS_DIR/"*; do
  [[ -f "\$src" ]] || continue
  install -m 0755 -o root -g root "\$src" "$DEPLOY_PRIVILEGED_SCRIPTS_DIR/\$(basename "\$src")"
done
cat >"$DEPLOY_SUDOERS_FILE" <<SUDOERS
# Managed by update-saturn-go.sh
Defaults:${DEPLOY_RUN_USER} secure_path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/install-shutdown-waiter-service.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/install-shutdown-waiter-service.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/setup-eth-fallback.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/setup-eth-fallback.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/fix-LED-power-button.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/fix-LED-power-button.sh *
SUDOERS
chmod 0440 "$DEPLOY_SUDOERS_FILE"
if command -v visudo >/dev/null 2>&1; then
  visudo -cf "$DEPLOY_SUDOERS_FILE" >/dev/null
fi
systemctl start "$SERVICE_NAME"
write_status "success" "root-deploy" "Root deploy helper completed" 1 0
EOF
  chmod 755 "$HELPER"
fi

progress 85
info "Dispatching detached root deploy helper via systemd-run..."
STATUS_PHASE="dispatch"
if (( DRY_RUN )); then
  write_status "success" "$STATUS_PHASE" "Dry-run completed; deploy helper not executed" 1 0
  info "[dry-run] sudo -n systemd-run --unit $UNIT_NAME --collect --no-block /bin/bash $HELPER"
else
  write_status "running" "$STATUS_PHASE" "Dispatching detached root deploy helper"
  run_cmd sudo -n systemd-run --unit "$UNIT_NAME" --collect --no-block /bin/bash "$HELPER"
  write_status "dispatched" "$STATUS_PHASE" "Deploy helper dispatched; service restart in progress"
fi

progress 95
info "Deploy dispatched as transient unit: $UNIT_NAME"
info "The web connection may drop when $SERVICE_NAME restarts. Reload the page after ~10-20 seconds."
progress 100
info "Done"
