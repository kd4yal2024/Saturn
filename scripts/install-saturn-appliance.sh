#!/usr/bin/env bash
# Install a complete Saturn radio appliance from an existing repository checkout.

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SATURN_REPO_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"
SATURN_USER="${SATURN_USER:-${SUDO_USER:-pi}}"
DRY_RUN=0
INSTALL_PACKAGES=1
INSTALL_DRIVER=1
INSTALL_P2=1
INSTALL_GO=1
VERIFY=1

info(){ printf '[saturn-appliance] %s\n' "$*"; }
die(){ printf '[saturn-appliance] ERROR: %s\n' "$*" >&2; exit 1; }
run(){
  if (( DRY_RUN )); then
    printf '[dry-run]'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

usage(){
  cat <<'EOF'
Usage: sudo scripts/install-saturn-appliance.sh [options]

Options:
  --user <name>       Runtime/build user (default: SUDO_USER or pi)
  --skip-packages     Do not install apt dependencies
  --skip-driver       Do not install/load XDMA through DKMS
  --skip-p2           Do not build/install p2app.service
  --skip-saturn-go    Do not install Saturn Go and Saturn Bridge
  --skip-verify       Do not run final service/device checks
  --dry-run           Print operations without changing the system
  -h, --help          Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --user) [[ $# -ge 2 ]] || die "--user requires a value"; SATURN_USER="$2"; shift 2 ;;
    --skip-packages) INSTALL_PACKAGES=0; shift ;;
    --skip-driver) INSTALL_DRIVER=0; shift ;;
    --skip-p2) INSTALL_P2=0; shift ;;
    --skip-saturn-go) INSTALL_GO=0; shift ;;
    --skip-verify) VERIFY=0; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

(( DRY_RUN || EUID == 0 )) || die "run as root, or use --dry-run"
getent passwd "$SATURN_USER" >/dev/null || die "user does not exist: $SATURN_USER"
[[ -f "$REPO_ROOT/sw_projects/P2_app/Makefile" ]] || die "invalid Saturn checkout: $REPO_ROOT"
SATURN_HOME="$(getent passwd "$SATURN_USER" | cut -d: -f6)"

install_packages(){
  local krel meta
  krel="$(uname -r)"
  meta="linux-headers-${krel#*+rpt-}"
  run apt-get update
  run env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates curl git rsync sudo nginx apache2-utils \
    build-essential dkms pkg-config libgpiod-dev libi2c-dev \
    libgtk-3-dev libayatana-appindicator3-dev libasound2-dev \
    libfftw3-dev libcurl4-openssl-dev python3
  if (( DRY_RUN )); then
    run apt-get install -y "linux-headers-$krel"
  elif apt-cache show "linux-headers-$krel" >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends "linux-headers-$krel"
  elif apt-cache show "$meta" >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends "$meta"
  elif apt-cache show raspberrypi-kernel-headers >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends raspberrypi-kernel-headers
  else
    die "no matching kernel-header package is available for $krel"
  fi
}

run_as_saturn_user(){
  if (( DRY_RUN )); then
    run runuser -u "$SATURN_USER" -- env HOME="$SATURN_HOME" "$@"
  else
    runuser -u "$SATURN_USER" -- env HOME="$SATURN_HOME" "$@"
  fi
}

install_driver(){
  run env SATURN_REPO_DIR="$REPO_ROOT" bash "$REPO_ROOT/scripts/install-xdma-dkms.sh"
  run install -d -m 0755 /etc/modules-load.d
  if (( DRY_RUN )); then
    info "[dry-run] write xdma to /etc/modules-load.d/xdma.conf"
  else
    printf 'xdma\n' >/etc/modules-load.d/xdma.conf
  fi
  run modprobe xdma
  run udevadm control --reload-rules
  run udevadm trigger --subsystem-match=xdma
}

install_p2(){
  run_as_saturn_user make -C "$REPO_ROOT/sw_projects/P2_app" -j"$(nproc)"
  run env HOME="$SATURN_HOME" SUDO_USER="$SATURN_USER" SATURN_USER="$SATURN_USER" \
    bash "$REPO_ROOT/sw_tools/p2app-control/install.sh"
}

install_saturn_go(){
  run env HOME="$SATURN_HOME" SUDO_USER="$SATURN_USER" SATURN_SERVICE_USER="$SATURN_USER" \
    SATURN_BRIDGE_WDSP_FLAVOR=wdsp2 SATURN_INSTALL_BRIDGE=1 SATURN_REQUIRE_BRIDGE=1 \
    bash "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
}

verify_install(){
  if (( INSTALL_DRIVER )); then
    [[ -c /dev/xdma0_user || -c /dev/xdma/card0/user ]] || die "XDMA user device is missing"
  fi
  if (( INSTALL_P2 )); then
    [[ "$(stat -c '%U:%G' /opt/saturn-radio/bin/p2app)" == "root:root" ]] || \
      die "p2app runtime is not root-owned"
    [[ "$(systemctl show -p User --value p2app.service)" == "saturn-radio" ]] || \
      die "p2app.service is not using the dedicated account"
    systemctl is-active --quiet p2app.service || die "p2app.service is not active"
  fi
  if (( INSTALL_GO )); then
    systemctl is-active --quiet saturn-bridge.service || die "saturn-bridge.service is not active"
    systemctl is-active --quiet saturn-go.service || die "saturn-go.service is not active"
    curl -fsS --max-time 3 http://127.0.0.1:8080/healthz >/dev/null || \
      die "Saturn Go health check failed"
  fi
  info "verification passed"
}

info "repository: $REPO_ROOT"
info "runtime user: $SATURN_USER"
(( INSTALL_PACKAGES )) && install_packages
run bash "$REPO_ROOT/rules/install-rules.sh"
(( INSTALL_DRIVER )) && install_driver
(( INSTALL_P2 )) && install_p2
(( INSTALL_GO )) && install_saturn_go
if (( VERIFY && ! DRY_RUN )); then
  verify_install
elif (( VERIFY )); then
  info "[dry-run] verify XDMA device, ownership, service users, services, and health endpoint"
fi
info "installation complete"
