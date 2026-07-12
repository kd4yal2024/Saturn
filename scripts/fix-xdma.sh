#!/usr/bin/env bash
# fix-xdma.sh
# Version: 2.5
# Rebuild & (re)install XDMA kernel module for the running kernel and, when
# present, pre-stage it for the newest installed kernel. Stop/start
# p2app.service, verify it's running, and emit a structured XDMA diagnosis.
# Supports a stage-only kernel mode for post-install hooks that must not touch
# the live module or service.
# Usage:
#   sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh
#   sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh --stage-kernel <kernel-release>
# Author: Jerry DeLong, KD4YAL

set -euo pipefail

SERVICE_NAME="p2app.service"
STAGE_ONLY=0
TARGET_KERNELS=()
LOCK_FD=""
ROLLBACK_DIR=""
TRANSACTION_ACTIVE=0
WAS_ACTIVE=0
XDMA_WAS_LOADED=0

RED=$'\033[0;31m'; GRN=$'\033[0;32m'; YLW=$'\033[0;33m'; CYA=$'\033[0;36m'; NC=$'\033[0m'
info(){ printf "${CYA}[INFO]${NC} %s\n" "$*"; }
ok()  { printf "${GRN}[ OK ]${NC} %s\n" "$*"; }
warn(){ printf "${YLW}[WARN]${NC} %s\n" "$*"; }
die(){
  local rc=1
  printf "${RED}[ERR ] %s${NC}\n" "$*" >&2
  if (( TRANSACTION_ACTIVE )); then
    trap - ERR
    restore_live_module "$(uname -r)"
    [[ -n "$ROLLBACK_DIR" ]] && rm -rf "$ROLLBACK_DIR"
    TRANSACTION_ACTIVE=0
  fi
  exit "$rc"
}
have(){ command -v "$1" >/dev/null 2>&1; }

usage(){
  cat <<'EOF'
Usage:
  sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh
  sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh --stage-kernel <kernel-release>

Options:
  --stage-kernel <kernel-release>  Build/install XDMA only for the named kernel
                                   without stopping p2app, unloading xdma, or
                                   touching the live running system.
  -h, --help                       Show this help text.
EOF
}

parse_args(){
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --stage-kernel)
        [[ $# -ge 2 ]] || die "--stage-kernel requires a kernel release argument"
        [[ -n "${2:-}" ]] || die "--stage-kernel requires a non-empty kernel release argument"
        [[ "${2}" =~ ^[0-9][A-Za-z0-9._+-]*$ ]] || die "--stage-kernel: invalid release format: ${2}"
        STAGE_ONLY=1
        TARGET_KERNELS+=("$2")
        shift 2
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
}

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

resolve_build_user(){
  local u
  if [[ -n "${SATURN_BUILD_USER:-}" ]]; then
    u="${SATURN_BUILD_USER}"
  elif [[ -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]]; then
    u="${SUDO_USER}"
  elif [[ -n "${SATURN_USER:-}" ]]; then
    u="${SATURN_USER}"
  else
    u="pi"
  fi

  getent passwd "${u}" >/dev/null 2>&1 || die "Build user not found: ${u}"
  printf "%s" "${u}"
}

resolve_build_home(){
  local user="$1"
  getent passwd "${user}" | cut -d: -f6
}

resolve_doctor_script(){
  local uh d
  if [[ -n "${SATURN_XDMA_DOCTOR_SCRIPT:-}" ]]; then
    d="${SATURN_XDMA_DOCTOR_SCRIPT}"
  elif [[ -n "${SATURN_REPO_DIR:-}" ]]; then
    d="${SATURN_REPO_DIR}/scripts/saturn-xdma-doctor.sh"
  else
    uh="$(resolve_user_home)"
    d="${uh}/github/Saturn/scripts/saturn-xdma-doctor.sh"
  fi
  printf "%s" "$d"
}

kernel_flavor(){
  local krel="$1"
  printf "%s" "${krel#*+rpt-}"
}

latest_installed_kernel(){
  local flavor="${1:-}"
  find /lib/modules -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | \
    { if [[ -n "$flavor" ]]; then grep -F "+rpt-${flavor}" || true; else cat; fi; } | \
    sort -V | tail -n1
}

build_dir_for_kernel(){
  local krel="$1"
  local modules_build="/lib/modules/${krel}/build"
  local headers_dir="/usr/src/linux-headers-${krel}"
  if [[ -d "$modules_build" ]]; then
    printf "%s" "$modules_build"
  elif [[ -d "$headers_dir" ]]; then
    printf "%s" "$headers_dir"
  else
    printf "%s" "$modules_build"
  fi
}

module_updates_dir_for_kernel(){
  local krel="$1"
  printf "/lib/modules/%s/updates" "$krel"
}

repair_kernel_header_links(){
  local krel="$1"
  local modules_dir="/lib/modules/${krel}"
  local build_link="${modules_dir}/build"
  local source_link="${modules_dir}/source"
  local headers_dir="/usr/src/linux-headers-${krel}"
  local common_dir="/usr/src/linux-headers-${krel%-rpi*}-common-rpi"

  [[ -d "$modules_dir" ]] || return 0

  if [[ -d "$headers_dir" && ! -d "$build_link" ]]; then
    ln -sfn "../../../src/linux-headers-${krel}" "$build_link"
    ok "Repaired kernel build link for ${krel}."
  fi

  if [[ -d "$common_dir" && ! -d "$source_link" ]]; then
    ln -sfn "../../../src/$(basename "$common_dir")" "$source_link"
    ok "Repaired kernel source link for ${krel}."
  fi
}

ensure_headers(){
  local krel="$1" kbuild meta_pkg headers_dir
  headers_dir="/usr/src/linux-headers-${krel}"
  repair_kernel_header_links "$krel"
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
    repair_kernel_header_links "$krel"
    kbuild="$(build_dir_for_kernel "$krel")"
    if [[ ! -d "$kbuild" && -d "$headers_dir" ]]; then
      kbuild="$headers_dir"
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
  if [[ ${#TARGET_KERNELS[@]} -gt 0 ]]; then
    printf "%s\n" "${TARGET_KERNELS[@]}"
    return 0
  fi

  local running_krel latest_krel running_flavor
  running_krel="$(uname -r)"
  running_flavor="$(kernel_flavor "$running_krel")"
  latest_krel="$(latest_installed_kernel "$running_flavor")"

  printf "%s\n" "$running_krel"
  if [[ -n "$latest_krel" && "$latest_krel" != "$running_krel" ]]; then
    printf "%s\n" "$latest_krel"
  fi
}

build_for_kernel(){
  local krel="$1" driver_dir="$2" xdma_inc="$3" kbuild
  kbuild="$(build_dir_for_kernel "$krel")"

  info "Cleaning previous build for ${krel}…"
  run_make_as_build_user -C "${kbuild}" M="${driver_dir}" clean || warn "make clean reported issues for ${krel}; continuing"

  info "Building xdma.ko for ${krel}…"
  run_make_as_build_user -C "${kbuild}" M="${driver_dir}" \
    KCFLAGS="-I${xdma_inc} -Wno-empty-body -Wno-missing-prototypes -Wno-missing-declarations" \
    KBUILD_VERBOSE=0 \
    modules

}

install_built_for_kernel(){
  local krel="$1" driver_dir="$2" kbuild
  kbuild="$(build_dir_for_kernel "$krel")"

  info "Installing module for ${krel}…"
  run_make -C "${kbuild}" M="${driver_dir}" DEPMOD=/bin/true modules_install
  normalize_installed_module_owner "${krel}"

  info "Running depmod for ${krel}…"
  depmod "${krel}"
}

build_and_install_for_kernel(){
  local krel="$1" driver_dir="$2" xdma_inc="$3"
  build_for_kernel "$krel" "$driver_dir" "$xdma_inc"
  install_built_for_kernel "$krel" "$driver_dir"
}

# Emits logs to STDERR, returns include dir on STDOUT
ensure_xdma_header(){
  local driver_dir="$1"
  local sys_inc_dir="/usr/local/include/xdma"
  local sys_hdr="${sys_inc_dir}/libxdma_api.h"
  local repo_hdr
  repo_hdr="$(dirname "$driver_dir")/include/libxdma_api.h"
  [[ -f "$repo_hdr" ]] || die "Pinned repository header is missing: $repo_hdr"
  mkdir -p "$sys_inc_dir"
  install -m 0644 -o root -g root "$repo_hdr" "$sys_hdr"
  ok "Staged repository header: ${repo_hdr}" >&2
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

run_make_as_build_user(){
  local user="${BUILD_USER:-root}" home="${BUILD_HOME:-/root}"

  if [[ "$user" == "root" ]]; then
    run_make "$@"
    return
  fi

  if [[ "${VERBOSE:-0}" == "1" ]]; then
    runuser -u "$user" -- \
      env HOME="$home" USER="$user" LOGNAME="$user" PATH="$PATH" \
      make "$@"
  else
    runuser -u "$user" -- \
      env HOME="$home" USER="$user" LOGNAME="$user" PATH="$PATH" MAKEFLAGS="${MAKEFLAGS:-} -s" \
      make "$@" 2>&1 | \
      awk '!/Makefile:[0-9]+: XVC_FLAGS: \./ && !/Warning: modules_install: missing '\''System.map'\'' file\. Skipping depmod\./ { print }'
  fi
}

acquire_build_lock(){
  local lock_file="${SATURN_XDMA_BUILD_LOCK:-/var/lock/saturn-xdma-build.lock}"

  if ! have flock; then
    warn "flock not found; proceeding without an XDMA build lock."
    return 0
  fi

  install -d -m 0755 "$(dirname "$lock_file")" 2>/dev/null || true
  exec {LOCK_FD}> "$lock_file"
  flock -x "$LOCK_FD"
  info "Acquired XDMA build lock: ${lock_file}"
}

run_xdma_doctor(){
  local doctor_script="${DOCTOR_SCRIPT:-}"
  if [[ -z "$doctor_script" ]]; then
    doctor_script="$(resolve_doctor_script)"
  fi

  if [[ ! -x "$doctor_script" ]]; then
    warn "XDMA doctor script not found/executable: ${doctor_script}"
    return 0
  fi

  info "Running Saturn XDMA doctor…"
  "$doctor_script" "$@" || true
}

normalize_installed_module_owner(){
  local krel="$1" moddir
  moddir="$(module_updates_dir_for_kernel "$krel")"

  [[ -d "$moddir" ]] || return 0

  if [[ -f "${moddir}/xdma.ko" ]]; then
    chown root:root "${moddir}/xdma.ko"
  fi

  if [[ -f "${moddir}/xdma.ko.xz" ]]; then
    chown root:root "${moddir}/xdma.ko.xz"
  fi
}

snapshot_live_module(){
  local krel="$1" module
  ROLLBACK_DIR="$(mktemp -d /var/tmp/saturn-xdma-rollback.XXXXXX)"
  chmod 0700 "$ROLLBACK_DIR"
  while IFS= read -r -d '' module; do
    cp -a --parents "$module" "$ROLLBACK_DIR"
  done < <(find "/lib/modules/$krel" -xdev -type f \
    \( -name 'xdma.ko' -o -name 'xdma.ko.xz' -o -name 'xdma.ko.zst' -o -name 'xdma.ko.gz' \) -print0)
  if lsmod | awk '{print $1}' | grep -qx xdma; then
    XDMA_WAS_LOADED=1
  else
    XDMA_WAS_LOADED=0
  fi
  TRANSACTION_ACTIVE=1
  info "Saved live XDMA state in ${ROLLBACK_DIR}."
}

restore_live_module(){
  local krel="$1"
  set +e
  systemctl stop "$SERVICE_NAME" >/dev/null 2>&1 || true
  modprobe -r xdma >/dev/null 2>&1 || true
  find "/lib/modules/$krel" -xdev -type f \
    \( -name 'xdma.ko' -o -name 'xdma.ko.xz' -o -name 'xdma.ko.zst' -o -name 'xdma.ko.gz' \) -delete
  if [[ -d "$ROLLBACK_DIR/lib" ]]; then
    cp -a "$ROLLBACK_DIR/lib/." /lib/
  fi
  depmod "$krel" >/dev/null 2>&1 || true
  if (( XDMA_WAS_LOADED )); then
    modprobe xdma >/dev/null 2>&1 || true
  fi
  if (( WAS_ACTIVE )); then
    systemctl start "$SERVICE_NAME" >/dev/null 2>&1 || true
  fi
}

rollback_on_error(){
  local rc="$?" krel
  trap - ERR
  krel="$(uname -r)"
  if (( TRANSACTION_ACTIVE )); then
    warn "XDMA update failed; restoring the previous module and service state."
    restore_live_module "$krel"
  fi
  [[ -n "$ROLLBACK_DIR" ]] && rm -rf "$ROLLBACK_DIR"
  exit "$rc"
}

commit_live_module(){
  TRANSACTION_ACTIVE=0
  trap - ERR
  [[ -n "$ROLLBACK_DIR" ]] && rm -rf "$ROLLBACK_DIR"
  ROLLBACK_DIR=""
}

service_exists(){ systemctl list-unit-files --type=service | awk '{print $1}' | grep -qx "$SERVICE_NAME"; }
service_active(){ systemctl is-active --quiet "$SERVICE_NAME"; }

stop_service_if_running(){
  if service_exists; then
    if service_active; then
      info "Stopping ${SERVICE_NAME}…"
      WAS_ACTIVE=1
      systemctl stop "$SERVICE_NAME" || die "Failed to stop ${SERVICE_NAME}"
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
    info "Leaving ${SERVICE_NAME} inactive because it was inactive before repair."
    return 0
  fi

  if service_exists; then
    if service_active; then
      ok "${SERVICE_NAME} is active."
    else
      run_xdma_doctor
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
  if ! modprobe xdma; then
    run_xdma_doctor --mark-modprobe-failed
    die "xdma failed to load; see Saturn XDMA doctor output above"
  fi
  if lsmod | awk '{print $1}' | grep -qx xdma; then
    ok "xdma loaded."
  else
    die "xdma not present after modprobe."
  fi
}

main(){
  parse_args "$@"
  need_root
  acquire_build_lock
  if (( ! STAGE_ONLY )); then
    warn_if_newer_kernel_installed
  fi

  local driver_dir running_krel
  local -a kernels=()
  driver_dir="$(resolve_driver_dir)"
  running_krel="$(uname -r)"
  BUILD_USER="$(resolve_build_user)"
  BUILD_HOME="$(resolve_build_home "${BUILD_USER}")"
  DOCTOR_SCRIPT="$(resolve_doctor_script)"

  info "Running kernel: ${running_krel}"
  info "Driver: ${driver_dir}"
  info "Building module as: ${BUILD_USER}"

  # Ensure API header
  local xdma_inc; xdma_inc="$(ensure_xdma_header "${driver_dir}")"
  info "Using include dir: ${xdma_inc}"

  local krel
  mapfile -t kernels < <(target_kernels)
  for krel in "${kernels[@]}"; do
    [[ -n "$krel" ]] || continue
    ensure_headers "$krel"
  done

  if (( STAGE_ONLY )); then
    info "Stage-only mode: building/staging XDMA without touching the live module or service."
    cd "${driver_dir}"
    for krel in "${kernels[@]}"; do
      [[ -n "$krel" ]] || continue
      build_and_install_for_kernel "$krel" "${driver_dir}" "${xdma_inc}"
    done
    ok "XDMA staged for kernel(s): ${TARGET_KERNELS[*]}"
    return 0
  fi

  # Pre-stage non-running kernels without interrupting the active radio. Build
  # the running-kernel artifact last so it remains ready for the short swap.
  cd "${driver_dir}"
  for krel in "${kernels[@]}"; do
    [[ -n "$krel" && "$krel" != "$running_krel" ]] || continue
    build_and_install_for_kernel "$krel" "${driver_dir}" "${xdma_inc}"
  done
  build_for_kernel "$running_krel" "${driver_dir}" "${xdma_inc}"

  # Snapshot before touching the active module. The ERR trap restores module
  # files, loaded state, and service state if any later operation fails.
  snapshot_live_module "$running_krel"
  trap rollback_on_error ERR

  # 1) Stop service (so it releases /dev/xdma*)
  WAS_ACTIVE=0
  stop_service_if_running

  # 2) Try to unload the module now that p2app is stopped
  unload_xdma_if_loaded

  # 3) Install the already-built running-kernel artifact.
  install_built_for_kernel "$running_krel" "${driver_dir}"

  # 4) Reload module
  reload_xdma_module

  # 5) Start service and verify
  start_service_and_verify
  commit_live_module

  # 6) Emit structured diagnosis so repair ends with a classified state.
  run_xdma_doctor

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
