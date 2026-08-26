#!/usr/bin/env bash
set -Eeuo pipefail

SATURN_USER="${SATURN_USER:-${SUDO_USER:-pi}}"
SATURN_USER_HOME="${SATURN_USER_HOME:-$(getent passwd "$SATURN_USER" 2>/dev/null | cut -d: -f6 || true)}"
SATURN_USER_HOME="${SATURN_USER_HOME:-/home/${SATURN_USER}}"
SATURN_REPO_ROOT="${SATURN_REPO_ROOT:-${SATURN_USER_HOME}/github/Saturn}"
SATURN_BRIDGE_SOURCE_DIR="${SATURN_BRIDGE_SOURCE_DIR:-${SATURN_REPO_ROOT}/update_manager/saturn-bridge}"
SATURN_GO_ROOT="${SATURN_GO_ROOT:-/opt/saturn-go}"
SATURN_BRIDGE_BIN="${SATURN_BRIDGE_BIN:-${SATURN_GO_ROOT}/bin/saturn-bridge}"
SATURN_BRIDGE_SERVICE="${SATURN_BRIDGE_SERVICE:-/etc/systemd/system/saturn-bridge.service}"
SATURN_BRIDGE_BUILD_PROFILE="${SATURN_BRIDGE_BUILD_PROFILE:-release}"
SATURN_BRIDGE_CARGO_TARGET_DIR="${SATURN_BRIDGE_CARGO_TARGET_DIR:-${SATURN_BRIDGE_SOURCE_DIR}/target-local}"
SATURN_BRIDGE_BUILD_TMP_DIR="${SATURN_BRIDGE_BUILD_TMP_DIR:-${SATURN_BRIDGE_SOURCE_DIR}/.tmp}"
SATURN_BRIDGE_BUILD_JOBS="${SATURN_BRIDGE_BUILD_JOBS:-${SATURN_SATURNGO_BUILD_JOBS:-1}}"
SATURN_BRIDGE_BUILD_NICE="${SATURN_BRIDGE_BUILD_NICE:-${SATURN_SATURNGO_BUILD_NICE:-15}}"
SATURN_BRIDGE_BUILD_IONICE_CLASS="${SATURN_BRIDGE_BUILD_IONICE_CLASS:-${SATURN_SATURNGO_BUILD_IONICE_CLASS:-3}}"
SATURN_BRIDGE_BUILD_SWAP_FILE="${SATURN_BRIDGE_BUILD_SWAP_FILE:-${SATURN_SATURNGO_BUILD_SWAP_FILE:-${SATURN_USER_HOME}/saturn-build.swap}}"
SATURN_BRIDGE_BUILD_SWAP_MIB="${SATURN_BRIDGE_BUILD_SWAP_MIB:-${SATURN_SATURNGO_BUILD_SWAP_MIB:-2048}}"
SATURN_BRIDGE_BUILD_PREFLIGHT_HELPER="${SATURN_BRIDGE_BUILD_PREFLIGHT_HELPER:-${SATURN_REPO_ROOT}/update_manager/scripts/saturn-go-build-preflight.sh}"
SATURN_BRIDGE_INSTALLED_PREFLIGHT_HELPER="${SATURN_BRIDGE_INSTALLED_PREFLIGHT_HELPER:-/usr/local/lib/saturn-go/scripts/saturn-go-build-preflight.sh}"
SATURN_BRIDGE_RF_TX_ENABLED="${SATURN_BRIDGE_RF_TX_ENABLED:-1}"
SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED="${SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED:-1}"
SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ="${SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ:-192}"
SATURN_BRIDGE_BUILD_ONLY="${SATURN_BRIDGE_BUILD_ONLY:-0}"
SATURN_BRIDGE_OUTPUT_BIN="${SATURN_BRIDGE_OUTPUT_BIN:-}"
SATURN_BRIDGE_WDSP_FLAVOR="${SATURN_BRIDGE_WDSP_FLAVOR:-wdsp2}"
SATURN_INSTALL_PACKAGES="${SATURN_INSTALL_PACKAGES:-1}"
SATURN_BRIDGE_VERIFY_RUNTIME="${SATURN_BRIDGE_VERIFY_RUNTIME:-1}"
SATURN_BRIDGE_BACKEND_STATE_FILE="${SATURN_BRIDGE_BACKEND_STATE_FILE:-/var/lib/saturn-radio-backend/selection.json}"
SATURN_BRIDGE_BACKEND_SWITCH_HELPER="${SATURN_BRIDGE_BACKEND_SWITCH_HELPER:-/usr/local/lib/saturn-go/scripts/saturn-radio-backend-switch-root.sh}"
SATURN_BRIDGE_PRESERVED_BACKEND="p2"
SATURN_BRIDGE_FFTW_WISDOM_SOURCE="${SATURN_BRIDGE_FFTW_WISDOM_SOURCE:-${SATURN_REPO_ROOT}/update_manager/scripts/saturn-fftw-wisdom.sh}"
SATURN_BRIDGE_FFTW_WISDOM_HELPER="${SATURN_BRIDGE_FFTW_WISDOM_HELPER:-/usr/local/lib/saturn-go/scripts/saturn-fftw-wisdom.sh}"
SATURN_BRIDGE_FFTW_WISDOM_CACHE_DIR="${SATURN_BRIDGE_FFTW_WISDOM_CACHE_DIR:-/var/cache/saturn-bridge}"
SATURN_BRIDGE_FFTW_WISDOM_PATH="${SATURN_BRIDGE_FFTW_WISDOM_PATH:-${SATURN_BRIDGE_FFTW_WISDOM_CACHE_DIR}/wdspWisdom01}"
SATURN_BRIDGE_FFTW_WISDOM_SERVICE="${SATURN_BRIDGE_FFTW_WISDOM_SERVICE:-/etc/systemd/system/saturn-fftw-wisdom.service}"
SATURN_BRIDGE_FFTW_WISDOM_TIMER="${SATURN_BRIDGE_FFTW_WISDOM_TIMER:-/etc/systemd/system/saturn-fftw-wisdom.timer}"
SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE="${SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE:-262144}"
SATURN_BRIDGE_FFTW_WISDOM_ON_CALENDAR="${SATURN_BRIDGE_FFTW_WISDOM_ON_CALENDAR:-Sun *-*-* 03:15:00}"
SATURN_BRIDGE_FFTW_WISDOM_GENERATE_ON_INSTALL="${SATURN_BRIDGE_FFTW_WISDOM_GENERATE_ON_INSTALL:-1}"

# These commits are part of the Saturn Bridge native build contract. Updating
# either pin requires rebuilding and re-running the WDSP/bridge test matrix.
SATURN_WDSP2_REPO_URL="${SATURN_WDSP2_REPO_URL:-https://github.com/TAPR/OpenHPSDR-wdsp.git}"
SATURN_WDSP2_REF="${SATURN_WDSP2_REF:-584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3}"
SATURN_PIHPSDR_PORT_REPO_URL="${SATURN_PIHPSDR_PORT_REPO_URL:-https://github.com/dl1ycf/pihpsdr.git}"
SATURN_PIHPSDR_PORT_REF="${SATURN_PIHPSDR_PORT_REF:-974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1}"
SATURN_BRIDGE_NATIVE_SOURCE_ROOT="${SATURN_BRIDGE_NATIVE_SOURCE_ROOT:-${SATURN_BRIDGE_CARGO_TARGET_DIR}/native-src}"
SATURN_WDSP2_REPO_DIR="${SATURN_WDSP2_REPO_DIR:-${SATURN_BRIDGE_NATIVE_SOURCE_ROOT}/OpenHPSDR-wdsp}"
SATURN_PIHPSDR_PORT_REPO_DIR="${SATURN_PIHPSDR_PORT_REPO_DIR:-${SATURN_BRIDGE_NATIVE_SOURCE_ROOT}/pihpsdr}"
SATURN_WDSP2_SOURCE_DIR="${SATURN_WDSP2_SOURCE_DIR:-${SATURN_WDSP2_REPO_DIR}/wdsp 2.00/Source}"
SATURN_PIHPSDR_WDSP_DIR="${SATURN_PIHPSDR_WDSP_DIR:-${SATURN_PIHPSDR_PORT_REPO_DIR}/wdsp}"
SATURN_WDSP2_BUILD_DIR="${SATURN_WDSP2_BUILD_DIR:-${SATURN_BRIDGE_CARGO_TARGET_DIR}/wdsp2-linux-arm}"

# Legacy opt-in only. The default installer does not need a prebuilt piHPSDR
# checkout and always uses the pinned WDSP 2.00 path above.
SATURN_PIHPSDR_DIR="${SATURN_PIHPSDR_DIR:-${SATURN_USER_HOME}/github/pihpsdr}"

log(){ printf '[install-saturn-bridge] %s\n' "$*"; }
die(){ printf '[install-saturn-bridge] ERROR: %s\n' "$*" >&2; exit 1; }

flag_enabled() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

need_root() {
  [[ ${EUID:-$(id -u)} -eq 0 ]] || die "Run as root."
}

need_file() {
  [[ -f "$1" ]] || die "$2 not found: $1"
}

need_dir() {
  [[ -d "$1" ]] || die "$2 not found: $1"
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

require_positive_integer() {
  local name="$1" value="$2"
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || die "$name must be a positive integer, got: $value"
}

require_nonnegative_integer() {
  local name="$1" value="$2"
  [[ "$value" =~ ^[0-9]+$ ]] || die "$name must be a non-negative integer, got: $value"
}

apt_pkg_installed() {
  dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q '^install ok installed$'
}

ensure_apt_packages() {
  local missing=()
  local pkg
  for pkg in build-essential binutils pkg-config libfftw3-dev libopus0 ca-certificates git python3; do
    apt_pkg_installed "$pkg" || missing+=("$pkg")
  done
  if (( ${#missing[@]} == 0 )); then
    log "Bridge system dependencies already installed."
    return 0
  fi
  if ! flag_enabled "$SATURN_INSTALL_PACKAGES"; then
    die "Missing bridge system dependencies while SATURN_INSTALL_PACKAGES=0: ${missing[*]}"
  fi
  log "Installing bridge system dependencies: ${missing[*]}"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y --no-install-recommends "${missing[@]}"
}

bridge_build_user() {
  if id -u "$SATURN_USER" >/dev/null 2>&1; then
    printf '%s\n' "$SATURN_USER"
  else
    printf 'root\n'
  fi
}

run_as_bridge_user() {
  local build_user build_home cargo_home rustup_home path_prefix
  build_user="$(bridge_build_user)"
  build_home="$(getent passwd "$build_user" | cut -d: -f6)"
  [[ -n "$build_home" && -d "$build_home" ]] || die "Cannot resolve home for build user: $build_user"
  cargo_home="${CARGO_HOME:-${build_home}/.cargo}"
  rustup_home="${RUSTUP_HOME:-${build_home}/.rustup}"
  path_prefix="${cargo_home}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

  if [[ "$(id -u)" -eq "$(id -u "$build_user")" ]]; then
    env HOME="$build_home" CARGO_HOME="$cargo_home" RUSTUP_HOME="$rustup_home" PATH="$path_prefix" "$@"
  elif [[ "$(id -u)" -eq 0 ]]; then
    runuser -u "$build_user" -- env \
      HOME="$build_home" CARGO_HOME="$cargo_home" RUSTUP_HOME="$rustup_home" PATH="$path_prefix" "$@"
  else
    die "Cannot run build as $build_user from user $(id -un)"
  fi
}

ensure_build_directories() {
  local build_user build_group
  build_user="$(bridge_build_user)"
  build_group="$(id -gn "$build_user")"
  if [[ "$(id -u)" -eq 0 ]]; then
    install -d -m 0755 -o "$build_user" -g "$build_group" \
      "$SATURN_BRIDGE_CARGO_TARGET_DIR" "$SATURN_BRIDGE_NATIVE_SOURCE_ROOT" \
      "$SATURN_BRIDGE_BUILD_TMP_DIR"
  else
    mkdir -p "$SATURN_BRIDGE_CARGO_TARGET_DIR" "$SATURN_BRIDGE_NATIVE_SOURCE_ROOT" \
      "$SATURN_BRIDGE_BUILD_TMP_DIR"
  fi
}

ensure_low_memory_build_capacity() {
  local build_user helper
  build_user="$(bridge_build_user)"
  helper="$SATURN_BRIDGE_BUILD_PREFLIGHT_HELPER"
  require_positive_integer SATURN_BRIDGE_BUILD_JOBS "$SATURN_BRIDGE_BUILD_JOBS"
  require_positive_integer SATURN_BRIDGE_BUILD_SWAP_MIB "$SATURN_BRIDGE_BUILD_SWAP_MIB"
  require_nonnegative_integer SATURN_BRIDGE_BUILD_NICE "$SATURN_BRIDGE_BUILD_NICE"
  [[ "$SATURN_BRIDGE_BUILD_IONICE_CLASS" =~ ^[0-3]$ ]] \
    || die "SATURN_BRIDGE_BUILD_IONICE_CLASS must be between 0 and 3, got: $SATURN_BRIDGE_BUILD_IONICE_CLASS"
  need_file "$helper" "Rust build preflight helper"

  if [[ "$(id -u)" -eq 0 ]]; then
    env \
      SATURN_SATURNGO_BUILD_USER="$build_user" \
      SATURN_SATURNGO_BUILD_SWAP_FILE="$SATURN_BRIDGE_BUILD_SWAP_FILE" \
      SATURN_SATURNGO_BUILD_SWAP_MIB="$SATURN_BRIDGE_BUILD_SWAP_MIB" \
      bash "$helper" ensure-swap
  else
    if ! env \
      SATURN_SATURNGO_BUILD_USER="$build_user" \
      SATURN_SATURNGO_BUILD_SWAP_FILE="$SATURN_BRIDGE_BUILD_SWAP_FILE" \
      SATURN_SATURNGO_BUILD_SWAP_MIB="$SATURN_BRIDGE_BUILD_SWAP_MIB" \
      bash "$helper" ensure-swap
    then
      [[ -x "$SATURN_BRIDGE_INSTALLED_PREFLIGHT_HELPER" ]] \
        || die "Build swap is inactive and the installed privileged preflight helper is unavailable: $SATURN_BRIDGE_INSTALLED_PREFLIGHT_HELPER"
      need_cmd sudo
      sudo -n "$SATURN_BRIDGE_INSTALLED_PREFLIGHT_HELPER" ensure-swap
    fi
  fi

  log "Rust build settings: CARGO_BUILD_JOBS=$SATURN_BRIDGE_BUILD_JOBS TMPDIR=$SATURN_BRIDGE_BUILD_TMP_DIR CARGO_TARGET_DIR=$SATURN_BRIDGE_CARGO_TARGET_DIR nice -n $SATURN_BRIDGE_BUILD_NICE ionice -c $SATURN_BRIDGE_BUILD_IONICE_CLASS"
}

ensure_pinned_sparse_checkout() {
  local url="$1"
  local ref="$2"
  local dest="$3"
  shift 3
  local current=""
  local expected=""

  if [[ -d "$dest/.git" ]]; then
    current="$(git -C "$dest" rev-parse HEAD 2>/dev/null || true)"
    if [[ "$current" == "$ref" ]]; then
      log "Pinned native source ready: $dest @ $ref"
      return 0
    fi
  elif [[ -e "$dest" ]]; then
    die "Native source cache exists but is not a git checkout: $dest"
  fi

  log "Provisioning pinned native source: $url @ $ref"
  if [[ ! -d "$dest/.git" ]]; then
    run_as_bridge_user git init -q "$dest"
    run_as_bridge_user git -C "$dest" remote add origin "$url"
  else
    run_as_bridge_user git -C "$dest" remote set-url origin "$url"
  fi
  run_as_bridge_user git -C "$dest" sparse-checkout init --no-cone
  run_as_bridge_user git -C "$dest" sparse-checkout set --no-cone "$@"
  run_as_bridge_user git -C "$dest" fetch --depth 1 origin "$ref"
  expected="$(git -C "$dest" rev-parse FETCH_HEAD 2>/dev/null || true)"
  [[ -n "$expected" ]] || die "Could not resolve pinned source ref for $dest: $ref"
  run_as_bridge_user git -C "$dest" checkout -q --detach FETCH_HEAD
  current="$(git -C "$dest" rev-parse HEAD 2>/dev/null || true)"
  [[ "$current" == "$expected" ]] || die "Pinned checkout mismatch for $dest: expected $expected, got ${current:-unknown}"
}

prepare_wdsp2_sources() {
  ensure_pinned_sparse_checkout \
    "$SATURN_WDSP2_REPO_URL" "$SATURN_WDSP2_REF" "$SATURN_WDSP2_REPO_DIR" \
    '/wdsp 2.00/Source/'
  ensure_pinned_sparse_checkout \
    "$SATURN_PIHPSDR_PORT_REPO_URL" "$SATURN_PIHPSDR_PORT_REF" "$SATURN_PIHPSDR_PORT_REPO_DIR" \
    '/wdsp/linux_port.c' '/wdsp/linux_port.h'

  need_dir "$SATURN_WDSP2_SOURCE_DIR" "WDSP 2.00 source directory"
  need_file "$SATURN_PIHPSDR_WDSP_DIR/linux_port.c" "piHPSDR Linux port source"
  need_file "$SATURN_PIHPSDR_WDSP_DIR/linux_port.h" "piHPSDR Linux port header"
}

verify_wdsp2_archive() {
  local archive="$SATURN_WDSP2_BUILD_DIR/libwdsp.a"
  local symbols
  need_file "$archive" "WDSP 2.00 archive"
  symbols="$(nm -g --defined-only "$archive")"
  local symbol
  for symbol in \
    SetRXAWBFMdmph GetRXAWBFMStereoIndicator \
    SetTXAPHROTAutoMode SetTXAPHROTRun \
    pscc SetPSMox SetPSControl GetPSInfo SetPSFeedbackRate
  do
    grep -Eq "[[:space:]]${symbol}$" <<<"$symbols" || die "WDSP 2.00 archive is missing required symbol: $symbol"
  done
  log "WDSP 2.00 archive symbol verification passed."
}

build_wdsp2() {
  local helper="$SATURN_BRIDGE_SOURCE_DIR/scripts/build-wdsp2-linux-arm.sh"
  need_file "$helper" "WDSP 2.00 Linux/ARM build helper"
  [[ -x "$helper" ]] || die "WDSP 2.00 build helper is not executable: $helper"
  prepare_wdsp2_sources
  log "Building pinned WDSP 2.00 Linux/ARM archive"
  run_as_bridge_user env \
    WDSP2_SOURCE_DIR="$SATURN_WDSP2_SOURCE_DIR" \
    PIHPSDR_WDSP_DIR="$SATURN_PIHPSDR_WDSP_DIR" \
    WDSP2_BUILD_DIR="$SATURN_WDSP2_BUILD_DIR" \
    bash "$helper"
  verify_wdsp2_archive
}

verify_bridge_inputs() {
  need_dir "$SATURN_BRIDGE_SOURCE_DIR" "saturn-bridge source directory"
  need_file "$SATURN_BRIDGE_SOURCE_DIR/Cargo.toml" "saturn-bridge Cargo manifest"
  need_cmd git
  need_cmd ionice
  need_cmd flock
  need_cmd nm
  need_cmd nice
  need_cmd pkg-config
  need_cmd python3
  pkg-config --exists fftw3 || die "fftw3 development package is required"
  need_file "$SATURN_BRIDGE_FFTW_WISDOM_SOURCE" "Saturn FFTW wisdom helper"
}

build_bridge() {
  local cargo_args=(build)
  local native_env=()
  if [[ "$SATURN_BRIDGE_BUILD_PROFILE" == "release" ]]; then
    cargo_args+=(--release)
  fi

  case "$SATURN_BRIDGE_WDSP_FLAVOR" in
    wdsp2|2.00)
      build_wdsp2
      native_env+=(SATURN_WDSP_DIR="$SATURN_WDSP2_BUILD_DIR")
      ;;
    pihpsdr|legacy)
      need_file "$SATURN_PIHPSDR_DIR/wdsp/libwdsp.a" "piHPSDR WDSP archive"
      need_file "$SATURN_PIHPSDR_DIR/rnnoise/librnnoise.a" "piHPSDR rnnoise archive"
      need_file "$SATURN_PIHPSDR_DIR/libspecbleach/libspecbleach.a" "piHPSDR specbleach archive"
      native_env+=(SATURN_PIHPSDR_DIR="$SATURN_PIHPSDR_DIR")
      ;;
    *)
      die "Unsupported SATURN_BRIDGE_WDSP_FLAVOR: $SATURN_BRIDGE_WDSP_FLAVOR"
      ;;
  esac

  log "Building saturn-bridge ($SATURN_BRIDGE_BUILD_PROFILE, WDSP=$SATURN_BRIDGE_WDSP_FLAVOR)"
  run_as_bridge_user env \
    CARGO_BUILD_JOBS="$SATURN_BRIDGE_BUILD_JOBS" \
    CARGO_TARGET_DIR="$SATURN_BRIDGE_CARGO_TARGET_DIR" \
    TMPDIR="$SATURN_BRIDGE_BUILD_TMP_DIR" \
    "${native_env[@]}" \
    nice -n "$SATURN_BRIDGE_BUILD_NICE" \
    ionice -c "$SATURN_BRIDGE_BUILD_IONICE_CLASS" \
    cargo "${cargo_args[@]}" -j "$SATURN_BRIDGE_BUILD_JOBS" \
      --manifest-path "$SATURN_BRIDGE_SOURCE_DIR/Cargo.toml"
}

built_bridge_path() {
  if [[ "$SATURN_BRIDGE_BUILD_PROFILE" == "release" ]]; then
    printf '%s/release/saturn-bridge\n' "$SATURN_BRIDGE_CARGO_TARGET_DIR"
  else
    printf '%s/debug/saturn-bridge\n' "$SATURN_BRIDGE_CARGO_TARGET_DIR"
  fi
}

verify_built_bridge() {
  local built_bin
  built_bin="$(built_bridge_path)"
  need_file "$built_bin" "built saturn-bridge binary"
  if [[ "$SATURN_BRIDGE_WDSP_FLAVOR" == "wdsp2" || "$SATURN_BRIDGE_WDSP_FLAVOR" == "2.00" ]]; then
    local symbol symbols
    symbols="$(nm -a "$built_bin")"
    for symbol in SetRXAWBFMdmph SetTXAPHROTAutoMode pscc SetPSControl; do
      grep -Eq "[[:space:]]${symbol}$" <<<"$symbols" || die "Built bridge is missing WDSP 2.00 symbol: $symbol"
    done
  fi
}

copy_build_only_output() {
  local built_bin
  built_bin="$(built_bridge_path)"
  [[ -n "$SATURN_BRIDGE_OUTPUT_BIN" ]] || die "SATURN_BRIDGE_OUTPUT_BIN is required in build-only mode"
  install -D -m 0755 "$built_bin" "$SATURN_BRIDGE_OUTPUT_BIN"
  log "Staged bridge binary: $SATURN_BRIDGE_OUTPUT_BIN"
}

install_binary() {
  local built_bin
  built_bin="$(built_bridge_path)"
  install -d -m 0755 "$(dirname "$SATURN_BRIDGE_BIN")"
  install -m 0755 -o root -g root "$built_bin" "$SATURN_BRIDGE_BIN"
  log "Installed bridge binary: $SATURN_BRIDGE_BIN"
}

install_fftw_wisdom_maintenance() {
  local service_name timer_name
  service_name="$(basename "$SATURN_BRIDGE_FFTW_WISDOM_SERVICE")"
  timer_name="$(basename "$SATURN_BRIDGE_FFTW_WISDOM_TIMER")"

  install -D -m 0755 -o root -g root \
    "$SATURN_BRIDGE_FFTW_WISDOM_SOURCE" "$SATURN_BRIDGE_FFTW_WISDOM_HELPER"
  install -d -m 0755 -o root -g root "$SATURN_BRIDGE_FFTW_WISDOM_CACHE_DIR"

  cat >"$SATURN_BRIDGE_FFTW_WISDOM_SERVICE" <<EOF
[Unit]
Description=Check and refresh Saturn Bridge FFTW wisdom
Documentation=https://github.com/kd4yal2024/Saturn
ConditionFileIsExecutable=${SATURN_BRIDGE_BIN}

[Service]
Type=oneshot
ExecStart=${SATURN_BRIDGE_FFTW_WISDOM_HELPER} --check
Environment=SATURN_FFTW_WISDOM_BRIDGE_BIN=${SATURN_BRIDGE_BIN}
Environment=SATURN_FFTW_WISDOM_CACHE_DIR=${SATURN_BRIDGE_FFTW_WISDOM_CACHE_DIR}
Environment=SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE=${SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE}
Nice=19
IOSchedulingClass=idle
CPUWeight=10
TimeoutStartSec=6h
NoNewPrivileges=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=${SATURN_BRIDGE_FFTW_WISDOM_CACHE_DIR}
RestrictAddressFamilies=AF_UNIX
LockPersonality=yes
RestrictSUIDSGID=yes
EOF

  cat >"$SATURN_BRIDGE_FFTW_WISDOM_TIMER" <<EOF
[Unit]
Description=Periodically verify Saturn Bridge FFTW wisdom fingerprint

[Timer]
OnCalendar=${SATURN_BRIDGE_FFTW_WISDOM_ON_CALENDAR}
RandomizedDelaySec=45m
Persistent=true
Unit=${service_name}

[Install]
WantedBy=timers.target
EOF

  chmod 0644 "$SATURN_BRIDGE_FFTW_WISDOM_SERVICE" "$SATURN_BRIDGE_FFTW_WISDOM_TIMER"
  systemctl daemon-reload
  systemctl enable "$timer_name"
  if systemctl is-active --quiet "$timer_name"; then
    systemctl restart "$timer_name"
  else
    systemctl start "$timer_name"
  fi
  log "Installed FFTW wisdom fingerprint timer: $timer_name"

  if flag_enabled "$SATURN_BRIDGE_FFTW_WISDOM_GENERATE_ON_INSTALL"; then
    log "Checking machine-local FFTW wisdom during installation"
    if ! env \
      SATURN_FFTW_WISDOM_BRIDGE_BIN="$SATURN_BRIDGE_BIN" \
      SATURN_FFTW_WISDOM_CACHE_DIR="$SATURN_BRIDGE_FFTW_WISDOM_CACHE_DIR" \
      SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE="$SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE" \
      "$SATURN_BRIDGE_FFTW_WISDOM_HELPER" --check
    then
      log "WARNING: FFTW wisdom generation failed; Saturn Bridge will use safe runtime planning and the timer will retry"
    fi
  else
    log "Skipping install-time FFTW wisdom generation; periodic fingerprint checks remain enabled"
  fi
}

capture_selected_backend() {
  local selected=""
  if [[ -r "$SATURN_BRIDGE_BACKEND_STATE_FILE" ]]; then
    selected="$(python3 - "$SATURN_BRIDGE_BACKEND_STATE_FILE" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        value = json.load(handle).get("active", "")
except (OSError, TypeError, ValueError):
    value = ""
print(value if value in {"p2", "xdma"} else "")
PY
)"
  fi
  if [[ -z "$selected" ]] && systemctl cat "$(basename "$SATURN_BRIDGE_SERVICE")" >/dev/null 2>&1; then
    local environment
    environment="$(systemctl show --property=Environment --value "$(basename "$SATURN_BRIDGE_SERVICE")" 2>/dev/null || true)"
    case " $environment " in
      *" SATURN_BRIDGE_RADIO_BACKEND=xdma "*) selected="xdma" ;;
      *" SATURN_BRIDGE_RADIO_BACKEND=p2 "*) selected="p2" ;;
    esac
  fi
  SATURN_BRIDGE_PRESERVED_BACKEND="${selected:-p2}"
  log "Preserving appliance radio backend: $SATURN_BRIDGE_PRESERVED_BACKEND"
}

install_service() {
  local service_user service_group service_name
  service_user="$(bridge_build_user)"
  service_group="$(id -gn "$service_user")"
  service_name="$(basename "$SATURN_BRIDGE_SERVICE")"
  cat >"$SATURN_BRIDGE_SERVICE" <<EOF
[Unit]
Description=Saturn Bridge (WDSP 2.00)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=${service_user}
Group=${service_group}
WorkingDirectory=${SATURN_GO_ROOT}
ExecStart=${SATURN_BRIDGE_BIN}
Restart=on-failure
RestartSec=2
RuntimeDirectory=saturn-bridge
RuntimeDirectoryMode=0750
LimitRTPRIO=21
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
RestrictSUIDSGID=yes
LockPersonality=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
Environment=SATURN_BRIDGE_RADIO_HOST=127.0.0.1
Environment=SATURN_BRIDGE_RADIO_PORT=1024
Environment=SATURN_BRIDGE_RADIO_BACKEND=p2
Environment=SATURN_BRIDGE_XDMA_READY_PATH=/run/saturn-bridge/xdma-ready.json
Environment=SATURN_BRIDGE_FFTW_WISDOM_PATH=${SATURN_BRIDGE_FFTW_WISDOM_PATH}
Environment=SATURN_BRIDGE_CLIENT_HOST=127.0.0.1
Environment=SATURN_BRIDGE_CLIENT_PORT=12000
Environment=SATURN_BRIDGE_TCI_HOST=127.0.0.1
Environment=SATURN_BRIDGE_TCI_PORT=50001
Environment=SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ=${SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ}
Environment=SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED=${SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED}
Environment=SATURN_REMOTE_TX_RF_ENABLED=${SATURN_BRIDGE_RF_TX_ENABLED}

[Install]
WantedBy=multi-user.target
EOF
  chmod 0644 "$SATURN_BRIDGE_SERVICE"
  systemctl daemon-reload
  # The backend transaction owns both the active and boot-time service policy.
  # P2 is deliberately P2app-only at boot; the browser bridge is started on
  # demand. Direct XDMA instead enables the bridge and disables P2app.
  if [[ -x "$SATURN_BRIDGE_BACKEND_SWITCH_HELPER" ]]; then
    "$SATURN_BRIDGE_BACKEND_SWITCH_HELPER" switch "$SATURN_BRIDGE_PRESERVED_BACKEND"
    log "Reapplied preserved backend '$SATURN_BRIDGE_PRESERVED_BACKEND' through the transactional owner switch"
  else
    systemctl restart "$service_name"
    log "Enabled and restarted $service_name (backend broker not installed)"
  fi
}

verify_runtime() {
  local service_name environment runtime_backend="unknown"
  service_name="$(basename "$SATURN_BRIDGE_SERVICE")"
  environment="$(systemctl show --property=Environment --value "$service_name")"
  case " $environment " in
    *" SATURN_BRIDGE_RADIO_BACKEND=xdma "*) runtime_backend="xdma" ;;
    *" SATURN_BRIDGE_RADIO_BACKEND=p2 "*) runtime_backend="p2" ;;
  esac
  [[ "$runtime_backend" == "$SATURN_BRIDGE_PRESERVED_BACKEND" ]] \
    || die "saturn-bridge backend changed during install: preserved=$SATURN_BRIDGE_PRESERVED_BACKEND runtime=$runtime_backend"
  if [[ "$SATURN_BRIDGE_PRESERVED_BACKEND" == "p2" ]]; then
    systemctl is-active --quiet p2app.service \
      || die "p2app.service is not active after applying the P2 startup policy"
    ! systemctl is-active --quiet "$service_name" \
      || die "saturn-bridge must be stopped after applying the P2-only startup policy"
    ! systemctl is-enabled --quiet "$service_name" \
      || die "saturn-bridge must be disabled for P2-only startup"
    log "saturn-bridge installation check passed; P2-only startup policy is active."
  else
    if ! systemctl is-active --quiet "$service_name"; then
      systemctl --no-pager status "$service_name" || true
      die "saturn-bridge service is not active after install"
    fi
    if command -v ss >/dev/null 2>&1 && ! ss -ltn | grep -q ':50001 '; then
      systemctl --no-pager status "$service_name" || true
      die "saturn-bridge is active but TCI port 127.0.0.1:50001 is not listening"
    fi
    log "saturn-bridge runtime check passed."
  fi
}

main() {
  if flag_enabled "$SATURN_BRIDGE_BUILD_ONLY"; then
    verify_bridge_inputs
    ensure_build_directories
    ensure_low_memory_build_capacity
    build_bridge
    verify_built_bridge
    copy_build_only_output
    return 0
  fi

  need_root
  ensure_apt_packages
  verify_bridge_inputs
  ensure_build_directories
  ensure_low_memory_build_capacity
  build_bridge
  verify_built_bridge
  capture_selected_backend
  install_binary
  install_fftw_wisdom_maintenance
  install_service
  if flag_enabled "$SATURN_BRIDGE_VERIFY_RUNTIME"; then
    verify_runtime
  else
    log "Skipping live bridge verification (SATURN_BRIDGE_VERIFY_RUNTIME=0)"
  fi
}

main "$@"
