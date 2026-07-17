#!/usr/bin/env bash
# Canonical Saturn appliance installer for both manual and cloud-init installs.

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SATURN_REPO_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"
PROVISIONER="$REPO_ROOT/provision/cloud-init/provision-saturn.sh"
SATURN_USER="${SATURN_USER:-${SUDO_USER:-pi}}"
PROFILE="${SATURN_INSTALL_PROFILE:-appliance}"
DRY_RUN=0
NONINTERACTIVE="${SATURN_NONINTERACTIVE:-0}"
FORCE_REPROVISION="${SATURN_FORCE_REPROVISION:-0}"
INSTALL_PACKAGES="${SATURN_INSTALL_PACKAGES:-1}"
INSTALL_DRIVER="${SATURN_REBUILD_XDMA:-1}"
INSTALL_P2="${SATURN_INSTALL_P2APP_CONTROL:-1}"
INSTALL_GO="${SATURN_INSTALL_UPDATE_MANAGER:-1}"
INSTALL_PIHPSDR_INSTALLER="${SATURN_PIHPSDR_INSTALLER_ENABLED:-1}"
VERIFY_EXPLICIT=0
[[ -v SATURN_VERIFY_MODE ]] && VERIFY_EXPLICIT=1
VERIFY_MODE="${SATURN_VERIFY_MODE:-hardware}"

info(){ printf '[saturn-install] %s\n' "$*"; }
warn(){ printf '[saturn-install] WARN: %s\n' "$*" >&2; }
die(){ printf '[saturn-install] ERROR: %s\n' "$*" >&2; exit 1; }

bool_true(){
  case "${1:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

default_env(){
  local name="$1" value="$2"
  if [[ ! -v "$name" ]]; then
    printf -v "$name" '%s' "$value"
  fi
  export "${name?}"
}

usage(){
  cat <<'EOF'
Usage: sudo ./install.sh [options]

The default appliance profile auto-detects Saturn hardware and installs the
radio runtime, XDMA through DKMS, P2, Saturn Go, and Saturn Bridge.

Options:
  --user <name>          Runtime/build user (default: SUDO_USER or pi)
  --profile <name>       appliance (default), desktop, or image-factory
                         (software verification plus image-sealing tools)
  --non-interactive      Never prompt; generate a five-character initial password
  --force                Re-run all provisioning phases after a completed install
  --verify <mode>        hardware (default), software, or none
  --skip-packages        Require existing dependencies; do not run apt
  --skip-driver          Do not install/load XDMA through DKMS
  --skip-p2              Do not build/install p2app.service
  --skip-saturn-go       Do not install Saturn Go or Saturn Bridge
  --skip-verify          Alias for --verify none
  --dry-run              Print the resolved install contract without changing the host
  -h, --help             Show this help

Password policy: newly set passwords are at least five characters with no
composition rules. Generated passwords remain five characters. Existing
credentials are retained during upgrades.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --user) [[ $# -ge 2 ]] || die "--user requires a value"; SATURN_USER="$2"; shift 2 ;;
    --profile) [[ $# -ge 2 ]] || die "--profile requires a value"; PROFILE="$2"; shift 2 ;;
    --non-interactive) NONINTERACTIVE=1; shift ;;
    --force) FORCE_REPROVISION=1; shift ;;
    --verify) [[ $# -ge 2 ]] || die "--verify requires a mode"; VERIFY_MODE="$2"; VERIFY_EXPLICIT=1; shift 2 ;;
    --skip-packages) INSTALL_PACKAGES=0; shift ;;
    --skip-driver) INSTALL_DRIVER=0; shift ;;
    --skip-p2) INSTALL_P2=0; shift ;;
    --skip-saturn-go) INSTALL_GO=0; shift ;;
    --skip-verify) VERIFY_MODE=none; VERIFY_EXPLICIT=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

case "$PROFILE" in
  appliance)
    default_env SATURN_INSTALL_DEVELOPER_TOOLS 0
    default_env SATURN_INSTALL_PIHPSDR 0
    default_env SATURN_DESKTOP_UI auto
    default_env SATURN_BUILD_OPTIONAL_TOOLS 1
    ;;
  desktop)
    default_env SATURN_INSTALL_DEVELOPER_TOOLS 1
    default_env SATURN_INSTALL_PIHPSDR 1
    default_env SATURN_DESKTOP_UI auto
    default_env SATURN_BUILD_OPTIONAL_TOOLS 1
    ;;
  image-factory)
    default_env SATURN_INSTALL_DEVELOPER_TOOLS 0
    default_env SATURN_INSTALL_PIHPSDR 0
    default_env SATURN_DESKTOP_UI 0
    default_env SATURN_BUILD_OPTIONAL_TOOLS 1
    default_env SATURN_BRIDGE_VERIFY_RUNTIME 0
    default_env SATURN_INSTALL_CLOUD_INIT 1
    if (( ! VERIFY_EXPLICIT )) && [[ "$VERIFY_MODE" == hardware ]]; then
      VERIFY_MODE=software
    fi
    ;;
  *) die "unsupported profile '$PROFILE' (use appliance, desktop, or image-factory)" ;;
  esac

case "$VERIFY_MODE" in
  hardware|software|none) ;;
  *) die "unsupported verification mode '$VERIFY_MODE'" ;;
esac

[[ -f "$REPO_ROOT/sw_projects/P2_app/Makefile" ]] || die "invalid Saturn checkout: $REPO_ROOT"
[[ -x "$PROVISIONER" ]] || die "shared provisioner not found or executable: $PROVISIONER"
getent passwd "$SATURN_USER" >/dev/null || die "user does not exist: $SATURN_USER"
SATURN_HOME="$(getent passwd "$SATURN_USER" | cut -d: -f6)"
SATURN_GROUP="$(id -gn "$SATURN_USER")"
[[ -n "$SATURN_HOME" && -d "$SATURN_HOME" ]] || die "home directory is unavailable for $SATURN_USER"

if (( ! DRY_RUN )); then
  (( EUID == 0 )) || die "run as root (sudo), or use --dry-run"
  command -v apt-get >/dev/null 2>&1 || die "apt-get is required; install on Debian 13 (Trixie)"
  command -v systemctl >/dev/null 2>&1 || die "systemd is required"
  architecture="$(dpkg --print-architecture 2>/dev/null || uname -m)"
  [[ "$architecture" == arm64 || "$architecture" == aarch64 ]] || \
    die "unsupported architecture '$architecture'; Saturn appliance installs require arm64"
  if [[ -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    source /etc/os-release
    if [[ "${VERSION_CODENAME:-}" != trixie ]] && ! bool_true "${SATURN_ALLOW_UNSUPPORTED_OS:-0}"; then
      die "unsupported OS '${PRETTY_NAME:-unknown}'; use Debian/Raspberry Pi OS Trixie or set SATURN_ALLOW_UNSUPPORTED_OS=1"
    fi
  fi
fi

export SATURN_USER SATURN_GROUP
export SATURN_REPO_DIR="$REPO_ROOT"
export SATURN_REPO_SYNC=0
export SATURN_INSTALL_PROFILE="$PROFILE"
export SATURN_NONINTERACTIVE="$NONINTERACTIVE"
export SATURN_FORCE_REPROVISION="$FORCE_REPROVISION"
export SATURN_INSTALL_PACKAGES="$INSTALL_PACKAGES"
export SATURN_REBUILD_XDMA="$INSTALL_DRIVER"
export SATURN_INSTALL_P2APP_CONTROL="$INSTALL_P2"
export SATURN_INSTALL_UPDATE_MANAGER="$INSTALL_GO"
export SATURN_INSTALL_SATURN_BRIDGE="$INSTALL_GO"
export SATURN_REQUIRE_SATURN_BRIDGE="$INSTALL_GO"
export SATURN_PIHPSDR_INSTALLER_ENABLED="$INSTALL_PIHPSDR_INSTALLER"
export SATURN_VERIFY_MODE="$VERIFY_MODE"

info "repository: $REPO_ROOT"
info "runtime user: $SATURN_USER ($SATURN_GROUP)"
info "profile: $PROFILE"
info "packages=$INSTALL_PACKAGES driver=$INSTALL_DRIVER p2=$INSTALL_P2 saturn-go=$INSTALL_GO verify=$VERIFY_MODE pihpsdr-installer=$INSTALL_PIHPSDR_INSTALLER"

if (( DRY_RUN )); then
  info "dry-run: would execute shared provisioner: $PROVISIONER"
  exit 0
fi

exec bash "$PROVISIONER"
