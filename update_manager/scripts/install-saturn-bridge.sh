#!/usr/bin/env bash
set -Eeuo pipefail

SATURN_USER="${SATURN_USER:-${SUDO_USER:-pi}}"
SATURN_REPO_ROOT="${SATURN_REPO_ROOT:-/home/${SATURN_USER}/github/Saturn}"
SATURN_PIHPSDR_DIR="${SATURN_PIHPSDR_DIR:-/home/${SATURN_USER}/github/pihpsdr}"
SATURN_BRIDGE_SOURCE_DIR="${SATURN_BRIDGE_SOURCE_DIR:-${SATURN_REPO_ROOT}/update_manager/saturn-bridge}"
SATURN_GO_ROOT="${SATURN_GO_ROOT:-/opt/saturn-go}"
SATURN_BRIDGE_BIN="${SATURN_BRIDGE_BIN:-${SATURN_GO_ROOT}/bin/saturn-bridge}"
SATURN_BRIDGE_SERVICE="${SATURN_BRIDGE_SERVICE:-/etc/systemd/system/saturn-bridge.service}"
SATURN_BRIDGE_BUILD_PROFILE="${SATURN_BRIDGE_BUILD_PROFILE:-release}"
SATURN_BRIDGE_CARGO_TARGET_DIR="${SATURN_BRIDGE_CARGO_TARGET_DIR:-${SATURN_BRIDGE_SOURCE_DIR}/target-local}"
SATURN_BRIDGE_RF_TX_ENABLED="${SATURN_BRIDGE_RF_TX_ENABLED:-0}"
SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED="${SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED:-1}"
SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ="${SATURN_BRIDGE_MAX_CLIENT_DDC0_SAMPLE_RATE_KHZ:-192}"

log(){ printf '[install-saturn-bridge] %s\n' "$*"; }
die(){ printf '[install-saturn-bridge] ERROR: %s\n' "$*" >&2; exit 1; }

need_root() {
  [[ ${EUID:-$(id -u)} -eq 0 ]] || die "Run as root."
}

need_file() {
  [[ -f "$1" ]] || die "$2 not found: $1"
}

need_dir() {
  [[ -d "$1" ]] || die "$2 not found: $1"
}

apt_pkg_installed() {
  dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q '^install ok installed$'
}

ensure_apt_packages() {
  local missing=()
  local pkg
  for pkg in build-essential pkg-config libfftw3-dev ca-certificates; do
    apt_pkg_installed "$pkg" || missing+=("$pkg")
  done
  if (( ${#missing[@]} == 0 )); then
    log "Bridge system dependencies already installed."
    return 0
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
  local build_user build_home cargo_home rustup_home path_prefix cmd arg
  build_user="$(bridge_build_user)"
  build_home="$(getent passwd "$build_user" | cut -d: -f6)"
  [[ -n "$build_home" && -d "$build_home" ]] || die "Cannot resolve home for bridge build user: $build_user"
  cargo_home="${CARGO_HOME:-${build_home}/.cargo}"
  rustup_home="${RUSTUP_HOME:-${build_home}/.rustup}"
  path_prefix="${cargo_home}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  printf -v cmd 'cd %q && HOME=%q CARGO_HOME=%q RUSTUP_HOME=%q PATH=%q CARGO_TARGET_DIR=%q SATURN_PIHPSDR_DIR=%q cargo' \
    "$SATURN_BRIDGE_SOURCE_DIR" "$build_home" "$cargo_home" "$rustup_home" "$path_prefix" "$SATURN_BRIDGE_CARGO_TARGET_DIR" "$SATURN_PIHPSDR_DIR"
  for arg in "$@"; do
    printf -v cmd '%s %q' "$cmd" "$arg"
  done
  if [[ "$build_user" == "root" ]]; then
    bash -lc "$cmd"
  else
    runuser -u "$build_user" -- bash -lc "$cmd"
  fi
}

verify_native_inputs() {
  need_dir "$SATURN_BRIDGE_SOURCE_DIR" "saturn-bridge source directory"
  need_file "${SATURN_BRIDGE_SOURCE_DIR}/Cargo.toml" "saturn-bridge Cargo manifest"
  need_dir "$SATURN_PIHPSDR_DIR" "piHPSDR checkout"
  need_file "${SATURN_PIHPSDR_DIR}/wdsp/libwdsp.a" "WDSP static library"
  need_file "${SATURN_PIHPSDR_DIR}/rnnoise/librnnoise.a" "rnnoise static library"
  need_file "${SATURN_PIHPSDR_DIR}/libspecbleach/libspecbleach.a" "specbleach static library"
}

build_bridge() {
  local cargo_bin
  cargo_bin="$(getent passwd "$(bridge_build_user)" | cut -d: -f6)/.cargo/bin/cargo"
  [[ -x "$cargo_bin" || -x /usr/bin/cargo || -x /usr/local/bin/cargo ]] || die "cargo not found for bridge build. Run Saturn Go installer first."

  log "Building saturn-bridge (${SATURN_BRIDGE_BUILD_PROFILE})"
  if [[ "$SATURN_BRIDGE_BUILD_PROFILE" == "release" ]]; then
    run_as_bridge_user build --release
  else
    run_as_bridge_user build
  fi
}

install_binary() {
  local built_bin
  if [[ "$SATURN_BRIDGE_BUILD_PROFILE" == "release" ]]; then
    built_bin="${SATURN_BRIDGE_CARGO_TARGET_DIR}/release/saturn-bridge"
  else
    built_bin="${SATURN_BRIDGE_CARGO_TARGET_DIR}/debug/saturn-bridge"
  fi
  need_file "$built_bin" "built saturn-bridge binary"
  install -d -m 0755 "$(dirname "$SATURN_BRIDGE_BIN")"
  install -m 0755 -o root -g root "$built_bin" "$SATURN_BRIDGE_BIN"
  log "Installed bridge binary: $SATURN_BRIDGE_BIN"
}

install_service() {
  local service_user service_group
  service_user="$(bridge_build_user)"
  service_group="$(id -gn "$service_user")"
  cat >"$SATURN_BRIDGE_SERVICE" <<EOF
[Unit]
Description=Saturn Bridge
After=network-online.target p2app.service
Wants=network-online.target p2app.service

[Service]
Type=simple
User=${service_user}
Group=${service_group}
WorkingDirectory=${SATURN_BRIDGE_SOURCE_DIR}
ExecStart=${SATURN_BRIDGE_BIN}
Restart=on-failure
RestartSec=2
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
Environment=SATURN_BRIDGE_RADIO_HOST=127.0.0.1
Environment=SATURN_BRIDGE_RADIO_PORT=1024
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
  systemctl enable --now "$(basename "$SATURN_BRIDGE_SERVICE")"
  log "Enabled and started $(basename "$SATURN_BRIDGE_SERVICE")"
}

verify_runtime() {
  if ! systemctl is-active --quiet "$(basename "$SATURN_BRIDGE_SERVICE")"; then
    systemctl --no-pager status "$(basename "$SATURN_BRIDGE_SERVICE")" || true
    die "saturn-bridge service is not active after install."
  fi
  if command -v ss >/dev/null 2>&1 && ! ss -ltn | grep -q ':50001 '; then
    systemctl --no-pager status "$(basename "$SATURN_BRIDGE_SERVICE")" || true
    die "saturn-bridge is active but TCI socket 127.0.0.1:50001 is not listening."
  fi
  log "saturn-bridge runtime check passed."
}

main() {
  need_root
  ensure_apt_packages
  verify_native_inputs
  build_bridge
  install_binary
  install_service
  verify_runtime
}

main "$@"
