#!/usr/bin/env bash
set -euo pipefail

INSTALLER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
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
SOURCE_DIR="${SATURN_UPDATE_MANAGER_SOURCE_DIR:-$INSTALLER_DIR}"
RUST_SRC_DIR="$SOURCE_DIR/rust-server"
WEB_ASSET_HELPERS="$SOURCE_DIR/scripts/saturn-go-web-assets.sh"
BUILD_PREFLIGHT_HELPER="$SOURCE_DIR/scripts/saturn-go-build-preflight.sh"
XDMA_FIX_SCRIPT_INSTALL="/usr/local/bin/saturn-fix-xdma.sh"
XDMA_POSTINST_HELPER_INSTALL="/usr/local/bin/saturn-xdma-kernel-postinst.sh"
XDMA_POSTINST_HOOK_PATH="/etc/kernel/postinst.d/saturn-xdma"
SATURN_BRIDGE_INSTALLER_NAME="install-saturn-bridge.sh"
SATURN_GO_DEPLOY_BROKER_NAME="saturn-go-deploy-root.sh"
SATURN_GO_DEPLOY_BROKER="$PRIVILEGED_SCRIPTS_DIR/$SATURN_GO_DEPLOY_BROKER_NAME"
SATURN_GO_DEPLOY_CONFIG="/etc/default/saturn-go-deploy"
SATURN_RELEASE_INSTALLER_NAME="saturn-release-install-root.sh"
SATURN_RELEASE_INSTALL_CONFIG="/etc/default/saturn-release-install"
SATURN_RELEASE_ACTIVATOR_NAME="saturn-release-activate-root.sh"
SATURN_RELEASE_ACTIVATE_CONFIG="/etc/default/saturn-release-activate"
SATURN_RELEASE_POLICY_DIR="$WATCHDOG_SCRIPT_DIR/release"
SATURN_RELEASE_MANIFEST_TOOL_NAME="saturn-release-manifest.py"
SATURN_RELEASE_COMPONENTS_NAME="components-v1.json"

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
SATURN_RELEASE_STAGING_DIR="${SATURN_RELEASE_STAGING_DIR:-${SATURN_STATE_DIR}/release-staging}"
SATURN_RELEASES_ROOT="${SATURN_RELEASES_ROOT:-/opt/saturn/releases}"
SATURN_RELEASE_CURRENT_LINK="${SATURN_RELEASE_CURRENT_LINK:-/opt/saturn/current}"
SATURN_RELEASE_TRANSACTION_FILE="${SATURN_RELEASE_TRANSACTION_FILE:-${SATURN_STATE_DIR}/deployments/current.json}"
SATURN_DEPLOYMENT_STATE_DIR="$(dirname "$SATURN_RELEASE_TRANSACTION_FILE")"
SATURN_RELEASE_ACTIVATION_ENABLED="${SATURN_RELEASE_ACTIVATION_ENABLED:-0}"
SATURN_WATCHDOG_URL="${SATURN_WATCHDOG_URL:-http://${SATURN_ADDR}/livez}"
SATURN_WATCHDOG_INTERVAL="${SATURN_WATCHDOG_INTERVAL:-30s}"
SATURN_INSTALL_PACKAGES="${SATURN_INSTALL_PACKAGES:-1}"
SATURN_INSTALL_LEGACY_XDMA_HOOK="${SATURN_INSTALL_LEGACY_XDMA_HOOK:-1}"
RUSTUP_INIT_URL="${RUSTUP_INIT_URL:-https://sh.rustup.rs}"
RUSTUP_INIT_SHA256="${RUSTUP_INIT_SHA256:-6c30b75a75b28a96fd913a037c8581b580080b6ee9b8169a3c0feb1af7fe8caf}"
TAILSCALE_INSTALL_URL="${TAILSCALE_INSTALL_URL:-https://tailscale.com/install.sh}"
TAILSCALE_INSTALL_SHA256="${TAILSCALE_INSTALL_SHA256:-ada2fe9d54df0d3e5a77879470bda195b2c53d27ecd73aba6de270c795725625}"
SATURN_INSTALL_BRIDGE="${SATURN_INSTALL_BRIDGE:-1}"
SATURN_REQUIRE_BRIDGE="${SATURN_REQUIRE_BRIDGE:-$SATURN_INSTALL_BRIDGE}"
SATURN_READY_REQUIRE_BRIDGE="${SATURN_READY_REQUIRE_BRIDGE:-$SATURN_REQUIRE_BRIDGE}"
SATURN_DEFER_FINAL_READINESS="${SATURN_DEFER_FINAL_READINESS:-0}"
SATURN_BRIDGE_WDSP_FLAVOR="${SATURN_BRIDGE_WDSP_FLAVOR:-wdsp2}"
SATURN_WDSP2_REPO_URL="${SATURN_WDSP2_REPO_URL:-https://github.com/TAPR/OpenHPSDR-wdsp.git}"
SATURN_WDSP2_REF="${SATURN_WDSP2_REF:-584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3}"
SATURN_PIHPSDR_PORT_REPO_URL="${SATURN_PIHPSDR_PORT_REPO_URL:-https://github.com/dl1ycf/pihpsdr.git}"
SATURN_PIHPSDR_PORT_REF="${SATURN_PIHPSDR_PORT_REF:-974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1}"

bold(){ printf "\e[1m%s\e[0m\n" "$*"; }
ok(){   printf "[OK] %s\n" "$*"; }
info(){ printf "[INFO] %s\n" "$*"; }
warn(){ printf "[WARN] %s\n" "$*"; }
err(){  printf "[ERR] %s\n" "$*" >&2; }

systemd_env_escape(){
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//%/%%}"
  printf '%s' "$value"
}

download_verified() {
  local url="$1" expected="$2" dest="$3" actual
  curl --proto '=https' --tlsv1.2 -fsSL "$url" -o "$dest"
  actual="$(sha256sum "$dest" | awk '{print $1}')"
  [[ "$actual" == "$expected" ]] || {
    rm -f "$dest"
    err "Checksum mismatch for $url (expected $expected, got $actual)"
    return 1
  }
}

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
ADMIN_PASSWORD_HELPER_SRC="$REPO_SOURCE_DIR/scripts/saturn-admin-password.sh"
EXTRA_PACKAGED_SCRIPTS=(
  "$REPO_SOURCE_DIR/scripts/fix-LED-power-button.sh"
  "$REPO_SOURCE_DIR/scripts/install-shutdown-waiter-service.sh"
  "$REPO_SOURCE_DIR/scripts/shutdown-waiter.sh"
  "$REPO_SOURCE_DIR/scripts/setup-eth-fallback.sh"
)
PRIVILEGED_HELPER_SCRIPTS=(
  "$SOURCE_DIR/scripts/saturn-go-build-preflight.sh"
  "$SOURCE_DIR/scripts/$SATURN_GO_DEPLOY_BROKER_NAME"
  "$SOURCE_DIR/scripts/$SATURN_RELEASE_INSTALLER_NAME"
  "$SOURCE_DIR/scripts/$SATURN_RELEASE_ACTIVATOR_NAME"
  "$SOURCE_DIR/scripts/$SATURN_BRIDGE_INSTALLER_NAME"
  "$ADMIN_PASSWORD_HELPER_SRC"
  "$REPO_SOURCE_DIR/scripts/saturn-flash-fpga.sh"
  "$REPO_SOURCE_DIR/scripts/saturn-xdma-doctor.sh"
  "$REPO_SOURCE_DIR/scripts/saturn-xdma-stage-current.sh"
  "$REPO_SOURCE_DIR/scripts/install-udev-rules-on-current-image.sh"
  "$REPO_SOURCE_DIR/scripts/deskhpsdr-install-deps-on-current-image.sh"
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
# shellcheck disable=SC1090
source "$WEB_ASSET_HELPERS"
for extra_script in "${EXTRA_PACKAGED_SCRIPTS[@]}"; do
  if [[ ! -f "$extra_script" ]]; then
    err "Extra packaged script not found: $extra_script"
    exit 1
  fi
done
if [[ ! -f "$SOURCE_DIR/scripts/$SATURN_RELEASE_MANIFEST_TOOL_NAME" ]]; then
  err "Release manifest validator not found: $SOURCE_DIR/scripts/$SATURN_RELEASE_MANIFEST_TOOL_NAME"
  exit 1
fi
if [[ ! -f "$SOURCE_DIR/release/$SATURN_RELEASE_COMPONENTS_NAME" ]]; then
  err "Release component policy not found: $SOURCE_DIR/release/$SATURN_RELEASE_COMPONENTS_NAME"
  exit 1
fi
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
SERVICE_GROUP="${SATURN_SERVICE_GROUP:-$(id -gn "$SERVICE_USER" 2>/dev/null || true)}"

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
SATURN_LOG_DIR="${SATURN_LOG_DIR:-$SERVICE_HOME/saturn-logs}"
DEFAULT_REPO_ROOT="${SATURN_REPO_ROOT:-$SERVICE_HOME/github/Saturn}"
SATURN_PIHPSDR_DIR="${SATURN_PIHPSDR_DIR:-$SERVICE_HOME/github/pihpsdr}"

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
RUST_BUILD_SWAP_FILE="${SATURN_SATURNGO_BUILD_SWAP_FILE:-$BUILD_HOME/saturn-build.swap}"
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
  local tmp_hook disabled_hook

  info "Installing XDMA kernel postinst helpers..."
  install -D -m 0755 -o root -g root "$XDMA_FIX_SCRIPT_SRC" "$XDMA_FIX_SCRIPT_INSTALL"
  install -D -m 0755 -o root -g root "$XDMA_POSTINST_HELPER_SRC" "$XDMA_POSTINST_HELPER_INSTALL"

  # DKMS and the legacy post-install hook must never both own XDMA kernel
  # updates. The appliance installer uses DKMS, so retain only the helpers
  # used for diagnostics and remove the legacy hook when DKMS is registered.
  if command -v dkms >/dev/null 2>&1 \
      && [[ -n "$(dkms status -m saturn-xdma 2>/dev/null || true)" ]]; then
    disabled_hook="${XDMA_POSTINST_HOOK_PATH}.disabled-by-dkms"
    if [[ -e "$XDMA_POSTINST_HOOK_PATH" || -L "$XDMA_POSTINST_HOOK_PATH" ]]; then
      rm -f "$disabled_hook"
      mv "$XDMA_POSTINST_HOOK_PATH" "$disabled_hook"
      ok "Disabled legacy XDMA hook because DKMS manages the driver"
    else
      ok "DKMS manages XDMA; legacy kernel post-install hook remains disabled"
    fi
    return 0
  fi

  if ! env_flag_enabled "$SATURN_INSTALL_LEGACY_XDMA_HOOK"; then
    info "Legacy XDMA kernel post-install hook installation is disabled"
    return 0
  fi

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

require_install_dependencies() {
  local missing=()
  local package
  for package in \
    nginx apache2-utils build-essential binutils pkg-config libfftw3-dev libopus0 \
    curl git rsync nodejs npm python3 python3-venv python3-psutil ca-certificates
  do
    apt_pkg_installed "$package" || missing+=("$package")
  done
  if (( ${#missing[@]} > 0 )); then
    err "Missing system dependencies while SATURN_INSTALL_PACKAGES=0: ${missing[*]}"
    exit 1
  fi
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

  local installer
  installer="$(mktemp)"
  info "Installing checksum-pinned Tailscale package via ${TAILSCALE_INSTALL_URL}"
  download_verified "$TAILSCALE_INSTALL_URL" "$TAILSCALE_INSTALL_SHA256" "$installer"
  sh "$installer"
  rm -f "$installer"
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

  if ! bridge_requested; then
    info "Saturn Remote bridge installation explicitly disabled"
    return 0
  fi

  info "Checking Saturn Remote bridge prerequisites..."
  [[ -f "$bridge_dir/Cargo.toml" ]] || missing+=("$bridge_dir/Cargo.toml")
  [[ -x "$bridge_dir/scripts/build-wdsp2-linux-arm.sh" ]] || missing+=("$bridge_dir/scripts/build-wdsp2-linux-arm.sh")
  [[ -x "$bridge_installer" ]] || missing+=("$bridge_installer")
  command -v "$RUSTUP_CARGO_BIN" >/dev/null 2>&1 || [[ -x "$RUSTUP_CARGO_BIN" ]] || missing+=("$RUSTUP_CARGO_BIN")
  apt_pkg_installed libfftw3-dev || missing+=("apt:libfftw3-dev")
  command -v git >/dev/null 2>&1 || missing+=("git")
  command -v python3 >/dev/null 2>&1 || missing+=("python3")
  command -v nm >/dev/null 2>&1 || missing+=("binutils:nm")

  if (( ${#missing[@]} == 0 )); then
    ok "Saturn Remote bridge prerequisites present"
    return 0
  fi

  warn "Saturn Remote bridge prerequisites are missing:"
  printf '  - %s\n' "${missing[@]}" >&2
  warn "Remote pages can load, but /remote and /remote-next will not work until saturn-bridge is installed."
  warn "The installer provisions pinned WDSP 2.00 and Linux-port sources; no piHPSDR build or cloud-init step is required."

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
    SATURN_BRIDGE_WDSP_FLAVOR="$SATURN_BRIDGE_WDSP_FLAVOR" \
    SATURN_WDSP2_REPO_URL="$SATURN_WDSP2_REPO_URL" \
    SATURN_WDSP2_REF="$SATURN_WDSP2_REF" \
    SATURN_PIHPSDR_PORT_REPO_URL="$SATURN_PIHPSDR_PORT_REPO_URL" \
    SATURN_PIHPSDR_PORT_REF="$SATURN_PIHPSDR_PORT_REF" \
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
  if ! env_flag_enabled "$SATURN_INSTALL_PACKAGES"; then
    info "Leaving distro Rust packages unchanged (SATURN_INSTALL_PACKAGES=0); the build uses the user rustup toolchain explicitly."
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
  # Backticks are literal Cargo error text.
  # shellcheck disable=SC2016
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
    local rustup_installer
    rustup_installer="$(mktemp)"
    info "Installing rustup toolchain for build user '$BUILD_USER'..."
    download_verified "$RUSTUP_INIT_URL" "$RUSTUP_INIT_SHA256" "$rustup_installer"
    chmod 0644 "$rustup_installer"
    run_as_build_user "sh \"$rustup_installer\" -y --profile minimal --default-toolchain stable"
    rm -f "$rustup_installer"
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

if env_flag_enabled "$SATURN_INSTALL_PACKAGES"; then
  info "Installing dependencies..."
  check_tmp_space_preflight
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq \
    nginx apache2-utils build-essential binutils pkg-config \
    libfftw3-dev libopus0 \
    curl git rsync nodejs npm \
    python3 python3-venv python3-psutil \
    ca-certificates
  ok "Dependencies installed"
else
  info "Skipping dependency installation (SATURN_INSTALL_PACKAGES=0)"
  require_install_dependencies
fi

ensure_modern_rust_toolchain
install_optional_tailscale
saturn_remote_bridge_preflight || true

info "Preparing runtime directories..."
mkdir -p "$BIN_DIR" "$SCRIPTS_DIR" "$WATCHDOG_SCRIPT_DIR" "$PRIVILEGED_SCRIPTS_DIR" "$WEB_ROOT" "$SATURN_STATE_DIR" "$SATURN_SNAPSHOT_DIR" "$SATURN_STAGING_DIR" "$SATURN_RELEASE_STAGING_DIR"
install -d -m 0755 -o root -g root "$SATURN_RELEASE_POLICY_DIR" "$SATURN_RELEASES_ROOT"
if [[ -L "$SATURN_LOG_DIR" ]]; then
  err "Refusing symlinked Saturn log directory: $SATURN_LOG_DIR"
  exit 1
fi
install -d -m 0755 -o "$SERVICE_USER" -g "$SERVICE_GROUP" "$SATURN_LOG_DIR"
ok "Directories ready"

info "Copying web assets..."
saturn_go_build_remote_web_assets "$SOURCE_DIR"
if ! saturn_go_copy_required_web_assets "$SOURCE_DIR/templates" "$SOURCE_DIR" "$WEB_ROOT"; then
  err "Missing required web asset in $SOURCE_DIR/templates or $SOURCE_DIR"
  exit 1
fi
if ! saturn_go_copy_shared_assets "$SOURCE_DIR/templates" "$WEB_ROOT"; then
  err "Missing shared web assets in $SOURCE_DIR/templates/assets"
  exit 1
fi
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
install -m 0755 -o root -g root \
  "$SOURCE_DIR/scripts/$SATURN_RELEASE_MANIFEST_TOOL_NAME" \
  "$PRIVILEGED_SCRIPTS_DIR/$SATURN_RELEASE_MANIFEST_TOOL_NAME"
install -m 0644 -o root -g root \
  "$SOURCE_DIR/release/$SATURN_RELEASE_COMPONENTS_NAME" \
  "$SATURN_RELEASE_POLICY_DIR/$SATURN_RELEASE_COMPONENTS_NAME"
# Whole-disk imaging is intentionally a local-console maintenance function.
# Remove helpers installed by older Saturn Go releases so the web service no
# longer retains sudo-capable imaging, clone, or target-wipe entry points.
rm -f \
  "$PRIVILEGED_SCRIPTS_DIR/make_pi_image.sh" \
  "$PRIVILEGED_SCRIPTS_DIR/clone_pi_to_device.sh" \
  "$PRIVILEGED_SCRIPTS_DIR/saturn-pi-wipe-target.sh"
install_xdma_kernel_postinst_hook

cat >"$WATCHDOG_SCRIPT_PATH" <<'WATCHDOG'
#!/usr/bin/env bash
set -euo pipefail

url="${SATURN_WATCHDOG_URL:-http://127.0.0.1:8080/livez}"
service="${SATURN_WATCHDOG_SERVICE:-saturn-go.service}"
timeout="${SATURN_WATCHDOG_TIMEOUT:-10}"
failure_limit="${SATURN_WATCHDOG_FAILURE_LIMIT:-3}"
failure_file="${SATURN_WATCHDOG_FAILURE_FILE:-/run/saturn-go-watchdog.failures}"

if curl -fsS --max-time "$timeout" "$url" >/dev/null 2>&1; then
  rm -f "$failure_file"
  exit 0
fi

failures=1
if [[ -r "$failure_file" ]]; then
  previous="$(cat "$failure_file" 2>/dev/null || true)"
  if [[ "$previous" =~ ^[0-9]+$ ]]; then
    failures=$((previous + 1))
  fi
fi
printf '%s\n' "$failures" >"$failure_file"

if (( failures < failure_limit )); then
  logger -t saturn-watchdog "health check failed for $url ($failures/$failure_limit); deferring restart"
  exit 0
fi

rm -f "$failure_file"
logger -t saturn-watchdog "health check failed for $url ($failures/$failure_limit); restarting $service"
systemctl restart "$service" || true
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
chown root:root "$SATURN_RELEASE_POLICY_DIR" "$SATURN_RELEASES_ROOT"
chmod 0755 "$SATURN_RELEASE_POLICY_DIR" "$SATURN_RELEASES_ROOT"
find "$PRIVILEGED_SCRIPTS_DIR" -maxdepth 1 -type f -print0 | xargs -0 -r chown root:root
find "$PRIVILEGED_SCRIPTS_DIR" -maxdepth 1 -type f -print0 | xargs -0 -r chmod 0755
chown -R "$SERVICE_USER:$SERVICE_GROUP" "$SCRIPTS_DIR"
chown "$SERVICE_USER:$SERVICE_GROUP" "$SATURN_STATE_DIR"
find "$SATURN_STATE_DIR" -mindepth 1 -path "$SATURN_DEPLOYMENT_STATE_DIR" -prune -o \
  -exec chown "$SERVICE_USER:$SERVICE_GROUP" {} +
install -d -m 0750 -o root -g "$SERVICE_GROUP" "$SATURN_DEPLOYMENT_STATE_DIR"
if [[ -f "$SATURN_RELEASE_TRANSACTION_FILE" && ! -L "$SATURN_RELEASE_TRANSACTION_FILE" ]]; then
  chown root:"$SERVICE_GROUP" "$SATURN_RELEASE_TRANSACTION_FILE"
  chmod 0640 "$SATURN_RELEASE_TRANSACTION_FILE"
fi
# Completed release bundles carry manifest-declared executable modes. Keep the
# staging root private, but never rewrite payload modes after validation.
find "$SATURN_STATE_DIR" \( -path "$SATURN_RELEASE_STAGING_DIR" -o -path "$SATURN_DEPLOYMENT_STATE_DIR" \) -prune -o -type d -print0 \
  | xargs -0 -r chmod 0750
find "$SATURN_STATE_DIR" \( -path "$SATURN_RELEASE_STAGING_DIR" -o -path "$SATURN_DEPLOYMENT_STATE_DIR" \) -prune -o -type f -print0 \
  | xargs -0 -r chmod 0640
chmod 0750 "$SATURN_RELEASE_STAGING_DIR"
ok "Permissions set"

info "Writing sudoers policy for privileged helper scripts..."
install -d -m 0755 /etc/default
cat >"$SATURN_GO_DEPLOY_CONFIG" <<EOF
# Managed by install_saturn_go_nginx.sh. This file must remain root-owned.
RUN_USER="$SERVICE_USER"
RUN_GROUP="$SERVICE_GROUP"
SATURN_GO_HEALTH_URL="http://${SATURN_ADDR}/readyz"
STAGING_ROOT="$SATURN_STAGING_DIR"
STATUS_FILE="$SATURN_SATURNGO_DEPLOY_STATUS_FILE"
SATURN_ROOT="$SATURN_ROOT"
SATURN_GO_BIN="$BIN_DIR/saturn-go"
SATURN_GO_SERVICE="saturn-go.service"
BRIDGE_BIN="$BIN_DIR/saturn-bridge"
BRIDGE_SERVICE="saturn-bridge.service"
BRIDGE_SERVICE_FILE="/etc/systemd/system/saturn-bridge.service"
WEB_ROOT="$WEB_ROOT"
SCRIPTS_DIR="$SCRIPTS_DIR"
BRIDGE_MAX_RATE_KHZ="${SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ:-192}"
BRIDGE_OPUS_ENABLED="${SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED:-1}"
BRIDGE_RF_TX_ENABLED="${SATURN_BRIDGE_RF_TX_ENABLED:-1}"
EOF
chown root:root "$SATURN_GO_DEPLOY_CONFIG"
chmod 0644 "$SATURN_GO_DEPLOY_CONFIG"
cat >"$SATURN_RELEASE_INSTALL_CONFIG" <<EOF
# Managed by install_saturn_go_nginx.sh. Parsed as data by the root installer.
RUN_USER="$SERVICE_USER"
STAGING_ROOT="$SATURN_RELEASE_STAGING_DIR"
RELEASES_ROOT="$SATURN_RELEASES_ROOT"
MANIFEST_TOOL="$PRIVILEGED_SCRIPTS_DIR/$SATURN_RELEASE_MANIFEST_TOOL_NAME"
COMPONENTS_FILE="$SATURN_RELEASE_POLICY_DIR/$SATURN_RELEASE_COMPONENTS_NAME"
INSTALL_OWNER="root"
INSTALL_GROUP="root"
EOF
chown root:root "$SATURN_RELEASE_INSTALL_CONFIG"
chmod 0644 "$SATURN_RELEASE_INSTALL_CONFIG"
cat >"$SATURN_RELEASE_ACTIVATE_CONFIG" <<EOF
# Managed by install_saturn_go_nginx.sh. Parsed as data by the root activator.
# Keep disabled until REM-0204 automatic rollback has been implemented and tested.
ACTIVATION_ENABLED="$SATURN_RELEASE_ACTIVATION_ENABLED"
SATURN_ROOT="$(dirname "$SATURN_RELEASES_ROOT")"
RELEASES_ROOT="$SATURN_RELEASES_ROOT"
CURRENT_LINK="$SATURN_RELEASE_CURRENT_LINK"
TRANSACTION_FILE="$SATURN_RELEASE_TRANSACTION_FILE"
LOCK_FILE="/run/lock/saturn-release-activate.lock"
MANIFEST_TOOL="$PRIVILEGED_SCRIPTS_DIR/$SATURN_RELEASE_MANIFEST_TOOL_NAME"
COMPONENTS_FILE="$SATURN_RELEASE_POLICY_DIR/$SATURN_RELEASE_COMPONENTS_NAME"
SYSTEMD_ROOT="/etc/systemd/system"
SATURN_GO_SERVICE="saturn-go.service"
BRIDGE_SERVICE="saturn-bridge.service"
P2APP_SERVICE="p2app.service"
SATURN_GO_READY_URL="http://${SATURN_ADDR}/readyz"
READY_TIMEOUT_SECONDS="30"
P2APP_PANEL_ENABLED="${SATURN_P2APP_PANEL_ENABLED:-0}"
TRANSACTION_GROUP="$SERVICE_GROUP"
EOF
chown root:root "$SATURN_RELEASE_ACTIVATE_CONFIG"
chmod 0644 "$SATURN_RELEASE_ACTIVATE_CONFIG"
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
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/install-udev-rules-on-current-image.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/install-udev-rules-on-current-image.sh *
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/deskhpsdr-install-deps-on-current-image.sh
${SERVICE_USER} ALL=(root) NOPASSWD: ${PRIVILEGED_SCRIPTS_DIR}/deskhpsdr-install-deps-on-current-image.sh *
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
${SERVICE_USER} ALL=(root) NOPASSWD: /usr/bin/systemd-run --unit saturn-go-self-deploy-* --collect --no-block ${SATURN_GO_DEPLOY_BROKER} ${SATURN_STAGING_DIR}/*
EOF
chmod 0440 "$SUDOERS_FILE"
if command -v visudo >/dev/null 2>&1; then
  visudo -cf "$SUDOERS_FILE" >/dev/null
fi
ok "Sudoers policy installed at $SUDOERS_FILE"

info "Building Rust server..."
RUST_BUILD_COMMIT="$(git -C "$REPO_SOURCE_DIR" rev-parse HEAD 2>/dev/null || true)"
if [[ ! "$RUST_BUILD_COMMIT" =~ ^[0-9a-fA-F]{40}$ ]]; then
  err "Could not resolve a full Git commit for the Saturn Go build: ${RUST_BUILD_COMMIT:-unknown}"
  exit 1
fi
RUST_BUILD_COMMIT="${RUST_BUILD_COMMIT,,}"
SATURN_SATURNGO_BUILD_SWAP_FILE="$RUST_BUILD_SWAP_FILE" \
SATURN_SATURNGO_BUILD_SWAP_MIB="$RUST_BUILD_SWAP_MIB" \
  "$BUILD_PREFLIGHT_HELPER" ensure-swap
info "Rust build settings: CARGO_BUILD_JOBS=$RUST_BUILD_JOBS TMPDIR=$RUST_BUILD_TMP_DIR CARGO_TARGET_DIR=$RUST_BUILD_TARGET_DIR nice -n $RUST_BUILD_NICE ionice -c $RUST_BUILD_IONICE_CLASS"
mkdir -p "$RUST_BUILD_TMP_DIR" "$RUST_BUILD_TARGET_DIR"
chown -R "$BUILD_USER:$BUILD_GROUP" "$RUST_BUILD_TMP_DIR" "$RUST_BUILD_TARGET_DIR"
pushd "$RUST_SRC_DIR" >/dev/null
run_as_build_user "cd \"$RUST_SRC_DIR\" && SATURN_BUILD_COMMIT=\"$RUST_BUILD_COMMIT\" CARGO_BUILD_JOBS=\"$RUST_BUILD_JOBS\" TMPDIR=\"$RUST_BUILD_TMP_DIR\" CARGO_TARGET_DIR=\"$RUST_BUILD_TARGET_DIR\" nice -n \"$RUST_BUILD_NICE\" ionice -c \"$RUST_BUILD_IONICE_CLASS\" \"$RUSTUP_CARGO_BIN\" build --release -j \"$RUST_BUILD_JOBS\""
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

  location = /saturn {
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
    return 302 https://\$host:8443/remote-next;
  }

  location = /remote-next/ {
    return 302 https://\$host:8443/remote-next;
  }

  location = /remote-next.html {
    return 302 https://\$host:8443/remote-next;
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
    return 302 https://\$host:8443/remote-next;
  }

  location = /saturn/remote-next/ {
    return 302 https://\$host:8443/remote-next;
  }

  location = /saturn/remote-next.html {
    return 302 https://\$host:8443/remote-next;
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
GENERATED_PASSWORD_LEN=5
REMOTE_AUTH_DROPIN_DIR="/etc/systemd/system/$(basename "$SERVICE_FILE").d"
REMOTE_AUTH_DROPIN_FILE="$REMOTE_AUTH_DROPIN_DIR/10-remote-auth.conf"
generate_readable_admin_password() {
  local charset='abcdefghjkmnpqrstuvwxyz23456789'
  local password='' byte index
  while [[ ${#password} -lt $GENERATED_PASSWORD_LEN ]]; do
    byte="$(od -An -N1 -tu1 /dev/urandom | tr -d '[:space:]')"
    [[ -n "$byte" ]] || continue
    index=$((byte % ${#charset}))
    password+="${charset:index:1}"
  done
  printf '%s' "$password"
}
generated_password=""
admin_password="${SATURN_ADMIN_PASSWORD:-}"
existing_credentials_in_sync=0
if [[ -s "$BASIC_AUTH_FILE" && -s "$REMOTE_AUTH_DROPIN_FILE" ]]; then
  credential_status="$("$ADMIN_PASSWORD_HELPER_SRC" status 2>/dev/null || true)"
  if grep -q '^sync_state=in_sync$' <<<"$credential_status"; then
    existing_credentials_in_sync=1
  else
    warn "Existing nginx and Saturn Remote credentials are not synchronized; replacing both together."
  fi
fi
if [[ -z "$admin_password" && "$existing_credentials_in_sync" -eq 0 ]]; then
  info "Creating HTTP basic auth credentials for admin user..."
  if [[ -t 0 ]]; then
    while true; do
      read -r -s -p "Enter admin password (at least ${PASSWORD_MIN_LEN} characters): " admin_password; echo
      read -r -s -p "Confirm admin password: " admin_password_confirm; echo
      if [[ "$admin_password" != "$admin_password_confirm" ]]; then
        warn "Passwords do not match. Try again."
        continue
      fi
      if [[ ${#admin_password} -lt ${PASSWORD_MIN_LEN} ]]; then
        warn "Password must be at least ${PASSWORD_MIN_LEN} characters."
        continue
      fi
      break
    done
  else
    admin_password="$(generate_readable_admin_password)"
    generated_password="$admin_password"
    warn "No TTY available; generated readable admin password."
  fi

fi

if [[ -n "${admin_password:-}" ]]; then
  info "Applying nginx and Saturn Remote credentials through the canonical password helper..."
  printf '%s\n' "$admin_password" | \
    SATURN_ADMIN_SKIP_SYSTEMD=1 "$ADMIN_PASSWORD_HELPER_SRC" set --restart none
  ok "Basic auth and Saturn Remote TLS credentials configured"
elif [[ "$existing_credentials_in_sync" -eq 1 ]]; then
  info "Reusing existing synchronized credential backends"
else
  err "Existing credential state is incomplete; nginx and Saturn Remote cannot be kept synchronized."
  err "Rerun with SATURN_ADMIN_PASSWORD=<at-least-five-characters> to repair both backends."
  exit 1
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
repo_root_systemd="$(systemd_env_escape "$DEFAULT_REPO_ROOT")"
cat >"$SERVICE_FILE" <<SERVICE
[Unit]
Description=Saturn Update Manager (Rust backend)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=HOME=${SERVICE_HOME}
Environment=SATURN_LOG_DIR=${SATURN_LOG_DIR}
Environment=PATH=${SERVICE_HOME}/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
Environment=CARGO_HOME=${SERVICE_HOME}/.cargo
Environment=RUSTUP_HOME=${SERVICE_HOME}/.rustup
Environment=SATURN_WEBROOT=${WEB_ROOT}
Environment=SATURN_CONFIG=${WEB_ROOT}/config.json
Environment=SATURN_ADDR=${SATURN_ADDR}
Environment="SATURN_REPO_ROOT=${repo_root_systemd}"
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
Environment=SATURN_READY_REQUIRE_BRIDGE=${SATURN_READY_REQUIRE_BRIDGE}
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
Environment=SATURN_WATCHDOG_TIMEOUT=10
Environment=SATURN_WATCHDOG_FAILURE_LIMIT=3
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

if env_flag_enabled "$SATURN_DEFER_FINAL_READINESS"; then
  info "Deferring target-aware readiness to the canonical provisioning orchestrator"
else
  info "Waiting for target-aware backend readiness endpoint..."
  healthy=0
  for _ in {1..40}; do
    if curl -fsS "http://${SATURN_ADDR}/readyz?expected_commit=${RUST_BUILD_COMMIT}" >/dev/null 2>&1; then
      healthy=1
      ok "Backend is ready at commit ${RUST_BUILD_COMMIT}"
      break
    fi
    sleep 0.25
  done
  if [[ $healthy -ne 1 ]]; then
    err "Backend readiness check failed for commit ${RUST_BUILD_COMMIT} at http://${SATURN_ADDR}/readyz"
    echo "[INFO] saturn-go.service status:"
    systemctl --no-pager --full status saturn-go.service || true
    echo "[INFO] Recent saturn-go.service logs:"
    journalctl -u saturn-go.service -n 40 --no-pager || true
    exit 1
  fi
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
