#!/usr/bin/env bash

set -euo pipefail

REPO_DIR="${REPO_DIR:-${HOME}/github/deskhpsdr}"
CHECK_ONLY=0

log(){ printf '[deskhpsdr-install-deps] %s\n' "$*"; }
die(){ printf '[deskhpsdr-install-deps] ERR: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: deskhpsdr-install-deps-on-current-image.sh [options]

Install Debian prerequisites required to build deskHPSDR on the current image.

Options:
  --repo PATH       deskHPSDR repo path used to detect legacy GPIO sources
  --check-only      report missing packages without installing them
  -h, --help        show this help
EOF
}

package_installed() {
  dpkg-query -W -f='${Status}\n' "$1" 2>/dev/null | grep -q "install ok installed"
}

detect_legacy_gpio_support() {
  if [[ -f "${REPO_DIR}/src/gpio.c" ]] && grep -q "GPIOD_VERSION" "${REPO_DIR}/Makefile" 2>/dev/null; then
    return 0
  fi
  return 1
}

remove_redundant_pulseaudio_daemon() {
  if package_installed pipewire-pulse && package_installed pulseaudio; then
    log "pipewire-pulse found; removing redundant pulseaudio daemon package"
    env DEBIAN_FRONTEND=noninteractive apt-get --yes remove pulseaudio
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      REPO_DIR="$2"
      shift 2
      ;;
    --check-only)
      CHECK_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "Unknown argument: $1"
      ;;
  esac
done

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  die "Run as root. Web updates should call this helper through passwordless sudo."
fi

if [[ -r /etc/os-release ]]; then
  # shellcheck disable=SC1091
  . /etc/os-release
else
  die "/etc/os-release missing; cannot determine base image"
fi

if [[ "${ID:-}" != "debian" && "${ID_LIKE:-}" != *debian* ]]; then
  die "This script currently expects a Debian-based image. Detected ID=${ID:-unknown}."
fi

case "${VERSION_CODENAME:-}" in
  trixie)
    WEBKIT_DEV_PKG="libwebkit2gtk-4.1-dev"
    ;;
  bookworm)
    WEBKIT_DEV_PKG="libwebkit2gtk-4.0-dev"
    ;;
  *)
    WEBKIT_DEV_PKG="libwebkit2gtk-4.1-dev"
    ;;
esac

DEBIAN_PACKAGES=(
  build-essential
  pkg-config
  make
  gcc
  g++
  git
  cmake
  autoconf
  perl
  autopoint
  gettext
  automake
  libtool
  dos2unix
  libzstd-dev
  python3-dev
  wget
  meson
  ninja-build
  clang
  llvm
  libfftw3-dev
  libgtk-3-dev
  "${WEBKIT_DEV_PKG}"
  libasound2-dev
  libssl-dev
  libcurl4-openssl-dev
  libusb-1.0-0-dev
  libi2c-dev
  libpulse-dev
  libpcap-dev
  libjson-c-dev
  gnome-themes-extra
  libaio-dev
  libavahi-client-dev
  libad9361-dev
  libiio-dev
  bison
  flex
  libxml2-dev
)

if detect_legacy_gpio_support; then
  DEBIAN_PACKAGES+=(libgpiod-dev)
fi

if ! package_installed pipewire-pulse && ! package_installed pulseaudio; then
  DEBIAN_PACKAGES+=(pipewire-pulse)
fi

missing_packages=()
for pkg in "${DEBIAN_PACKAGES[@]}"; do
  if ! package_installed "$pkg"; then
    missing_packages+=("$pkg")
  fi
done

if [[ ${#missing_packages[@]} -eq 0 ]]; then
  log "All deskHPSDR Debian prerequisites are installed."
  exit 0
fi

log "Missing deskHPSDR Debian prerequisites: ${missing_packages[*]}"

if [[ ${CHECK_ONLY} -eq 1 ]]; then
  exit 0
fi

log "Installing deskHPSDR prerequisites for ${PRETTY_NAME:-Debian-based image}"
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get --yes install "${missing_packages[@]}"
remove_redundant_pulseaudio_daemon
log "deskHPSDR prerequisite install complete."
