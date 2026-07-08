#!/usr/bin/env bash
set -euo pipefail

target="${1:-}"

die() {
  echo "ERR: $*" >&2
  exit 1
}

log() {
  echo "$*"
}

part_suffix() {
  local dev="$1"
  if [[ "$dev" =~ [0-9]$ ]]; then
    printf 'p'
  fi
}

device_allowed() {
  local name="$1"
  local sys_block_path="/sys/block/$name"

  case "$name" in
    mmcblk0|loop*|ram*|zram*|dm-*|md*|nbd*|sr*) return 1 ;;
  esac
  [[ -d "$sys_block_path" ]] || return 1
  if [[ -r "$sys_block_path/removable" ]] && [[ "$(tr -d '[:space:]' <"$sys_block_path/removable")" == "1" ]]; then
    return 0
  fi
  local device_path
  device_path="$(readlink -f "$sys_block_path/device" 2>/dev/null || true)"
  [[ "$device_path" == *"/usb"* ]]
}

[[ -n "$target" ]] || die "target device required"
[[ "$target" == /dev/* ]] || die "target must be a /dev path"
[[ "$target" != "/dev/mmcblk0" ]] || die "target cannot be source device"
[[ -b "$target" ]] || die "target device not found: $target"

name="${target#/dev/}"
device_allowed "$name" || die "target device is not removable"

log "Target: $target"

while read -r path kind; do
  [[ "$kind" == "part" ]] || continue
  if umount "$path" >/tmp/saturn-pi-wipe-umount.log 2>&1; then
    log "Unmounted $path"
  else
    msg="$(tr '\n' ' ' </tmp/saturn-pi-wipe-umount.log | sed 's/[[:space:]]\+/ /g; s/[[:space:]]$//')"
    log "Unmount $path: ${msg:-not mounted}"
  fi
done < <(lsblk -ln -o PATH,TYPE "$target")

wipe_output="$(wipefs -af "$target" 2>&1 || true)"
if [[ -n "$wipe_output" ]]; then
  log "wipefs: $wipe_output"
else
  log "wipefs: signatures cleared"
fi

if command -v sgdisk >/dev/null 2>&1; then
  if sgdisk --zap-all "$target" >/tmp/saturn-pi-wipe-sgdisk.log 2>&1; then
    log "sgdisk: GPT/MBR metadata zapped"
  else
    msg="$(tr '\n' ' ' </tmp/saturn-pi-wipe-sgdisk.log | sed 's/[[:space:]]\+/ /g; s/[[:space:]]$//')"
    log "sgdisk: ${msg:-skipped}"
  fi
else
  log "sgdisk: skipped (not installed)"
fi

size_bytes="$(blockdev --getsize64 "$target")"
[[ "$size_bytes" =~ ^[0-9]+$ ]] && (( size_bytes > 0 )) || die "target size is zero"
log "Size: $size_bytes bytes"

dd if=/dev/zero "of=$target" bs=1M count=16 conv=fsync status=none
log "Zeroed first 16 MiB"

mib=$((1024 * 1024))
size_mib=$((size_bytes / mib))
if (( size_mib > 16 )); then
  seek_mib=$((size_mib - 16))
  dd if=/dev/zero "of=$target" bs=1M count=16 "seek=$seek_mib" conv=fsync status=none
  log "Zeroed last 16 MiB"
fi

sync || true
partprobe "$target" 2>/dev/null || true
udevadm settle 2>/dev/null || true

log "Target wiped (signatures/partition metadata cleared)."
