#!/usr/bin/env bash
set -Eeuo pipefail

STATUS_FILE=""
LOG_FILE=""
SCRIPT_PATH="${PIHPSDR_INSTALLER_SCRIPT:-/opt/saturn-go/scripts/update-pihpsdr.py}"

usage() {
  cat <<'EOF'
Usage: pihpsdr-installer-run.sh --status-file PATH --log-file PATH
EOF
}

write_status() {
  local state="$1"
  local message="${2:-}"
  local status_dir
  status_dir="$(dirname "$STATUS_FILE")"
  mkdir -p "$status_dir"
  printf '%s|%s|%s\n' "$state" "$(date '+%Y-%m-%d %H:%M:%S')" "$message" >"$STATUS_FILE"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --status-file)
      STATUS_FILE="${2:-}"
      shift 2
      ;;
    --log-file)
      LOG_FILE="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ -n "$STATUS_FILE" ]] || {
  usage >&2
  exit 2
}
[[ -n "$LOG_FILE" ]] || {
  usage >&2
  exit 2
}

mkdir -p "$(dirname "$LOG_FILE")"
: >"$LOG_FILE"
write_status "RUNNING" "Starting piHPSDR install. This can take a few minutes."

{
  printf '[%s] piHPSDR installer started\n' "$(date '+%Y-%m-%d %H:%M:%S')"
  printf '[%s] Using installer script: %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$SCRIPT_PATH"

  if [[ ! -f "$SCRIPT_PATH" ]]; then
    printf '[%s] ERROR: update-pihpsdr.py not found at %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$SCRIPT_PATH"
    write_status "FAILED" "Installed update-pihpsdr.py script was not found."
    exit 1
  fi

  cd "$HOME"
  if stdbuf -oL -eL /usr/bin/python3 "$SCRIPT_PATH" -y --verbose; then
    printf '\n[%s] piHPSDR installer completed successfully\n' "$(date '+%Y-%m-%d %H:%M:%S')"
    write_status "SUCCESS" "piHPSDR install completed successfully."
    exit 0
  fi

  rc=$?
  printf '\n[%s] piHPSDR installer failed with exit code %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$rc"
  write_status "FAILED" "piHPSDR install failed. Review the terminal output below."
  exit "$rc"
} >>"$LOG_FILE" 2>&1
