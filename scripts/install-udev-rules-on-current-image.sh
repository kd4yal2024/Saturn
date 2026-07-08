#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: install-udev-rules-on-current-image.sh [--repo PATH] [--check-only]

Installs Saturn udev rules from the current Saturn repository.
USAGE
}

repo_root="${SATURN_REPO_ROOT:-/home/pi/github/Saturn}"
check_only=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      [[ $# -ge 2 ]] || { echo "ERROR: --repo requires a path" >&2; exit 2; }
      repo_root="$2"
      shift 2
      ;;
    --check-only)
      check_only=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

repo_root="$(realpath -m "$repo_root")"
rules_dir="$repo_root/rules"
xdma_rules_dir="$repo_root/linuxdriver/etc/udev/rules.d"
dest_dir="/etc/udev/rules.d"

if [[ ! -d "$repo_root/.git" ]] || [[ ! -f "$repo_root/update_manager/scripts/update-G2.py" ]]; then
  echo "ERROR: Not a Saturn repository root: $repo_root" >&2
  exit 1
fi

serial_rules="$rules_dir/61-g2-serial.rules"
xdma_rules="$xdma_rules_dir/60-xdma.rules"
xdma_command="$xdma_rules_dir/xdma-udev-command.sh"

if [[ ! -f "$serial_rules" ]]; then
  echo "ERROR: Saturn serial udev rules are missing: $serial_rules" >&2
  exit 1
fi

if (( check_only )); then
  echo "Saturn udev helper ready: $serial_rules"
  exit 0
fi

if [[ "$EUID" -ne 0 ]]; then
  echo "ERROR: This helper must run as root." >&2
  exit 1
fi

echo "Installing Saturn udev rules from: $repo_root"
install -d -m 0755 "$dest_dir"
install -m 0644 "$serial_rules" "$dest_dir/$(basename "$serial_rules")"

if [[ -f "$xdma_rules" ]]; then
  install -m 0644 "$xdma_rules" "$dest_dir/$(basename "$xdma_rules")"
fi
if [[ -f "$xdma_command" ]]; then
  install -m 0755 "$xdma_command" "$dest_dir/$(basename "$xdma_command")"
fi

udevadm control --reload-rules
udevadm trigger
udevadm trigger --subsystem-match=xdma || true

echo "Udev rules reloaded successfully."
