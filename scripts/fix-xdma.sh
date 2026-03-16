#!/usr/bin/env bash
# fix-xdma.sh
# Version: 2.2
# Rebuild & (re)install XDMA kernel module for the running kernel and, when
# present, pre-stage it for the newest installed kernel. Stop/start
# p2app.service and verify it's running.
# Usage: sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh
# Author: Jerry DeLong, KD4YAL

set -euo pipefail

SERVICE_NAME="p2app.service"

RED=$'\033[0;31m'; GRN=$'\033[0;32m'; YLW=$'\033[0;33m'; CYA=$'\033[0;36m'; NC=$'\033[0m'
info(){ printf "${CYA}[INFO]${NC} %s\n" "$*"; }
ok()  { printf "${GRN}[ OK ]${NC} %s\n" "$*"; }
warn(){ printf "${YLW}[WARN]${NC} %s\n" "$*"; }
die(){ printf "${RED}[ERR ] %s${NC}\n" "$*" >&2; exit 1; }
have(){ command -v "$1" >/dev/null 2>&1; }

need_root(){ [[ $(id -u) -eq 0 ]] || die "Please run as root (sudo)."; }

resolve_user_home(){
  if [[ -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]]; then
    getent passwd "${SUDO_USER}" | cut -d: -f6
  else
    getent passwd "pi" | cut -d: -f6 || echo "${HOME}"
  fi
}

resolve_driver_dir(){
  local uh d
  if [[ -n "${SATURN_DRIVER_DIR:-}" ]]; then
    d="${SATURN_DRIVER_DIR}"
  elif [[ -n "${SATURN_REPO_DIR:-}" ]]; then
    d="${SATURN_REPO_DIR}/linuxdriver/xdma"
  else
    uh="$(resolve_user_home)"
    d="${uh}/github/Saturn/linuxdriver/xdma"
  fi
  [[ -d "$d" ]] || die "Driver directory not found: $d"
  printf "%s" "$d"
}

kernel_flavor(){
  local krel="$1"
  printf "%s" "${krel#*+rpt-}"
}

latest_installed_kernel(){
  local flavor="${1:-}"
  find /lib/modules -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | \
    { [[ -n "$flavor" ]] && grep -F "+rpt-${flavor}$" || cat; } | \
    sort -V | tail -n1
}

build_dir_for_kernel(){
  local krel="$1"
  printf "/lib/modules/%s/build" "$krel"
}

ensure_headers(){
  local krel="$1" kbuild meta_pkg
  kbuild="$(build_dir_for_kernel "$krel")"
  meta_pkg="linux-headers-$(kernel_flavor "$krel")"
  if [[ ! -d "$kbuild" ]]; then
    warn "Kernel headers for ${krel} not found. Installing…"
    apt-get update -y || die "apt update failed"
    if apt-cache show "linux-headers-${krel}" >/dev/null 2>&1; then
      apt-get install -y "linux-headers-${krel}" || die "linux-headers-${krel} install failed"
    elif apt-cache show "${meta_pkg}" >/dev/null 2>&1; then
      apt-get install -y "${meta_pkg}" || die "${meta_pkg} install failed"
    elif apt-cache show raspberrypi-kernel-headers >/dev/null 2>&1; then
      apt-get install -y raspberrypi-kernel-headers || die "raspberrypi-kernel-headers install failed"
    else
      die "No suitable kernel header package found for ${krel}"
    fi
    [[ -d "$kbuild" ]] || die "Headers still missing after install."
  fi
  ok "Kernel headers present for ${krel}."
}

warn_if_newer_kernel_installed(){
  local running_krel latest_krel running_flavor
  running_krel="$(uname -r)"
  running_flavor="$(kernel_flavor "$running_krel")"
  latest_krel="$(latest_installed_kernel "$running_flavor")"
  [[ -n "$latest_krel" ]] || return 0
  [[ "$latest_krel" == "$running_krel" ]] && return 0

  warn "A newer kernel is installed (${latest_krel}) than the one currently running (${running_krel})."
  warn "XDMA will be built for the running kernel now and pre-staged for ${latest_krel} so it is ready after reboot."
}

target_kernels(){
  local running_krel latest_krel running_flavor
  running_krel="$(uname -r)"
  running_flavor="$(kernel_flavor "$running_krel")"
  latest_krel="$(latest_installed_kernel "$running_flavor")"

  printf "%s\n" "$running_krel"
  if [[ -n "$latest_krel" && "$latest_krel" != "$running_krel" ]]; then
    printf "%s\n" "$latest_krel"
  fi
}

build_and_install_for_kernel(){
  local krel="$1" driver_dir="$2" xdma_inc="$3" kbuild
  kbuild="$(build_dir_for_kernel "$krel")"

  info "Cleaning previous build for ${krel}…"
  run_make -C "${kbuild}" M="${driver_dir}" clean || warn "make clean reported issues for ${krel}; continuing"

  info "Building xdma.ko for ${krel}…"
  run_make -C "${kbuild}" M="${driver_dir}" \
    EXTRA_CFLAGS="-I${xdma_inc} -Wno-empty-body -Wno-missing-prototypes -Wno-missing-declarations" \
    KBUILD_VERBOSE=0 \
    modules

  info "Installing module for ${krel}…"
  run_make -C "${kbuild}" M="${driver_dir}" DEPMOD=/bin/true modules_install

  info "Running depmod for ${krel}…"
  depmod "${krel}"
}

# Emits logs to STDERR, returns include dir on STDOUT
ensure_xdma_header(){
  local driver_dir="$1"
  local sys_inc_dir="/usr/local/include/xdma"
  local sys_hdr="${sys_inc_dir}/libxdma_api.h"
  mkdir -p "$sys_inc_dir"

  if [[ -f "$sys_hdr" ]]; then
    ok "Found header: ${sys_hdr}" >&2; printf "%s" "$sys_inc_dir"; return 0
  fi

  # Search repo
  local root found=""; root="$(dirname "$driver_dir")"
  while IFS= read -r p; do found="$p"; break; done < <(find "$root" -maxdepth 5 -type f -name libxdma_api.h 2>/dev/null | head -n1)
  if [[ -n "$found" ]]; then
    info "Staging libxdma_api.h from ${found}" >&2
    cp -f "$found" "$sys_hdr"; ok "Header staged at ${sys_hdr}" >&2
    printf "%s" "$sys_inc_dir"; return 0
  fi

  # Fetch
  info "libxdma_api.h not found locally; attempting download…" >&2
  local url1="https://raw.githubusercontent.com/Xilinx/dma_ip_drivers/master/XDMA/linux-kernel/include/libxdma_api.h"
  local url2="https://gitlab.esss.lu.se/icshwi/dma_ip_drivers/-/raw/master/XDMA/linux-kernel/include/libxdma_api.h?inline=false"
  if have curl; then
    curl -fsSL "$url1" -o "$sys_hdr" || curl -fsSL "$url2" -o "$sys_hdr" || die "Failed to download libxdma_api.h"
  elif have wget; then
    wget -q -O "$sys_hdr" "$url1" || wget -q -O "$sys_hdr" "$url2" || die "Failed to download libxdma_api.h"
  else
    apt-get update -y && apt-get install -y curl || die "Could not install curl"
    curl -fsSL "$url1" -o "$sys_hdr" || curl -fsSL "$url2" -o "$sys_hdr" || die "Failed to download libxdma_api.h"
  fi
  ok "Header downloaded to ${sys_hdr}" >&2
  printf "%s" "$sys_inc_dir"
}

run_make(){
  # Quiet the XVC_FLAGS and System.map chatter; set VERBOSE=1 to see everything.
  if [[ "${VERBOSE:-0}" == "1" ]]; then
    make "$@"
  else
    env MAKEFLAGS="${MAKEFLAGS:-} -s" make "$@" 2>&1 | \
      awk '!/Makefile:[0-9]+: XVC_FLAGS: \./ && !/Warning: modules_install: missing '\''System.map'\'' file\. Skipping depmod\./ { print }'
  fi
}

service_exists(){ systemctl list-unit-files --type=service | awk '{print $1}' | grep -qx "$SERVICE_NAME"; }
service_active(){ systemctl is-active --quiet "$SERVICE_NAME"; }

stop_service_if_running(){
  if service_exists; then
    if service_active; then
      info "Stopping ${SERVICE_NAME}…"
      systemctl stop "$SERVICE_NAME" || die "Failed to stop ${SERVICE_NAME}"
      WAS_ACTIVE=1
    else
      WAS_ACTIVE=0
      warn "${SERVICE_NAME} is not active."
    fi
  else
    WAS_ACTIVE=0
    warn "${SERVICE_NAME} not found. Skipping stop/start."
  fi
}

start_service_and_verify(){
  if [[ "${WAS_ACTIVE}" -eq 1 ]]; then
    info "Starting ${SERVICE_NAME}…"
    systemctl start "$SERVICE_NAME" || die "Failed to start ${SERVICE_NAME}"
  else
    # If it wasn't active before but the unit exists, still start it (you asked to restart)
    if service_exists; then
      info "Starting ${SERVICE_NAME} (was not active before)…"
      systemctl start "$SERVICE_NAME" || die "Failed to start ${SERVICE_NAME}"
    fi
  fi

  if service_exists; then
    if service_active; then
      ok "${SERVICE_NAME} is active."
    else
      die "${SERVICE_NAME} is not active after start. Check: journalctl -u ${SERVICE_NAME} -n 50 --no-pager"
    fi
  fi
}

unload_xdma_if_loaded(){
  if lsmod | awk '{print $1}' | grep -qx xdma; then
    info "Unloading xdma…"
    if ! modprobe -r xdma; then
      warn "xdma is still in use by some process. Will continue build; reload attempt may fail."
    fi
  fi
}

reload_xdma_module(){
  # Always try a fresh reload after install
  if lsmod | awk '{print $1}' | grep -qx xdma; then
    info "Unloading xdma for fresh reload…"
    modprobe -r xdma || warn "Could not unload xdma; another process may be holding /dev/xdma*"
  fi
  info "Loading xdma…"
  modprobe xdma || die "xdma failed to load; check dmesg"
  if lsmod | awk '{print $1}' | grep -qx xdma; then
    ok "xdma loaded."
  else
    die "xdma not present after modprobe."
  fi
}

main(){
  need_root
  warn_if_newer_kernel_installed

  local driver_dir running_krel
  driver_dir="$(resolve_driver_dir)"
  running_krel="$(uname -r)"

  info "Running kernel: ${running_krel}"
  info "Driver: ${driver_dir}"

  # Ensure API header
  local xdma_inc; xdma_inc="$(ensure_xdma_header "${driver_dir}")"
  info "Using include dir: ${xdma_inc}"

  local krel
  while IFS= read -r krel; do
    [[ -n "$krel" ]] || continue
    ensure_headers "$krel"
  done < <(target_kernels)

  # 1) Stop service (so it releases /dev/xdma*)
  WAS_ACTIVE=0
  stop_service_if_running

  # 2) Try to unload the module now that p2app is stopped
  unload_xdma_if_loaded

  # 3) Build & install
  cd "${driver_dir}"
  while IFS= read -r krel; do
    [[ -n "$krel" ]] || continue
    build_and_install_for_kernel "$krel" "${driver_dir}" "${xdma_inc}"
  done < <(target_kernels)

  # 4) Reload module
  reload_xdma_module

  # 5) Start service and verify
  start_service_and_verify

  # Friendly summary
  modinfo -n xdma 2>/dev/null | xargs -I{} printf "%s%s%s\n" "${CYA}" "Module file: {}" "${NC}" || true
  local latest_krel
  latest_krel="$(latest_installed_kernel "$(kernel_flavor "$running_krel")")"
  if [[ -n "$latest_krel" && "$latest_krel" != "$running_krel" ]]; then
    ok "XDMA also staged for ${latest_krel}. Reboot to start using that kernel."
  fi
  ok "Done: XDMA updated, ${SERVICE_NAME} running."
}

main "$@"
