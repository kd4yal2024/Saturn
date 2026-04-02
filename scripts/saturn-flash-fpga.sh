#!/usr/bin/env bash
set -euo pipefail

IMAGE=""
USE_FALLBACK=false
USE_PRIMARY=false
VERIFY=true
CONFIRM=""
DRY_RUN=false

SATURN_FPGA_FLASH_TRANSIENT_MARKER="${SATURN_FPGA_FLASH_TRANSIENT_MARKER:-0}"
ORIGINAL_ARGS=("$@")

progress(){ echo "Progress: $1%"; }
info(){ echo "$@"; }
warn(){ echo "WARN: $@"; }
err(){ echo "ERR: $@" >&2; exit 1; }

running_under_saturngo_service() {
  grep -q 'saturn-go\.service' /proc/self/cgroup 2>/dev/null
}

maybe_reexec_via_systemd_run() {
  [[ "$SATURN_FPGA_FLASH_TRANSIENT_MARKER" == "1" ]] && return 0
  running_under_saturngo_service || return 0
  command -v systemd-run >/dev/null 2>&1 || return 0
  systemd-run --help 2>&1 | grep -q -- '--pipe' || return 0

  local unit="saturn-flash-fpga-$(date +%Y%m%d%H%M%S)-$$"
  local -a cmd=(
    systemd-run --quiet --wait --collect --pipe --service-type=exec
    --unit "$unit"
    --setenv=SATURN_FPGA_FLASH_TRANSIENT_MARKER=1
  )
  for key in SUDO_USER HOME SATURN_ACTIVE_REPO_ROOT SATURN_REPO_ROOT SATURN_DIR SATURN_FPGA_DIR; do
    if [[ -n "${!key:-}" ]]; then
      cmd+=(--setenv="${key}=${!key}")
    fi
  done
  cmd+=("$0" "$@")
  exec "${cmd[@]}"
}

discover_user_home(){
  local home_guess="${HOME:-/root}"
  if [[ -n "${SUDO_USER:-}" ]]; then
    local sudo_home
    sudo_home="$(getent passwd "$SUDO_USER" | cut -d: -f6 || true)"
    if [[ -n "$sudo_home" ]]; then
      home_guess="$sudo_home"
    fi
  fi
  echo "$home_guess"
}

resolve_saturn_dir(){
  local user_home="$1"
  local candidates=()

  if [[ -n "${SATURN_ACTIVE_REPO_ROOT:-}" ]]; then
    candidates+=("$SATURN_ACTIVE_REPO_ROOT")
  fi
  if [[ -n "${SATURN_REPO_ROOT:-}" ]]; then
    candidates+=("$SATURN_REPO_ROOT")
  fi
  if [[ -n "${SATURN_DIR:-}" ]]; then
    candidates+=("$SATURN_DIR")
  fi
  if [[ -n "${SATURN_FPGA_DIR:-}" ]]; then
    candidates+=("$(dirname "$SATURN_FPGA_DIR")")
  fi
  candidates+=("$user_home/github/Saturn" "$user_home/github/saturn")
  if [[ -n "${SUDO_USER:-}" ]]; then
    candidates+=("/home/$SUDO_USER/github/Saturn" "/home/$SUDO_USER/github/saturn")
  fi
  if [[ -d /home ]]; then
    local home_entry
    for home_entry in /home/*; do
      [[ -d "$home_entry" ]] || continue
      candidates+=("$home_entry/github/Saturn" "$home_entry/github/saturn")
    done
  fi

  local c
  for c in "${candidates[@]}"; do
    if [[ -d "$c/sw_tools/load-FPGA" || -d "$c/sw_tools/spiload" ]]; then
      echo "$c"
      return 0
    fi
  done
  return 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --image)
      [[ $# -ge 2 ]] || err "--image requires a path"
      IMAGE="$2"
      shift 2
      ;;
    --latest)
      IMAGE="latest"
      shift
      ;;
    --primary)
      USE_PRIMARY=true
      shift
      ;;
    --fallback)
      USE_FALLBACK=true
      shift
      ;;
    --verify)
      VERIFY=true
      shift
      ;;
    --no-verify)
      VERIFY=false
      shift
      ;;
    --confirm)
      [[ $# -ge 2 ]] || err "--confirm requires a value"
      CONFIRM="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    *) err "Unknown argument: $1" ;;
  esac
done

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  err "Run as root."
fi

maybe_reexec_via_systemd_run "${ORIGINAL_ARGS[@]}"

progress 5

if [[ -f /proc/sys/kernel/osrelease ]] && grep -qi microsoft /proc/sys/kernel/osrelease; then
  err "WSL detected. This script must run on a Raspberry Pi."
fi

if ! grep -qi "Raspberry Pi" /proc/cpuinfo 2>/dev/null; then
  warn "Raspberry Pi not detected. Proceeding anyway."
fi

USER_HOME="$(discover_user_home)"
SATURN_DIR="$(resolve_saturn_dir "$USER_HOME" || true)"
if [[ -z "$SATURN_DIR" ]]; then
  err "Saturn repo not found. Set SATURN_DIR or SATURN_FPGA_DIR."
fi

if [[ -n "${SATURN_FPGA_DIR:-}" ]]; then
  FPGA_DIR="$SATURN_FPGA_DIR"
else
  FPGA_DIR="${SATURN_DIR}/FPGA"
fi
LOADER_DIR="${SATURN_DIR}/sw_tools/load-FPGA"
LOADER_BIN="${LOADER_DIR}/load-FPGA"

if [[ "$USE_PRIMARY" == true && "$USE_FALLBACK" == true ]]; then
  err "Choose only one: --primary or --fallback"
fi
if [[ "$USE_PRIMARY" == false && "$USE_FALLBACK" == false ]]; then
  USE_PRIMARY=true
fi

TARGET_NAME="PRIMARY"
if [[ "$USE_FALLBACK" == true ]]; then
  TARGET_NAME="FALLBACK"
fi

if [[ ! -d "$FPGA_DIR" ]]; then
  err "FPGA directory not found: $FPGA_DIR"
fi

if [[ -z "$IMAGE" || "$IMAGE" == "latest" ]]; then
  IMAGE="$(ls -t "$FPGA_DIR"/*.bin 2>/dev/null | head -n1 || true)"
  [[ -n "$IMAGE" ]] || err "No FPGA .bin image found in $FPGA_DIR"
fi

if [[ "$IMAGE" != /* && -f "$FPGA_DIR/$IMAGE" ]]; then
  IMAGE="$FPGA_DIR/$IMAGE"
fi

if [[ ! -f "$IMAGE" ]]; then
  err "Image not found: $IMAGE"
fi
if [[ "${IMAGE##*.}" != "bin" ]]; then
  err "load-FPGA requires a .bin image file: $IMAGE"
fi

progress 15

if [[ ! -d "$LOADER_DIR" ]]; then
  err "load-FPGA source directory not found: $LOADER_DIR"
fi

if [[ ! -x "$LOADER_BIN" ]]; then
  info "Building load-FPGA..."
  if [[ "$DRY_RUN" == true ]]; then
    info "[Dry Run] make -C ${LOADER_DIR}"
  else
    make -C "${LOADER_DIR}"
  fi
fi

if [[ ! -x "$LOADER_BIN" && "$DRY_RUN" == false ]]; then
  err "load-FPGA not found or not executable at $LOADER_BIN"
fi

progress 25

SHA="$(sha256sum "$IMAGE" | awk '{print $1}')"
SHORT="${SHA:0:6}"

info "FPGA image: $IMAGE"
info "SHA256: $SHA"
info "Target slot: $TARGET_NAME"
info "Verification: $([[ "$VERIFY" == true ]] && echo ON || echo OFF)"
info "Loader: $LOADER_BIN"

if [[ "$CONFIRM" != "FLASH" && "$CONFIRM" != "$SHORT" ]]; then
  err "Confirmation required. Re-run with: --confirm FLASH (or --confirm ${SHORT})"
fi

progress 35

CMD=("$LOADER_BIN" -b "$IMAGE")
if [[ "$VERIFY" == true ]]; then
  CMD+=(-v)
fi
if [[ "$USE_FALLBACK" == true ]]; then
  CMD+=(-f)
fi

info "Running: ${CMD[*]}"

if [[ "$DRY_RUN" == true ]]; then
  info "[Dry Run] Flash skipped."
  progress 100
  exit 0
fi

if ! "${CMD[@]}"; then
  err "load-FPGA failed"
fi

progress 100
info "FPGA flash complete."
