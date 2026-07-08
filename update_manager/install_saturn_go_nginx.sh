#!/usr/bin/env bash
set -euo pipefail

SATURN_ROOT="/opt/saturn-go"
BIN_DIR="$SATURN_ROOT/bin"
SCRIPTS_DIR="$SATURN_ROOT/scripts"
WATCHDOG_SCRIPT_DIR="/usr/local/lib/saturn-go"
PRIVILEGED_SCRIPTS_DIR="$WATCHDOG_SCRIPT_DIR/scripts"
WEB_ROOT="/var/lib/saturn-web"
NGINX_SITE="/etc/nginx/sites-available/saturn"
NGINX_SITE_LINK="/etc/nginx/sites-enabled/saturn"
NGINX_SSE_MAP="/etc/nginx/conf.d/saturn_sse_map.conf"
BASIC_AUTH_FILE="/etc/nginx/.htpasswd"
SERVICE_FILE="/etc/systemd/system/saturn-go.service"
WATCHDOG_SCRIPT_NAME="saturn-health-watchdog.sh"
WATCHDOG_SCRIPT_PATH="$WATCHDOG_SCRIPT_DIR/$WATCHDOG_SCRIPT_NAME"
WATCHDOG_SERVICE_FILE="/etc/systemd/system/saturn-go-watchdog.service"
WATCHDOG_TIMER_FILE="/etc/systemd/system/saturn-go-watchdog.timer"
SUDOERS_FILE="/etc/sudoers.d/saturn-go-maintenance"
SOURCE_DIR="/home/${SUDO_USER:-$USER}/github/Saturn/update_manager"
RUST_SRC_DIR="$SOURCE_DIR/rust-server"
WEB_ASSET_HELPERS="$SOURCE_DIR/scripts/saturn-go-web-assets.sh"
BUILD_PREFLIGHT_HELPER="$SOURCE_DIR/scripts/saturn-go-build-preflight.sh"
XDMA_FIX_SCRIPT_INSTALL="/usr/local/bin/saturn-fix-xdma.sh"
XDMA_POSTINST_HELPER_INSTALL="/usr/local/bin/saturn-xdma-kernel-postinst.sh"
XDMA_POSTINST_HOOK_PATH="/etc/kernel/postinst.d/saturn-xdma"
SATURN_BRIDGE_INSTALLER_NAME="install-saturn-bridge.sh"

SATURN_ADDR="${SATURN_ADDR:-127.0.0.1:8080}"
SATURN_MAX_BODY_BYTES="${SATURN_MAX_BODY_BYTES:-2147483648}"
SATURN_RESTORE_MAX_UPLOAD_BYTES="${SATURN_RESTORE_MAX_UPLOAD_BYTES:-2147483648}"
SATURN_NGINX_CLIENT_MAX_BODY_SIZE="${SATURN_NGINX_CLIENT_MAX_BODY_SIZE:-2G}"
SATURN_STATE_DIR="${SATURN_STATE_DIR:-/var/lib/saturn-state}"
SATURN_REPO_ROOT_FILE="${SATURN_REPO_ROOT_FILE:-${SATURN_STATE_DIR}/repo_root.txt}"
SATURN_UPDATE_POLICY_FILE="${SATURN_UPDATE_POLICY_FILE:-${SATURN_STATE_DIR}/update_policy.json}"
SATURN_SATURNGO_UPDATE_POLICY_FILE="${SATURN_SATURNGO_UPDATE_POLICY_FILE:-${SATURN_STATE_DIR}/saturngo_update_policy.json}"
SATURN_SATURNGO_DEPLOY_STATUS_FILE="${SATURN_SATURNGO_DEPLOY_STATUS_FILE:-${SATURN_STATE_DIR}/saturngo_deploy_status.json}"
SATURN_UPDATE_STATE_FILE="${SATURN_UPDATE_STATE_FILE:-${SATURN_STATE_DIR}/update_state.json}"
SATURN_SNAPSHOT_DIR="${SATURN_SNAPSHOT_DIR:-${SATURN_STATE_DIR}/snapshots}"
SATURN_STAGING_DIR="${SATURN_STAGING_DIR:-${SATURN_STATE_DIR}/repo-staging}"
SATURN_WATCHDOG_URL="${SATURN_WATCHDOG_URL:-http://${SATURN_ADDR}/healthz}"
SATURN_WATCHDOG_INTERVAL="${SATURN_WATCHDOG_INTERVAL:-30s}"
RUSTUP_INIT_URL="${RUSTUP_INIT_URL:-https://sh.rustup.rs}"
TAILSCALE_INSTALL_URL="${TAILSCALE_INSTALL_URL:-https://tailscale.com/install.sh}"
SATURN_REMOTE_NEXT_DEFAULT_QUERY="${SATURN_REMOTE_NEXT_DEFAULT_QUERY:-phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1&client_bust=bridgeprefill240-cfcessb3}"
SATURN_INSTALL_BRIDGE="${SATURN_INSTALL_BRIDGE:-0}"
SATURN_REQUIRE_BRIDGE="${SATURN_REQUIRE_BRIDGE:-0}"
SATURN_PIHPSDR_DIR="${SATURN_PIHPSDR_DIR:-/home/${SUDO_USER:-$USER}/github/pihpsdr}"

bold(){ printf "\e[1m%s\e[0m\n" "$*"; }
ok(){   printf "[OK] %s\n" "$*"; }
info(){ printf "[INFO] %s\n" "$*"; }
warn(){ printf "[WARN] %s\n" "$*"; }
err(){  printf "[ERR] %s\n" "$*" >&2; }

check_tmp_space_preflight() {
  local warn_pct="${SATURN_TMP_WARN_PCT:-90}"
  local warn_min_kb="${SATURN_TMP_WARN_MIN_KB:-131072}" # 128 MiB
  local avail_kb used_pct mount_point

  if ! read -r avail_kb used_pct mount_point < <(
    df -Pk /tmp 2>/dev/null | awk 'NR==2 { gsub(/%/, "", $5); print $4, $5, $6 }'
  ); then
    return 0
  fi

  [[ -n "${avail_kb:-}" && -n "${used_pct:-}" ]] || return 0

  if (( used_pct >= warn_pct || avail_kb <= warn_min_kb )); then
    warn "/tmp is low on space before apt update (mount=${mount_point:-/tmp}, used=${used_pct}%, avail=${avail_kb}KB)"
    warn "This can cause apt signature verification/download failures (for example 'Splitting up ... InRelease' or 'No space left on device')."
    warn "Consider cleaning /tmp and retrying if install fails."
  fi
}

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  err "Run as root (sudo)."
  exit 1
fi

if [[ ! -d "$SOURCE_DIR" ]]; then
  err "Source directory not found: $SOURCE_DIR"
  exit 1
fi
REPO_SOURCE_DIR="$(cd "$SOURCE_DIR/.." && pwd)"
XDMA_FIX_SCRIPT_SRC="$REPO_SOURCE_DIR/scripts/fix-xdma.sh"
XDMA_POSTINST_HELPER_SRC="$REPO_SOURCE_DIR/scripts/saturn-xdma-kernel-postinst.sh"
EXTRA_PACKAGED_SCRIPTS=(
  "$REPO_SOURCE_DIR/scripts/fix-LED-power-button.sh"
  "$REPO_SOURCE_DIR/scripts/install-shutdown-waiter-service.sh"
  "$REPO_SOURCE_DIR/scripts/shutdown-waiter.sh"
  "$REPO_SOURCE_DIR/scripts/setup-eth-fallback.sh"
)
PRIVILEGED_HELPER_SCRIPTS=(
  "$SOURCE_DIR/scripts/saturn-go-build-preflight.sh"
  "$SOURCE_DIR/scripts/$SATURN_BRIDGE_INSTALLER_NAME"
  "$REPO_SOURCE_DIR/scripts/saturn-admin-password.sh"
  "$REPO_SOURCE_DIR/scripts/saturn-flash-fpga.sh"
  "$REPO_SOURCE_DIR/scripts/saturn-xdma-doctor.sh"
  "$REPO_SOURCE_DIR/scripts/saturn-xdma-stage-current.sh"
  "$XDMA_FIX_SCRIPT_SRC"
  "$XDMA_POSTINST_HELPER_SRC"
  "$REPO_SOURCE_DIR/scripts/fix-LED-power-button.sh"
  "$REPO_SOURCE_DIR/scripts/install-shutdown-waiter-service.sh"
  "$REPO_SOURCE_DIR/scripts/shutdown-waiter.sh"
  "$REPO_SOURCE_DIR/scripts/setup-eth-fallback.sh"
  "$SOURCE_DIR/scripts/saturn-tailscale.sh"
  "$SOURCE_DIR/scripts/saturn-go-tailscale-serve.sh"
)
if [[ ! -f "$RUST_SRC_DIR/Cargo.toml" ]]; then
  err "Rust server source not found: $RUST_SRC_DIR"
  exit 1
fi
if [[ ! -f "$WEB_ASSET_HELPERS" ]]; then
  err "Web asset helper not found: $WEB_ASSET_HELPERS"
  exit 1
fi
if [[ ! -f "$BUILD_PREFLIGHT_HELPER" ]]; then
  err "Build preflight helper not found: $BUILD_PREFLIGHT_HELPER"
  exit 1
fi
source "$WEB_ASSET_HELPERS"
for extra_script in "${EXTRA_PACKAGED_SCRIPTS[@]}"; do
  if [[ ! -f "$extra_script" ]]; then
    err "Extra packaged script not found: $extra_script"
    exit 1
  fi
done
for privileged_script in "${PRIVILEGED_HELPER_SCRIPTS[@]}"; do
  if [[ ! -f "$privileged_script" ]]; then
    err "Privileged helper script not found: $privileged_script"
    exit 1
  fi
done

# Pick a non-root service user by default.
if [[ -n "${SATURN_SERVICE_USER:-}" ]]; then
  SERVICE_USER="$SATURN_SERVICE_USER"
elif [[ -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]]; then
  SERVICE_USER="$SUDO_USER"
elif id -u pi >/dev/null 2>&1; then
  SERVICE_USER="pi"
else
  err "Set SATURN_SERVICE_USER to a valid non-root user."
  exit 1
fi
SERVICE_GROUP="${SATURN_SERVICE_GROUP:-$SERVICE_USER}"

if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
  err "Service user does not exist: $SERVICE_USER"
  exit 1
fi
if ! getent group "$SERVICE_GROUP" >/dev/null 2>&1; then
  err "Service group does not exist: $SERVICE_GROUP"
  exit 1
fi

SERVICE_HOME="$(getent passwd "$SERVICE_USER" | cut -d: -f6)"
if [[ -z "$SERVICE_HOME" || ! -d "$SERVICE_HOME" ]]; then
  err "Cannot resolve home directory for $SERVICE_USER"
  exit 1
fi
DEFAULT_REPO_ROOT="${SATURN_REPO_ROOT:-$SERVICE_HOME/github/Saturn}"

if [[ -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]] && id -u "${SUDO_USER}" >/dev/null 2>&1; then
  BUILD_USER="${SUDO_USER}"
else
  BUILD_USER="$SERVICE_USER"
fi
BUILD_GROUP="$(id -gn "$BUILD_USER")"
BUILD_HOME="$(getent passwd "$BUILD_USER" | cut -d: -f6)"
if [[ -z "$BUILD_HOME" || ! -d "$BUILD_HOME" ]]; then
  err "Cannot resolve build user home directory for $BUILD_USER"
  exit 1
fi
RUSTUP_BIN_DIR="$BUILD_HOME/.cargo/bin"
RUSTUP_CARGO_BIN="$RUSTUP_BIN_DIR/cargo"
RUSTUP_RUSTC_BIN="$RUSTUP_BIN_DIR/rustc"
RUSTUP_CMD_BIN="$RUSTUP_BIN_DIR/rustup"
RUST_BUILD_TMP_DIR="$RUST_SRC_DIR/.tmp"
RUST_BUILD_TARGET_DIR="$RUST_SRC_DIR/target-local"
RUST_BUILD_SWAP_FILE="${SATURN_SATURNGO_BUILD_SWAP_FILE:-/home/pi/saturn-build.swap}"
RUST_BUILD_SWAP_MIB="${SATURN_SATURNGO_BUILD_SWAP_MIB:-2048}"
RUST_BUILD_JOBS="${SATURN_SATURNGO_BUILD_JOBS:-1}"
RUST_BUILD_NICE="${SATURN_SATURNGO_BUILD_NICE:-15}"
RUST_BUILD_IONICE_CLASS="${SATURN_SATURNGO_BUILD_IONICE_CLASS:-3}"

run_as_build_user() {
  local cmd="$1"
  local shell_cmd="export HOME=\"$BUILD_HOME\"; export PATH=\"$RUSTUP_BIN_DIR:\$PATH\"; $cmd"
  if [[ "$BUILD_USER" == "root" ]]; then
    bash -lc "$shell_cmd"
  else
    runuser -u "$BUILD_USER" -- bash -lc "$shell_cmd"
  fi
}

install_xdma_kernel_postinst_hook() {
  local tmp_hook

  info "Installing XDMA kernel postinst helpers..."
  install -D -m 0755 -o root -g root "$XDMA_FIX_SCRIPT_SRC" "$XDMA_FIX_SCRIPT_INSTALL"
  install -D -m 0755 -o root -g root "$XDMA_POSTINST_HELPER_SRC" "$XDMA_POSTINST_HELPER_INSTALL"

  tmp_hook="$(mktemp)"
  cat >"$tmp_hook" <<EOF
#!/bin/sh
set -eu
HELPER="${XDMA_POSTINST_HELPER_INSTALL}"
if [ -x "\$HELPER" ]; then
  "\$HELPER" "\$@" || true
fi
exit 0
EOF

  if [[ ! -f "$XDMA_POSTINST_HOOK_PATH" ]] || ! cmp -s "$tmp_hook" "$XDMA_POSTINST_HOOK_PATH"; then
    install -D -m 0755 -o root -g root "$tmp_hook" "$XDMA_POSTINST_HOOK_PATH"
    ok "XDMA kernel postinst hook installed at $XDMA_POSTINST_HOOK_PATH"
  else
    ok "XDMA kernel postinst hook already current"
  fi
  rm -f "$tmp_hook"
}

apt_pkg_installed() {
  dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q '^install ok installed$'
}

env_flag_enabled() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

install_optional_tailscale() {
  if ! env_flag_enabled "${SATURN_INSTALL_TAILSCALE:-}"; then
    return 0
  fi

  if command -v tailscale >/dev/null 2>&1; then
    ok "Tailscale CLI already installed"
    return 0
  fi

  info "Installing Tailscale package via ${TAILSCALE_INSTALL_URL}"
  curl -fsSL "$TAILSCALE_INSTALL_URL" | sh
  if ! command -v tailscale >/dev/null 2>&1; then
    err "Tailscale installer completed but the tailscale CLI is not on PATH."
    exit 1
  fi
  ok "Tailscale package installed"
}

bridge_requested() {
  env_flag_enabled "$SATURN_INSTALL_BRIDGE" || env_flag_enabled "$SATURN_REQUIRE_BRIDGE"
}

saturn_remote_bridge_preflight() {
  local bridge_dir="$SOURCE_DIR/saturn-bridge"
  local bridge_installer="$SOURCE_DIR/scripts/$SATURN_BRIDGE_INSTALLER_NAME"
  local missing=()

  info "Checking Saturn Remote bridge prerequisites..."
  [[ -f "$bridge_dir/Cargo.toml" ]] || missing+=("$bridge_dir/Cargo.toml")
  [[ -x "$bridge_installer" ]] || missing+=("$bridge_installer")
  command -v "$RUSTUP_CARGO_BIN" >/dev/null 2>&1 || [[ -x "$RUSTUP_CARGO_BIN" ]] || missing+=("$RUSTUP_CARGO_BIN")
  apt_pkg_installed libfftw3-dev || missing+=("apt:libfftw3-dev")
  [[ -f "$SATURN_PIHPSDR_DIR/wdsp/libwdsp.a" ]] || missing+=("$SATURN_PIHPSDR_DIR/wdsp/libwdsp.a")
  [[ -f "$SATURN_PIHPSDR_DIR/rnnoise/librnnoise.a" ]] || missing+=("$SATURN_PIHPSDR_DIR/rnnoise/librnnoise.a")
  [[ -f "$SATURN_PIHPSDR_DIR/libspecbleach/libspecbleach.a" ]] || missing+=("$SATURN_PIHPSDR_DIR/libspecbleach/libspecbleach.a")

  if (( ${#missing[@]} == 0 )); then
    ok "Saturn Remote bridge prerequisites present"
    return 0
  fi

  warn "Saturn Remote bridge prerequisites are missing:"
  printf '  - %s\n' "${missing[@]}" >&2
  warn "Remote pages can load, but /remote and /remote-next will not work until saturn-bridge is installed."
  warn "Install/build piHPSDR first, then run: sudo SATURN_INSTALL_BRIDGE=1 bash update_manager/install_saturn_go_nginx.sh"

  if env_flag_enabled "$SATURN_REQUIRE_BRIDGE"; then
    err "SATURN_REQUIRE_BRIDGE=1 and Saturn Remote bridge prerequisites are missing."
    exit 1
  fi
  return 1
}

install_saturn_bridge_if_requested() {
  bridge_requested || return 0
  saturn_remote_bridge_preflight

  info "Installing Saturn Remote bridge..."
  env \
    SATURN_USER="$SERVICE_USER" \
    SATURN_REPO_ROOT="$REPO_SOURCE_DIR" \
    SATURN_PIHPSDR_DIR="$SATURN_PIHPSDR_DIR" \
    SATURN_BRIDGE_SOURCE_DIR="$SOURCE_DIR/saturn-bridge" \
    SATURN_GO_ROOT="$SATURN_ROOT" \
    SATURN_BRIDGE_BIN="$BIN_DIR/saturn-bridge" \
    SATURN_BRIDGE_RF_TX_ENABLED="${SATURN_BRIDGE_RF_TX_ENABLED:-1}" \
    SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED="${SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED:-1}" \
    bash "$SOURCE_DIR/scripts/$SATURN_BRIDGE_INSTALLER_NAME"
  ok "Saturn Remote bridge installed"
}

remove_legacy_apt_rust() {
  local pkgs=()
  apt_pkg_installed cargo && pkgs+=(cargo)
  apt_pkg_installed rustc && pkgs+=(rustc)
  if [[ ${#pkgs[@]} -eq 0 ]]; then
    return 0
  fi
  info "Removing distro Rust packages (using rustup-managed toolchain instead): ${pkgs[*]}"
  apt-get purge -y -qq "${pkgs[@]}" || warn "Could not fully purge legacy cargo/rustc packages; continuing"
  apt-get autoremove -y -qq >/dev/null 2>&1 || true
}

cargo_lock_preflight() {
  local err_file
  err_file="$(mktemp)"
  if run_as_build_user "\"$RUSTUP_CARGO_BIN\" metadata --format-version 1 --locked --no-deps --manifest-path \"$RUST_SRC_DIR/Cargo.toml\"" \
    >/dev/null 2>"$err_file"; then
    rm -f "$err_file"
    return 0
  fi
  if grep -q 'lock file version `4`' "$err_file"; then
    rm -f "$err_file"
    return 2
  fi
  err "Cargo preflight failed:"
  sed 's/^/  /' "$err_file" >&2 || true
  rm -f "$err_file"
  return 1
}

ensure_modern_rust_toolchain() {
  remove_legacy_apt_rust

  if [[ ! -x "$RUSTUP_CARGO_BIN" || ! -x "$RUSTUP_RUSTC_BIN" || ! -x "$RUSTUP_CMD_BIN" ]]; then
    info "Installing rustup toolchain for build user '$BUILD_USER'..."
    run_as_build_user "curl --proto '=https' --tlsv1.2 -sSf \"$RUSTUP_INIT_URL\" | sh -s -- -y --profile minimal --default-toolchain stable"
  else
    info "rustup already installed for build user '$BUILD_USER'; updating stable toolchain..."
    run_as_build_user "\"$RUSTUP_CMD_BIN\" self update >/dev/null 2>&1 || true"
  fi

  run_as_build_user "\"$RUSTUP_CMD_BIN\" toolchain install stable --profile minimal"
  run_as_build_user "\"$RUSTUP_CMD_BIN\" default stable"

  local rc=0
  if cargo_lock_preflight; then
    rc=0
  else
    rc=$?
  fi
  if [[ $rc -ne 0 ]]; then
    if [[ $rc -eq 2 ]]; then
      err "Rust toolchain is still too old for Cargo.lock version 4 after rustup bootstrap."
      exit 1
    fi
    exit 1
  fi

  info "Using Rust toolchain (build user: $BUILD_USER)"
  info "  $(run_as_build_user "\"$RUSTUP_CARGO_BIN\" --version")"
  info "  $(run_as_build_user "\"$RUSTUP_RUSTC_BIN\" --version")"
}

info "Installing dependencies..."
check_tmp_space_preflight
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq \
  nginx apache2-utils build-essential pkg-config \
  libfftw3-dev \
  curl git rsync nodejs npm \
  python3 python3-venv python3-psutil \
  ca-certificates
ok "Dependencies installed"

ensure_modern_rust_toolchain
install_optional_tailscale
saturn_remote_bridge_preflight || true

info "Preparing runtime directories..."
mkdir -p "$BIN_DIR" "$SCRIPTS_DIR" "$WATCHDOG_SCRIPT_DIR" "$PRIVILEGED_SCRIPTS_DIR" "$WEB_ROOT" "$SATURN_STATE_DIR" "$SATURN_SNAPSHOT_DIR" "$SATURN_STAGING_DIR"
ok "Directories ready"

info "Copying web assets..."
saturn_go_build_remote_web_assets "$SOURCE_DIR"
if ! saturn_go_copy_required_web_assets "$SOURCE_DIR/templates" "$SOURCE_DIR" "$WEB_ROOT"; then
  err "Missing required web asset in $SOURCE_DIR/templates or $SOURCE_DIR"
  exit 1
fi
saturn_go_copy_optional_web_assets "$SOURCE_DIR/templates" "$SOURCE_DIR" "$WEB_ROOT"
if ! saturn_go_verify_remote_web_bundle "$WEB_ROOT"; then
  err "Deployed remote-web bundle checksum verification failed"
  exit 1
fi

if [[ -f "$SOURCE_DIR/scripts/config.json" ]]; then
  cp -f "$SOURCE_DIR/scripts/config.json" "$WEB_ROOT/config.json"
elif [[ -f "$SOURCE_DIR/config.json" ]]; then
  cp -f "$SOURCE_DIR/config.json" "$WEB_ROOT/config.json"
else
  err "Missing config.json in source tree"
  exit 1
fi

if [[ -f "$SOURCE_DIR/scripts/themes.json" ]]; then
  cp -f "$SOURCE_DIR/scripts/themes.json" "$WEB_ROOT/themes.json"
elif [[ -f "$SOURCE_DIR/themes.json" ]]; then
  cp -f "$SOURCE_DIR/themes.json" "$WEB_ROOT/themes.json"
else
  err "Missing themes.json in source tree"
  exit 1
fi
ok "Web assets copied"

info "Copying scripts..."
added_scripts=0
updated_scripts=0
kept_scripts=0
while IFS= read -r -d '' src; do
  dest="$SCRIPTS_DIR/$(basename "$src")"
  if [[ ! -e "$dest" ]]; then
    cp -f "$src" "$dest"
    added_scripts=$((added_scripts + 1))
  elif [[ "$src" -nt "$dest" ]]; then
    cp -f "$src" "$dest"
    updated_scripts=$((updated_scripts + 1))
  else
    kept_scripts=$((kept_scripts + 1))
  fi
done < <(find "$SOURCE_DIR/scripts" -maxdepth 1 -type f -print0)
for src in "${EXTRA_PACKAGED_SCRIPTS[@]}"; do
  dest="$SCRIPTS_DIR/$(basename "$src")"
  if [[ ! -e "$dest" ]]; then
    cp -f "$src" "$dest"
    added_scripts=$((added_scripts + 1))
  elif [[ "$src" -nt "$dest" ]]; then
    cp -f "$src" "$dest"
    updated_scripts=$((updated_scripts + 1))
  else
    kept_scripts=$((kept_scripts + 1))
  fi
done

info "Installing privileged helper scripts..."
for src in "${PRIVILEGED_HELPER_SCRIPTS[@]}"; do
  install -m 0755 -o root -g root "$src" "$PRIVILEGED_SCRIPTS_DIR/$(basename "$src")"
done
install_xdma_kernel_postinst_hook

cat >"$WATCHDOG_SCRIPT_PATH" <<'WATCHDOG'
#!/usr/bin/env bash
set -euo pipefail

url="${SATURN_WATCHDOG_URL:-http://127.0.0.1:8080/healthz}"
service="${SATURN_WATCHDOG_SERVICE:-saturn-go.service}"
timeout="${SATURN_WATCHDOG_TIMEOUT:-4}"

if ! curl -fsS --max-time "$timeout" "$url" >/dev/null 2>&1; then
  logger -t saturn-watchdog "health check failed for $url; restarting $service"
  systemctl restart "$service" || true
fi
WATCHDOG
ok "Scripts synced (added=$added_scripts updated=$updated_scripts kept=$kept_scripts; custom scripts preserved)"

info "Setting file permissions..."
chown -R root:root "$SATURN_ROOT" "$WEB_ROOT"
find "$WEB_ROOT" -type d -print0 | xargs -0 -r chmod 0755
find "$WEB_ROOT" -type f -print0 | xargs -0 -r chmod 0644
find "$SCRIPTS_DIR" -type d -print0 | xargs -0 -r chmod 0775
find "$SCRIPTS_DIR" -type f \( -name '*.sh' -o -name '*.py' \) -print0 | xargs -0 -r chmod 0755
find "$SCRIPTS_DIR" -type f ! \( -name '*.sh' -o -name '*.py' \) -print0 | xargs -0 -r chmod 0644
chown root:root "$WATCHDOG_SCRIPT_DIR" "$PRIVILEGED_SCRIPTS_DIR" "$WATCHDOG_SCRIPT_PATH"
chmod 0755 "$WATCHDOG_SCRIPT_DIR" "$PRIVILEGED_SCRIPTS_DIR" "$WATCHDOG_SCRIPT_PATH"
find "$PRIVILEGED_SCRIPTS_DIR" -maxdepth 1 -type f -print0 | xargs -0 -r chown root:root
find "$PRIVILEGED_SCRIPTS_DIR" -maxdepth 1 -type f -print0 | xargs -0 -r chmod 0755
chown -R "$SERVICE_USER:$SERVICE_GROUP" "$SCRIPTS_DIR"
chown -R "$SERVICE_USER:$SERVICE_GROUP" "$SATURN_STATE_DIR"
find "$SATURN_STATE_DIR" -type d -print0 | xargs -0 -r chmod 0750
find "$SATURN_STATE_DIR" -type f -print0 | xargs -0 -r chmod 0640
ok "Permissions set"

info "Writing sudoers policy for privileged helper scripts..."
cat >"$SUDOERS_FILE" <<EOF
# Managed by install_saturn_go_nginx.sh
Defaults:${SERVICE_USER} secure_path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/install-shutdown-waiter-service.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/install-shutdown-waiter-service.sh *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/setup-eth-fallback.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/setup-eth-fallback.sh *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/fix-LED-power-button.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/fix-LED-power-button.sh *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-xdma-doctor.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-xdma-doctor.sh *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-flash-fpga.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-flash-fpga.sh *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-xdma-stage-current.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-admin-password.sh set
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-admin-password.sh status
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-go-build-preflight.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-go-build-preflight.sh ensure-swap
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-go-build-preflight.sh status
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/${SATURN_BRIDGE_INSTALLER_NAME}
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/${SATURN_BRIDGE_INSTALLER_NAME} *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-tailscale.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-tailscale.sh *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-go-tailscale-serve.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/saturn-go-tailscale-serve.sh *
EOF
chmod 0440 "$SUDOERS_FILE"
if command -v visudo >/dev/null 2>&1; then
  visudo -cf "$SUDOERS_FILE" >/dev/null
fi
ok "Sudoers policy installed at $SUDOERS_FILE"

info "Building Rust server..."
SATURN_SATURNGO_BUILD_SWAP_FILE="$RUST_BUILD_SWAP_FILE" \
SATURN_SATURNGO_BUILD_SWAP_MIB="$RUST_BUILD_SWAP_MIB" \
  "$BUILD_PREFLIGHT_HELPER" ensure-swap
info "Rust build settings: CARGO_BUILD_JOBS=$RUST_BUILD_JOBS TMPDIR=$RUST_BUILD_TMP_DIR CARGO_TARGET_DIR=$RUST_BUILD_TARGET_DIR nice -n $RUST_BUILD_NICE ionice -c $RUST_BUILD_IONICE_CLASS"
mkdir -p "$RUST_BUILD_TMP_DIR" "$RUST_BUILD_TARGET_DIR"
chown -R "$BUILD_USER:$BUILD_GROUP" "$RUST_BUILD_TMP_DIR" "$RUST_BUILD_TARGET_DIR"
pushd "$RUST_SRC_DIR" >/dev/null
run_as_build_user "cd \"$RUST_SRC_DIR\" && CARGO_BUILD_JOBS=\"$RUST_BUILD_JOBS\" TMPDIR=\"$RUST_BUILD_TMP_DIR\" CARGO_TARGET_DIR=\"$RUST_BUILD_TARGET_DIR\" nice -n \"$RUST_BUILD_NICE\" ionice -c \"$RUST_BUILD_IONICE_CLASS\" \"$RUSTUP_CARGO_BIN\" build --release -j \"$RUST_BUILD_JOBS\""
cp -f "$RUST_BUILD_TARGET_DIR/release/saturn-go" "$BIN_DIR/saturn-go"
popd >/dev/null
chmod 0755 "$BIN_DIR/saturn-go"
ok "Rust binary installed to $BIN_DIR/saturn-go"

if [[ ! -f "$SATURN_REPO_ROOT_FILE" ]]; then
  printf '%s\n' "$DEFAULT_REPO_ROOT" > "$SATURN_REPO_ROOT_FILE"
  chown "$SERVICE_USER:$SERVICE_GROUP" "$SATURN_REPO_ROOT_FILE"
  chmod 0640 "$SATURN_REPO_ROOT_FILE"
fi

info "Configuring nginx..."
cat >"$NGINX_SSE_MAP" <<'NGINX'
map $http_accept $is_sse {
  default               0;
  "~*text/event-stream" 1;
}
NGINX

cat >"$NGINX_SITE" <<NGINX
server {
  listen 80 default_server;
  server_name _;
  client_max_body_size ${SATURN_NGINX_CLIENT_MAX_BODY_SIZE};

  location = / {
    return 302 /saturn/;
  }

  location = /remote {
    return 302 https://\$host:8443/remote;
  }

  location = /remote/ {
    return 302 https://\$host:8443/remote;
  }

  location = /remote.html {
    return 302 https://\$host:8443/remote;
  }

  location = /remote-next {
    return 302 https://\$host:8443/remote-next?${SATURN_REMOTE_NEXT_DEFAULT_QUERY};
  }

  location = /remote-next/ {
    return 302 https://\$host:8443/remote-next?${SATURN_REMOTE_NEXT_DEFAULT_QUERY};
  }

  location = /remote-next.html {
    return 302 https://\$host:8443/remote-next?${SATURN_REMOTE_NEXT_DEFAULT_QUERY};
  }

  location = /saturn/remote {
    return 302 https://\$host:8443/remote;
  }

  location = /saturn/remote/ {
    return 302 https://\$host:8443/remote;
  }

  location = /saturn/remote.html {
    return 302 https://\$host:8443/remote;
  }

  location = /saturn/remote-next {
    return 302 https://\$host:8443/remote-next?${SATURN_REMOTE_NEXT_DEFAULT_QUERY};
  }

  location = /saturn/remote-next/ {
    return 302 https://\$host:8443/remote-next?${SATURN_REMOTE_NEXT_DEFAULT_QUERY};
  }

  location = /saturn/remote-next.html {
    return 302 https://\$host:8443/remote-next?${SATURN_REMOTE_NEXT_DEFAULT_QUERY};
  }

  location = /saturn/run {
    auth_basic "Restricted";
    auth_basic_user_file ${BASIC_AUTH_FILE};

    include /etc/nginx/proxy_params;
    proxy_pass http://${SATURN_ADDR}/run;

    proxy_http_version 1.1;
    proxy_set_header Connection "";
    proxy_read_timeout 1d;
    proxy_send_timeout 1d;
    proxy_buffering off;
    proxy_request_buffering off;
    proxy_cache off;
    gzip off;
    add_header X-Accel-Buffering no;
    add_header Cache-Control "no-cache";
  }

  location /saturn/ {
    auth_basic "Restricted";
    auth_basic_user_file ${BASIC_AUTH_FILE};

    include /etc/nginx/proxy_params;
    proxy_pass http://${SATURN_ADDR}/;
    proxy_http_version 1.1;
    proxy_set_header Connection "";
    proxy_read_timeout 300s;
  }
}
NGINX

PASSWORD_MIN_LEN=5
# Reject newline / CR / tab / NUL: these cannot be safely written to a systemd
# Environment= line, and allowing them past .htpasswd would leave LAN auth
# updated while the TLS drop-in is skipped — a split-auth deployment where
# /saturn/* and /remote* accept different credentials.
password_has_control_char() {
  [[ "$1" == *[$'\n\r\t\0']* ]]
}
generate_readable_admin_password() {
  local words=(
    radio signal meter antenna vfo tuner audio remote beacon
    filter keyer waterfall spectrum carrier relay shack station
    gain level drive power field panel band mode
  )
  local digits n word_a word_b
  word_a="${words[$((RANDOM % ${#words[@]}))]}"
  word_b="${words[$((RANDOM % ${#words[@]}))]}"
  n="$(od -An -N2 -tu2 /dev/urandom)"
  digits="$(printf '%04d' "$((n % 10000))")"
  printf 'saturn-%s-%s-%s' "$word_a" "$word_b" "$digits"
}
generated_password=""
admin_password="${SATURN_ADMIN_PASSWORD:-}"
if [[ -n "$admin_password" ]]; then
  if [[ ${#admin_password} -lt ${PASSWORD_MIN_LEN} ]]; then
    err "Provided SATURN_ADMIN_PASSWORD is too short (minimum ${PASSWORD_MIN_LEN} characters)."
    exit 1
  fi
  if password_has_control_char "$admin_password"; then
    err "Provided SATURN_ADMIN_PASSWORD contains a control character (newline/CR/tab/NUL)."
    err "Use a password without control characters and re-run the installer."
    exit 1
  fi
  info "Setting HTTP basic auth credentials for admin user from SATURN_ADMIN_PASSWORD..."
  printf '%s\n' "$admin_password" | htpasswd -i -c "$BASIC_AUTH_FILE" admin >/dev/null
  chmod 0640 "$BASIC_AUTH_FILE"
  chown root:www-data "$BASIC_AUTH_FILE"
  ok "Basic auth configured"
elif [[ ! -s "$BASIC_AUTH_FILE" ]]; then
  info "Creating HTTP basic auth credentials for admin user..."
  if [[ -t 0 ]]; then
    while true; do
      read -r -s -p "Enter admin password (min ${PASSWORD_MIN_LEN} chars): " admin_password; echo
      read -r -s -p "Confirm admin password: " admin_password_confirm; echo
      if [[ "$admin_password" != "$admin_password_confirm" ]]; then
        warn "Passwords do not match. Try again."
        continue
      fi
      if [[ ${#admin_password} -lt ${PASSWORD_MIN_LEN} ]]; then
        warn "Password too short. Minimum ${PASSWORD_MIN_LEN} characters."
        continue
      fi
      if password_has_control_char "$admin_password"; then
        warn "Password contains a control character (newline/CR/tab/NUL). Try again."
        continue
      fi
      break
    done
  else
    admin_password="$(generate_readable_admin_password)"
    generated_password="$admin_password"
    warn "No TTY available; generated readable admin password."
  fi

  printf '%s\n' "$admin_password" | htpasswd -i -c "$BASIC_AUTH_FILE" admin >/dev/null
  chmod 0640 "$BASIC_AUTH_FILE"
  chown root:www-data "$BASIC_AUTH_FILE"
  ok "Basic auth configured"
else
  info "Reusing existing $BASIC_AUTH_FILE"
fi

# Saturn Remote TLS listener requires SATURN_REMOTE_BASIC_AUTH in the
# saturn-go.service environment. We write a systemd drop-in only when this
# install run captured a fresh plaintext password (either from the operator,
# from $SATURN_ADMIN_PASSWORD, or generated above). On reruns that reuse an
# existing /etc/nginx/.htpasswd we have no plaintext, so we preserve any
# existing drop-in and warn if none is present.
#
# Escape rules for systemd Environment="KEY=value":
#   - %% always (specifier expansion runs even inside quotes)
#   - \\ and \" inside the quoted form
#   - newlines / NULs / other control chars are not representable; reject them.
systemd_env_escape() {
  local v="$1"
  v="${v//\\/\\\\}"
  v="${v//\"/\\\"}"
  v="${v//%/%%}"
  printf '%s' "$v"
}
REMOTE_AUTH_DROPIN_DIR="/etc/systemd/system/$(basename "$SERVICE_FILE").d"
REMOTE_AUTH_DROPIN_FILE="$REMOTE_AUTH_DROPIN_DIR/10-remote-auth.conf"
if [[ -n "${admin_password:-}" ]]; then
  # admin_password is already validated control-char-free upstream (env-var,
  # interactive, and generated paths all check before writing .htpasswd), so
  # this branch can write the drop-in unconditionally and stay aligned with
  # the LAN nginx password.
  escaped_password="$(systemd_env_escape "$admin_password")"
  info "Writing Saturn Remote TLS auth drop-in: $REMOTE_AUTH_DROPIN_FILE"
  install -d -m 0755 -o root -g root "$REMOTE_AUTH_DROPIN_DIR"
  (
    umask 0177
    cat > "$REMOTE_AUTH_DROPIN_FILE" <<EOF
# Managed by install_saturn_go_nginx.sh
# Saturn Remote TLS listener basic-auth credentials. The TLS listener on
# :8443 (rust-server/src/remote_tls.rs) refuses to bind without this. Keep
# this aligned with /etc/nginx/.htpasswd so /saturn/* and /remote* accept
# the same admin password.
[Service]
Environment="SATURN_REMOTE_BASIC_AUTH=admin:${escaped_password}"
EOF
  )
  chmod 0600 "$REMOTE_AUTH_DROPIN_FILE"
  chown root:root "$REMOTE_AUTH_DROPIN_FILE"
  unset escaped_password
  ok "Saturn Remote TLS auth drop-in installed"
elif [[ -f "$REMOTE_AUTH_DROPIN_FILE" ]]; then
  ok "Preserving existing Saturn Remote TLS auth drop-in: $REMOTE_AUTH_DROPIN_FILE"
else
  warn "No fresh admin password and no existing $REMOTE_AUTH_DROPIN_FILE."
  warn "Saturn Remote TLS listener will refuse to bind on :8443 until SATURN_REMOTE_BASIC_AUTH is set."
  warn "To align manually with the existing /etc/nginx/.htpasswd password:"
  warn "  sudo systemctl edit $(basename "$SERVICE_FILE")"
  warn "  Add under [Service]:"
  warn "    Environment=\"SATURN_REMOTE_BASIC_AUTH=admin:<your-existing-password>\""
  warn "  sudo systemctl restart $(basename "$SERVICE_FILE")"
  warn "Or rerun this installer with SATURN_ADMIN_PASSWORD=<password> to write both."
fi

rm -f /etc/nginx/sites-enabled/default || true
ln -sf "$NGINX_SITE" "$NGINX_SITE_LINK"

if ss -ltnp | grep -q ':80 ' && ss -ltnp | grep -qi apache2; then
  warn "Apache detected on port 80; stopping and disabling apache2"
  systemctl stop apache2 || true
  systemctl disable apache2 || true
fi

nginx -t
if systemctl is-active --quiet nginx; then
  systemctl reload nginx
else
  systemctl enable --now nginx
fi
ok "nginx configured"

info "Writing systemd unit..."
cat >"$SERVICE_FILE" <<SERVICE
[Unit]
Description=Saturn Update Manager (Rust backend)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=HOME=${SERVICE_HOME}
Environment=PATH=${SERVICE_HOME}/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
Environment=CARGO_HOME=${SERVICE_HOME}/.cargo
Environment=RUSTUP_HOME=${SERVICE_HOME}/.rustup
Environment=SATURN_WEBROOT=${WEB_ROOT}
Environment=SATURN_CONFIG=${WEB_ROOT}/config.json
Environment=SATURN_ADDR=${SATURN_ADDR}
Environment=SATURN_REPO_ROOT=${DEFAULT_REPO_ROOT}
Environment=SATURN_REPO_ROOT_FILE=${SATURN_REPO_ROOT_FILE}
Environment=SATURN_STATE_DIR=${SATURN_STATE_DIR}
Environment=SATURN_UPDATE_POLICY_FILE=${SATURN_UPDATE_POLICY_FILE}
Environment=SATURN_SATURNGO_UPDATE_POLICY_FILE=${SATURN_SATURNGO_UPDATE_POLICY_FILE}
Environment=SATURN_SATURNGO_DEPLOY_STATUS_FILE=${SATURN_SATURNGO_DEPLOY_STATUS_FILE}
Environment=SATURN_UPDATE_STATE_FILE=${SATURN_UPDATE_STATE_FILE}
Environment=SATURN_SNAPSHOT_DIR=${SATURN_SNAPSHOT_DIR}
Environment=SATURN_STAGING_DIR=${SATURN_STAGING_DIR}
Environment=SATURN_MAX_BODY_BYTES=${SATURN_MAX_BODY_BYTES}
Environment=SATURN_RESTORE_MAX_UPLOAD_BYTES=${SATURN_RESTORE_MAX_UPLOAD_BYTES}
Environment=PYTHONUNBUFFERED=1
ExecStart=${BIN_DIR}/saturn-go
WorkingDirectory=${SATURN_ROOT}
User=${SERVICE_USER}
Group=${SERVICE_GROUP}
Restart=on-failure
RestartSec=2
PrivateTmp=true
RestrictSUIDSGID=true
LockPersonality=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
ProtectClock=true
SystemCallArchitectures=native
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
SERVICE

cat >"$WATCHDOG_SERVICE_FILE" <<WATCHDOG_SERVICE
[Unit]
Description=Saturn Update Manager Health Watchdog
After=network-online.target saturn-go.service
Wants=network-online.target

[Service]
Type=oneshot
Environment=SATURN_WATCHDOG_URL=${SATURN_WATCHDOG_URL}
Environment=SATURN_WATCHDOG_SERVICE=saturn-go.service
Environment=SATURN_WATCHDOG_TIMEOUT=4
ExecStart=${WATCHDOG_SCRIPT_PATH}
WATCHDOG_SERVICE

cat >"$WATCHDOG_TIMER_FILE" <<WATCHDOG_TIMER
[Unit]
Description=Run Saturn health watchdog

[Timer]
OnBootSec=45s
OnUnitActiveSec=${SATURN_WATCHDOG_INTERVAL}
AccuracySec=1s
Persistent=true
Unit=saturn-go-watchdog.service

[Install]
WantedBy=timers.target
WATCHDOG_TIMER

systemctl daemon-reload
systemctl enable saturn-go.service
systemctl enable saturn-go-watchdog.timer
if systemctl is-active --quiet saturn-go.service; then
  systemctl restart saturn-go.service
else
  systemctl start saturn-go.service
fi
if systemctl is-active --quiet saturn-go-watchdog.timer; then
  systemctl restart saturn-go-watchdog.timer
else
  systemctl start saturn-go-watchdog.timer
fi
ok "Service and watchdog enabled and restarted"

install_saturn_bridge_if_requested

info "Waiting for backend health endpoint..."
healthy=0
for _ in {1..40}; do
  if curl -fsS "http://${SATURN_ADDR}/healthz" >/dev/null 2>&1; then
    healthy=1
    ok "Backend is healthy"
    break
  fi
  sleep 0.25
done
if [[ $healthy -ne 1 ]]; then
  err "Backend health check failed at http://${SATURN_ADDR}/healthz"
  echo "[INFO] saturn-go.service status:"
  systemctl --no-pager --full status saturn-go.service || true
  echo "[INFO] Recent saturn-go.service logs:"
  journalctl -u saturn-go.service -n 40 --no-pager || true
  exit 1
fi

bold "[SUMMARY]"
echo " Web UI:   http://<host>/saturn/"
echo " API base: http://<host>/saturn/"
echo " Binary:   ${BIN_DIR}/saturn-go"
echo " Service:  saturn-go.service (user=${SERVICE_USER})"
echo " Watchdog: saturn-go-watchdog.timer (${SATURN_WATCHDOG_INTERVAL})"
echo " Repo root default: ${DEFAULT_REPO_ROOT}"
if env_flag_enabled "${SATURN_INSTALL_TAILSCALE:-}"; then
  echo " Tailscale: package install requested; setup is operator-driven."
  echo " Next: sudo tailscale set --hostname=<your-saturn-host>"
  echo " Next: sudo tailscale up"
  echo " Next: sudo ${SCRIPTS_DIR}/saturn-go-tailscale-serve.sh (see OPERATIONS_RUNBOOK.md -> Secure Remote Access with Tailscale)"
fi
if [[ -n "$generated_password" ]]; then
  echo " Admin user: admin"
  echo " Generated password: ${generated_password}"
  warn "Store this password now and change it after first login."
fi
ok "Install complete."
