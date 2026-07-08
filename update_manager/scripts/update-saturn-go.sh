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
  chmod 0640 "$STATUS_FILE" >/dev/null 2>&1 || true
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

run_git_no_prompt(){
  if (( DRY_RUN )); then
    info "[dry-run] GIT_TERMINAL_PROMPT=0 $*"
    return 0
  fi
  if (( VERBOSE )); then
    info "+ GIT_TERMINAL_PROMPT=0 $*"
  fi
  GIT_TERMINAL_PROMPT=0 "$@"
}

summarize_cmd_error(){
  local out="${1-}"
  local last_line=""
  while IFS= read -r line; do
    [[ -n "$line" ]] && last_line="$line"
  done <<< "$out"
  if [[ -n "$last_line" ]]; then
    printf '%s\n' "$last_line"
  else
    printf 'Unknown command error\n'
  fi
}

ensure_public_policy_repo_accessible(){
  local policy_url="${1-}"
  local target_ref="${2-}"
  local out rc lower detail

  if (( DRY_RUN )); then
    info "[dry-run] validate public policy repo: $policy_url @ $target_ref"
    return 0
  fi

  set +e
  out="$(GIT_TERMINAL_PROMPT=0 git ls-remote --symref "$policy_url" HEAD 2>&1)"
  rc=$?
  set -e
  if (( rc != 0 )); then
    lower="${out,,}"
    detail="$(summarize_cmd_error "$out")"
    if [[ "$lower" == *"could not read username"* || "$lower" == *"authentication failed"* ]]; then
      detail="GitHub requested credentials for $policy_url. This usually means the repo is private or the owner/repo is misspelled."
    elif [[ "$lower" == *"repository not found"* || "$lower" == *"not found"* ]]; then
      detail="GitHub could not find $policy_url. Check the owner/repo spelling."
    fi
    die "Configured Saturn Go repo $policy_url is not publicly reachable over HTTPS. Anonymous/self-update flows require a public GitHub repo. $detail"
  fi

  if [[ -n "$target_ref" && ! "$target_ref" =~ ^[0-9a-fA-F]{7,40}$ ]]; then
    set +e
    out="$(GIT_TERMINAL_PROMPT=0 git ls-remote --exit-code "$policy_url" "$target_ref" 2>&1)"
    rc=$?
    set -e
    if (( rc != 0 )); then
      detail="$(summarize_cmd_error "$out")"
      die "Configured Saturn Go ref '$target_ref' was not found in public repo $policy_url. $detail"
    fi
  fi
}

need_cmd(){
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

resolve_user_home(){
  local user="${1:-$(id -un)}"
  if [[ -n "${HOME:-}" && "${HOME}" != "/" && -d "${HOME}" ]]; then
    printf '%s\n' "$HOME"
    return 0
  fi

  local passwd_home
  passwd_home="$(getent passwd "$user" | cut -d: -f6)"
  if [[ -n "$passwd_home" && -d "$passwd_home" ]]; then
    printf '%s\n' "$passwd_home"
    return 0
  fi

  die "Could not resolve home directory for user: $user"
}

resolve_cargo_bin(){
  if [[ -n "${SATURN_SATURNGO_CARGO_BIN:-}" ]]; then
    [[ -x "${SATURN_SATURNGO_CARGO_BIN}" ]] || die "SATURN_SATURNGO_CARGO_BIN is not executable: ${SATURN_SATURNGO_CARGO_BIN}"
    printf '%s\n' "${SATURN_SATURNGO_CARGO_BIN}"
    return 0
  fi

  local candidate=""
  candidate="$(command -v cargo 2>/dev/null || true)"
  if [[ -n "$candidate" && -x "$candidate" ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi

  local build_user build_home
  build_user="${SATURN_SATURNGO_BUILD_USER:-$(id -un)}"
  build_home="$(resolve_user_home "$build_user")"
  for candidate in \
    "$build_home/.cargo/bin/cargo" \
    "/home/$build_user/.cargo/bin/cargo"
  do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  die "Required command not found: cargo (checked PATH and $build_home/.cargo/bin)"
}

active_swap_bytes(){
  local path="$1"
  swapon --show=NAME,SIZE --bytes --noheadings 2>/dev/null \
    | awk -v target="$path" '$1 == target { print $2; found=1; exit } END { if (!found) print 0 }'
}

ensure_build_swap(){
  local min_bytes active_bytes helper
  min_bytes=$(( BUILD_SWAP_MIB * 1024 * 1024 ))
  if (( min_bytes > 1048576 )); then
    min_bytes=$(( min_bytes - 1048576 ))
  fi
  active_bytes="$(active_swap_bytes "$BUILD_SWAP_FILE")"

  if (( active_bytes >= min_bytes )); then
    info "Build swap active: $BUILD_SWAP_FILE ($(( active_bytes / 1024 / 1024 )) MiB)"
    return 0
  fi

  if (( DRY_RUN )); then
    info "[dry-run] ensure build swap: $BUILD_SWAP_FILE (${BUILD_SWAP_MIB} MiB)"
    return 0
  fi

  if (( EUID == 0 )); then
    helper="$BUILD_PREFLIGHT_HELPER"
    [[ -x "$helper" ]] || die "Build preflight helper is not executable: $helper"
    "$helper" ensure-swap
  elif [[ -x "$DEPLOY_BUILD_PREFLIGHT_HELPER" ]]; then
    run_cmd sudo -n "$DEPLOY_BUILD_PREFLIGHT_HELPER" ensure-swap
  else
    die "Build swap is not active and the privileged helper is not installed: $DEPLOY_BUILD_PREFLIGHT_HELPER"
  fi

  active_bytes="$(active_swap_bytes "$BUILD_SWAP_FILE")"
  if (( active_bytes < min_bytes )); then
    die "Build swap is still not active at ${BUILD_SWAP_MIB} MiB: $BUILD_SWAP_FILE"
  fi
}

cargo_build_prefix_display(){
  printf 'CARGO_BUILD_JOBS=%s TMPDIR=%s CARGO_TARGET_DIR=%s nice -n %s ionice -c %s' \
    "$BUILD_JOBS" "$BUILD_TMP_DIR" "$BUILD_TARGET_DIR" "$BUILD_NICE" "$BUILD_IONICE_CLASS"
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
need_cmd systemd-run

BUILD_USER="${SATURN_SATURNGO_BUILD_USER:-$(id -un)}"
BUILD_HOME="$(resolve_user_home "$BUILD_USER")"
if [[ -z "${HOME:-}" || "${HOME}" == "/" ]]; then
  export HOME="$BUILD_HOME"
fi
export CARGO_HOME="${CARGO_HOME:-$BUILD_HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$BUILD_HOME/.rustup}"
CARGO_BIN="$(resolve_cargo_bin)"
export PATH="$(dirname "$CARGO_BIN"):$PATH"

RUN_STARTED_AT="$(date -Is)"
REPO_ROOT="${SATURN_ACTIVE_REPO_ROOT:-${SATURN_REPO_ROOT:-}}"
[[ -n "$REPO_ROOT" ]] || die "SATURN_ACTIVE_REPO_ROOT is not set"
[[ -d "$REPO_ROOT/.git" ]] || die "Repo root is not a git checkout: $REPO_ROOT"
[[ -d "$REPO_ROOT/update_manager" ]] || die "Repo root does not contain update_manager/: $REPO_ROOT"
XDMA_FIX_SCRIPT_SRC="$REPO_ROOT/scripts/fix-xdma.sh"
XDMA_POSTINST_HELPER_SRC="$REPO_ROOT/scripts/saturn-xdma-kernel-postinst.sh"
XDMA_FIX_SCRIPT_INSTALL="${SATURN_XDMA_FIX_SCRIPT_INSTALL:-/usr/local/bin/saturn-fix-xdma.sh}"
XDMA_POSTINST_HELPER_INSTALL="${SATURN_XDMA_POSTINST_HELPER_INSTALL:-/usr/local/bin/saturn-xdma-kernel-postinst.sh}"
XDMA_POSTINST_HOOK_PATH="${SATURN_XDMA_POSTINST_HOOK_PATH:-/etc/kernel/postinst.d/saturn-xdma}"
EXTRA_PACKAGED_SCRIPTS=(
  "$REPO_ROOT/scripts/fix-LED-power-button.sh"
  "$REPO_ROOT/scripts/install-shutdown-waiter-service.sh"
  "$REPO_ROOT/scripts/shutdown-waiter.sh"
  "$REPO_ROOT/scripts/setup-eth-fallback.sh"
)
PRIVILEGED_HELPER_SCRIPTS=(
  "$REPO_ROOT/update_manager/scripts/saturn-go-build-preflight.sh"
  "$REPO_ROOT/scripts/saturn-admin-password.sh"
  "$REPO_ROOT/scripts/saturn-flash-fpga.sh"
  "$REPO_ROOT/scripts/saturn-xdma-doctor.sh"
  "$REPO_ROOT/scripts/saturn-xdma-stage-current.sh"
  "$REPO_ROOT/scripts/deskhpsdr-install-deps-on-current-image.sh"
  "$XDMA_FIX_SCRIPT_SRC"
  "$XDMA_POSTINST_HELPER_SRC"
  "$REPO_ROOT/scripts/fix-LED-power-button.sh"
  "$REPO_ROOT/scripts/install-shutdown-waiter-service.sh"
  "$REPO_ROOT/scripts/shutdown-waiter.sh"
  "$REPO_ROOT/scripts/setup-eth-fallback.sh"
  "$REPO_ROOT/update_manager/scripts/saturn-tailscale.sh"
  "$REPO_ROOT/update_manager/scripts/saturn-go-tailscale-serve.sh"
)

SATURNGO_URL="${SATURN_SATURNGO_POLICY_URL:-}"
SATURNGO_REMOTE="${SATURN_SATURNGO_POLICY_REMOTE:-origin}"
SATURNGO_REF="${SATURN_SATURNGO_POLICY_REF:-main}"

RUST_DIR="$REPO_ROOT/update_manager/rust-server"
TEMPLATES_DIR="$REPO_ROOT/update_manager/templates"
SCRIPTS_SRC_DIR="$REPO_ROOT/update_manager/scripts"
WEB_ASSET_HELPERS="$SCRIPTS_SRC_DIR/saturn-go-web-assets.sh"
BUILD_TMP_DIR="$RUST_DIR/.tmp"
BUILD_TARGET_DIR="$RUST_DIR/target-local"
BUILD_PREFLIGHT_HELPER="$SCRIPTS_SRC_DIR/saturn-go-build-preflight.sh"
BUILD_SWAP_FILE="${SATURN_SATURNGO_BUILD_SWAP_FILE:-/home/pi/saturn-build.swap}"
BUILD_SWAP_MIB="${SATURN_SATURNGO_BUILD_SWAP_MIB:-2048}"
BUILD_JOBS="${SATURN_SATURNGO_BUILD_JOBS:-1}"
BUILD_NICE="${SATURN_SATURNGO_BUILD_NICE:-15}"
BUILD_IONICE_CLASS="${SATURN_SATURNGO_BUILD_IONICE_CLASS:-3}"

SERVICE_NAME="${SATURN_SATURNGO_SERVICE_NAME:-saturn-go.service}"
DEPLOY_RUN_USER="${SATURN_SATURNGO_RUN_USER:-$(id -un)}"
DEPLOY_BIN="${SATURN_SATURNGO_BIN_DEST:-/opt/saturn-go/bin/saturn-go}"
DEPLOY_WEBROOT="${SATURN_SATURNGO_WEBROOT:-/var/lib/saturn-web}"
DEPLOY_SCRIPTS_DIR="${SATURN_SATURNGO_SCRIPTS_DIR:-/opt/saturn-go/scripts}"
DEPLOY_PRIVILEGED_SCRIPTS_DIR="${SATURN_SATURNGO_PRIVILEGED_SCRIPTS_DIR:-/usr/local/lib/saturn-go/scripts}"
DEPLOY_BUILD_PREFLIGHT_HELPER="$DEPLOY_PRIVILEGED_SCRIPTS_DIR/saturn-go-build-preflight.sh"
DEPLOY_SUDOERS_FILE="${SATURN_SATURNGO_SUDOERS_FILE:-/etc/sudoers.d/saturn-go-maintenance}"
STATUS_FILE="${SATURN_SATURNGO_DEPLOY_STATUS_FILE:-/var/lib/saturn-state/saturngo_deploy_status.json}"

trap 'rc=$?; if (( rc != 0 )); then write_status "error" "$STATUS_PHASE" "Saturn Go self-update failed" 1 "$rc"; fi' EXIT

[[ -f "$RUST_DIR/Cargo.toml" ]] || die "Rust server Cargo.toml not found: $RUST_DIR/Cargo.toml"
[[ -d "$TEMPLATES_DIR" ]] || die "Templates dir not found: $TEMPLATES_DIR"
[[ -d "$SCRIPTS_SRC_DIR" ]] || die "Scripts dir not found: $SCRIPTS_SRC_DIR"
[[ -f "$WEB_ASSET_HELPERS" ]] || die "Web asset helper not found: $WEB_ASSET_HELPERS"
[[ -f "$BUILD_PREFLIGHT_HELPER" ]] || die "Build preflight helper not found: $BUILD_PREFLIGHT_HELPER"
source "$WEB_ASSET_HELPERS"
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
  ensure_public_policy_repo_accessible "$SATURNGO_URL" "$SATURNGO_REF"
  info "Using public policy repo directly for anonymous-safe Saturn Go update"
  run_git_no_prompt git -C "$REPO_ROOT" fetch --prune "$SATURNGO_URL" "$SATURNGO_REF"
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
  ensure_build_swap
  info "Build settings: $(cargo_build_prefix_display) $CARGO_BIN build --release -j$BUILD_JOBS"
  run_cmd mkdir -p "$BUILD_TMP_DIR" "$BUILD_TARGET_DIR"
  if (( DRY_RUN )); then
    info "[dry-run] $(cargo_build_prefix_display) $CARGO_BIN build --release -j$BUILD_JOBS (cwd=$RUST_DIR)"
  else
    (
      cd "$RUST_DIR"
      export CARGO_BUILD_JOBS="$BUILD_JOBS"
      TMPDIR="$BUILD_TMP_DIR" \
      CARGO_TARGET_DIR="$BUILD_TARGET_DIR" \
      nice -n "$BUILD_NICE" ionice -c "$BUILD_IONICE_CLASS" "$CARGO_BIN" build --release -j "$BUILD_JOBS"
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

STAGE_BASE="${SATURN_SATURNGO_STAGE_BASE:-${SATURN_STAGING_DIR:-/var/lib/saturn-state/repo-staging}/saturngo-deploy-stage}"
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
  saturn_go_build_remote_web_assets "$REPO_ROOT/update_manager"
  saturn_go_copy_required_web_assets "$TEMPLATES_DIR" "$REPO_ROOT/update_manager" "$STAGE_WEB_DIR"
  saturn_go_copy_optional_web_assets "$TEMPLATES_DIR" "$REPO_ROOT/update_manager" "$STAGE_WEB_DIR"
  saturn_go_verify_remote_web_bundle "$STAGE_WEB_DIR"
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
  info "[dry-run] stage binary, web assets, packaged scripts, privileged helpers, and XDMA postinst hook under $STAGE_DIR"
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
XDMA_FIX_SCRIPT_INSTALL="$(printf '%s' "$XDMA_FIX_SCRIPT_INSTALL")"
XDMA_POSTINST_HELPER_INSTALL="$(printf '%s' "$XDMA_POSTINST_HELPER_INSTALL")"
XDMA_POSTINST_HOOK_PATH="$(printf '%s' "$XDMA_POSTINST_HOOK_PATH")"
STARTED_AT="$(printf '%s' "$RUN_STARTED_AT")"
SCRIPT_UID="\$(id -u "$DEPLOY_RUN_USER")"
SCRIPT_GID="\$(id -g "$DEPLOY_RUN_USER")"
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
  chown "\$SCRIPT_UID:\$SCRIPT_GID" "\$STATUS_FILE" >/dev/null 2>&1 || true
  chmod 0640 "\$STATUS_FILE" >/dev/null 2>&1 || true
}
trap 'rc=\$?; write_status "error" "root-deploy" "Root deploy helper failed" 1 "\$rc"; exit "\$rc"' ERR
write_status "running" "root-deploy" "Root deploy helper started"
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
[[ -f "$STAGE_PRIVILEGED_SCRIPTS_DIR/fix-xdma.sh" ]] || { echo "Missing staged XDMA helper: $STAGE_PRIVILEGED_SCRIPTS_DIR/fix-xdma.sh" >&2; exit 1; }
[[ -f "$STAGE_PRIVILEGED_SCRIPTS_DIR/saturn-xdma-kernel-postinst.sh" ]] || { echo "Missing staged XDMA postinst helper: $STAGE_PRIVILEGED_SCRIPTS_DIR/saturn-xdma-kernel-postinst.sh" >&2; exit 1; }
install -D -m 0755 -o root -g root "$STAGE_PRIVILEGED_SCRIPTS_DIR/fix-xdma.sh" "\$XDMA_FIX_SCRIPT_INSTALL"
install -D -m 0755 -o root -g root "$STAGE_PRIVILEGED_SCRIPTS_DIR/saturn-xdma-kernel-postinst.sh" "\$XDMA_POSTINST_HELPER_INSTALL"
TMP_XDMA_HOOK="\$(mktemp)"
cat >"\$TMP_XDMA_HOOK" <<HOOK
#!/bin/sh
set -eu
HELPER="${XDMA_POSTINST_HELPER_INSTALL}"
if [ -x "\$HELPER" ]; then
  "\$HELPER" "\$@" || true
fi
exit 0
HOOK
if [[ ! -f "\$XDMA_POSTINST_HOOK_PATH" ]] || ! cmp -s "\$TMP_XDMA_HOOK" "\$XDMA_POSTINST_HOOK_PATH"; then
  install -D -m 0755 -o root -g root "\$TMP_XDMA_HOOK" "\$XDMA_POSTINST_HOOK_PATH"
fi
rm -f "\$TMP_XDMA_HOOK"
[[ -x "\$XDMA_FIX_SCRIPT_INSTALL" ]] || { echo "Installed XDMA fix helper is not executable: \$XDMA_FIX_SCRIPT_INSTALL" >&2; exit 1; }
[[ -x "\$XDMA_POSTINST_HELPER_INSTALL" ]] || { echo "Installed XDMA postinst helper is not executable: \$XDMA_POSTINST_HELPER_INSTALL" >&2; exit 1; }
[[ -x "\$XDMA_POSTINST_HOOK_PATH" ]] || { echo "Installed XDMA postinst hook is not executable: \$XDMA_POSTINST_HOOK_PATH" >&2; exit 1; }
grep -Fq "\$XDMA_POSTINST_HELPER_INSTALL" "\$XDMA_POSTINST_HOOK_PATH" || { echo "XDMA postinst hook does not reference helper: \$XDMA_POSTINST_HELPER_INSTALL" >&2; exit 1; }
cat >"$DEPLOY_SUDOERS_FILE" <<SUDOERS
# Managed by update-saturn-go.sh
Defaults:${DEPLOY_RUN_USER} secure_path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/install-shutdown-waiter-service.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/install-shutdown-waiter-service.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/setup-eth-fallback.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/setup-eth-fallback.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/fix-LED-power-button.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/fix-LED-power-button.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-xdma-doctor.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-xdma-doctor.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-flash-fpga.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-flash-fpga.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-xdma-stage-current.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/deskhpsdr-install-deps-on-current-image.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/deskhpsdr-install-deps-on-current-image.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-admin-password.sh set
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-admin-password.sh status
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-go-build-preflight.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-go-build-preflight.sh ensure-swap
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-go-build-preflight.sh status
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-tailscale.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-tailscale.sh *
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-go-tailscale-serve.sh
${DEPLOY_RUN_USER} ALL=(root) NOPASSWD: ${DEPLOY_PRIVILEGED_SCRIPTS_DIR}/saturn-go-tailscale-serve.sh *
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
