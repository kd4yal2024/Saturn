#!/usr/bin/env bash
set -euo pipefail

# Prepare the low-memory Pi build environment for Saturn Go release builds.
# The command is intentionally narrow so it can be installed as a privileged
# helper and allowed through sudoers for web-driven self-updates.

default_swap_file(){
  local build_user build_home
  build_user="${SATURN_SATURNGO_BUILD_USER:-${SUDO_USER:-$(id -un)}}"
  build_home="$(getent passwd "$build_user" 2>/dev/null | cut -d: -f6 || true)"
  [[ -n "$build_home" && -d "$build_home" ]] || build_home=/var/tmp
  printf '%s/saturn-build.swap\n' "$build_home"
}

SWAP_FILE="${SATURN_SATURNGO_BUILD_SWAP_FILE:-$(default_swap_file)}"
SWAP_MIB="${SATURN_SATURNGO_BUILD_SWAP_MIB:-2048}"
RESERVE_MIB="${SATURN_SATURNGO_BUILD_RESERVE_MIB:-512}"

log(){ printf '[saturn-go-build-preflight] %s\n' "$*"; }
die(){ printf '[saturn-go-build-preflight] ERR: %s\n' "$*" >&2; exit 1; }

require_positive_integer(){
  local name="$1" value="$2"
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || die "$name must be a positive integer, got: $value"
}

require_nonnegative_integer(){
  local name="$1" value="$2"
  [[ "$value" =~ ^[0-9]+$ ]] || die "$name must be a non-negative integer, got: $value"
}

need_cmd(){
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

requested_swap_bytes(){
  printf '%s\n' $(( SWAP_MIB * 1024 * 1024 ))
}

min_active_swap_bytes(){
  local requested
  requested="$(requested_swap_bytes)"
  printf '%s\n' $(( requested > 1048576 ? requested - 1048576 : requested ))
}

active_swap_bytes(){
  local path="$1"
  swapon --show=NAME,SIZE --bytes --noheadings 2>/dev/null \
    | awk -v target="$path" '$1 == target { print $2; found=1; exit } END { if (!found) print 0 }'
}

meminfo_mib(){
  local key="$1"
  awk -v key="$key" '$1 == key ":" { print int($2 / 1024); found=1; exit } END { if (!found) print 0 }' /proc/meminfo
}

capacity_report(){
  log "Capacity: RAM $(meminfo_mib MemTotal) MiB total, $(meminfo_mib MemAvailable) MiB available; swap $(meminfo_mib SwapTotal) MiB total, $(meminfo_mib SwapFree) MiB free"
}

available_bytes_at(){
  df -PB1 "$1" 2>/dev/null | awk 'NR == 2 { print $4; found=1; exit } END { if (!found) print 0 }'
}

verify_creation_space(){
  local parent="$1"
  local reclaimed_bytes="${2:-0}"
  local required_bytes available_bytes
  required_bytes=$(( (SWAP_MIB + RESERVE_MIB) * 1024 * 1024 ))
  available_bytes="$(available_bytes_at "$parent")"
  if (( available_bytes + reclaimed_bytes < required_bytes )); then
    die "Not enough free space to create ${SWAP_MIB} MiB build swap while retaining the ${RESERVE_MIB} MiB safety reserve at $parent (available: $(( available_bytes / 1024 / 1024 )) MiB)."
  fi
}

ensure_swap(){
  require_positive_integer SATURN_SATURNGO_BUILD_SWAP_MIB "$SWAP_MIB"
  require_nonnegative_integer SATURN_SATURNGO_BUILD_RESERVE_MIB "$RESERVE_MIB"
  need_cmd awk
  need_cmd chmod
  need_cmd df
  need_cmd mkswap
  need_cmd mkdir
  need_cmd swapon

  local min_bytes active_bytes parent
  capacity_report
  min_bytes="$(min_active_swap_bytes)"
  active_bytes="$(active_swap_bytes "$SWAP_FILE")"

  if (( active_bytes >= min_bytes )); then
    log "Build swap active: $SWAP_FILE ($(( active_bytes / 1024 / 1024 )) MiB)"
    return 0
  fi

  if (( EUID != 0 )); then
    die "Build swap is not active or is smaller than ${SWAP_MIB} MiB. Re-run through sudo or install/use the privileged helper."
  fi

  if (( active_bytes > 0 && active_bytes < min_bytes )); then
    die "Active swap at $SWAP_FILE is only $(( active_bytes / 1024 / 1024 )) MiB; refusing to replace an active swapfile."
  fi

  parent="$(dirname "$SWAP_FILE")"
  mkdir -p "$parent"

  if [[ -f "$SWAP_FILE" ]]; then
    local current_bytes
    current_bytes="$(stat -c '%s' "$SWAP_FILE" 2>/dev/null || printf '0')"
    if (( current_bytes < $(requested_swap_bytes) )); then
      verify_creation_space "$parent" "$current_bytes"
      log "Replacing undersized inactive build swap: $SWAP_FILE ($(( current_bytes / 1024 / 1024 )) MiB)"
      rm -f "$SWAP_FILE"
    fi
  fi

  if [[ ! -f "$SWAP_FILE" ]]; then
    verify_creation_space "$parent"
    log "Creating build swap: $SWAP_FILE (${SWAP_MIB} MiB)"
    umask 077
    if command -v fallocate >/dev/null 2>&1; then
      fallocate -l "${SWAP_MIB}M" "$SWAP_FILE" || dd if=/dev/zero of="$SWAP_FILE" bs=1M count="$SWAP_MIB" status=progress
    else
      dd if=/dev/zero of="$SWAP_FILE" bs=1M count="$SWAP_MIB" status=progress
    fi
  fi

  chmod 600 "$SWAP_FILE"
  mkswap -f "$SWAP_FILE" >/dev/null
  swapon "$SWAP_FILE"

  active_bytes="$(active_swap_bytes "$SWAP_FILE")"
  if (( active_bytes < min_bytes )); then
    die "Build swap did not activate at the expected size: $SWAP_FILE"
  fi
  log "Build swap ready: $SWAP_FILE ($(( active_bytes / 1024 / 1024 )) MiB)"
  capacity_report
}

status(){
  require_positive_integer SATURN_SATURNGO_BUILD_SWAP_MIB "$SWAP_MIB"
  require_nonnegative_integer SATURN_SATURNGO_BUILD_RESERVE_MIB "$RESERVE_MIB"
  need_cmd awk
  need_cmd swapon
  local active_bytes
  capacity_report
  active_bytes="$(active_swap_bytes "$SWAP_FILE")"
  if (( active_bytes > 0 )); then
    log "Build swap active: $SWAP_FILE ($(( active_bytes / 1024 / 1024 )) MiB)"
  else
    log "Build swap inactive: $SWAP_FILE"
  fi
}

usage(){
  cat <<EOF
Usage: $(basename "$0") <command>

Commands:
  ensure-swap   Create/activate the Saturn Go build swapfile if needed
  status        Report whether the build swapfile is active

Environment:
  SATURN_SATURNGO_BUILD_SWAP_FILE  default: <build-user-home>/saturn-build.swap
  SATURN_SATURNGO_BUILD_SWAP_MIB   default: 2048
  SATURN_SATURNGO_BUILD_RESERVE_MIB default: 512
EOF
}

case "${1:-}" in
  ensure-swap) ensure_swap ;;
  status) status ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 2 ;;
esac
