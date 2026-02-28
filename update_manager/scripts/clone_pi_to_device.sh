#!/usr/bin/env bash
set -euo pipefail

# clone_pi_to_device.sh - Clone /dev/mmcblk0 to a target device (e.g. /dev/sda)
# Version: 1.0.0
#
# Usage:
#   ./clone_pi_to_device.sh --target /dev/sdX
#
# Notes:
# - DEST device will be overwritten.
# - Requires root (or passwordless sudo).

SRC_DEV="/dev/mmcblk0"
TARGET=""
SUDO=""
VERIFY_COMPARE=0

progress(){ echo "Progress: $1%"; }
info(){ echo "$@"; }
err(){ echo "ERR: $@" >&2; exit 1; }

if [[ "$(id -u)" -ne 0 ]]; then
  if sudo -n true 2>/dev/null; then
    SUDO="sudo -n"
  else
    err "Root privileges required. Run with sudo or enable passwordless sudo."
  fi
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      [[ $# -ge 2 ]] || err "--target requires a device path"
      TARGET="$2"
      shift 2
      ;;
    --verify-compare)
      VERIFY_COMPARE=1
      shift
      ;;
    *) err "Unknown argument: $1" ;;
  esac
done

progress 5

if [[ ! -b "$SRC_DEV" ]]; then
  err "Source device $SRC_DEV not found."
fi
if [[ -z "$TARGET" ]]; then
  err "Target device not specified."
fi
if [[ ! -b "$TARGET" ]]; then
  err "Target device $TARGET not found."
fi
if [[ "$TARGET" == "$SRC_DEV" ]]; then
  err "Target cannot be the same as source."
fi

SRC_BYTES="$($SUDO blockdev --getsize64 "$SRC_DEV")"
TGT_BYTES="$($SUDO blockdev --getsize64 "$TARGET")"
info "Source: ${SRC_DEV} (${SRC_BYTES} bytes)"
info "Target: ${TARGET} (${TGT_BYTES} bytes)"
if [[ "$TGT_BYTES" -lt "$SRC_BYTES" ]]; then
  err "Target device too small."
fi

progress 10

if command -v pv >/dev/null 2>&1; then
  info "Cloning with pv progress..."
  # pv -n prints integer percentage to stderr; map clone phase into 10..84%
  $SUDO pv -n -s "$SRC_BYTES" "$SRC_DEV" \
    | $SUDO dd of="$TARGET" bs=4M conv=fsync status=none \
    2> >(while read -r p; do
          [[ "$p" =~ ^[0-9]+$ ]] || continue
          clone_p=$((10 + (p * 74 / 100)))
          progress "$clone_p"
        done)
else
  info "Cloning with dd status=progress..."
  $SUDO dd if="$SRC_DEV" of="$TARGET" bs=4M status=progress conv=fsync
fi

progress 85
info "Post-clone validation: refreshing partition table..."
$SUDO sync || true
$SUDO partprobe "$TARGET" 2>/dev/null || true
$SUDO udevadm settle 2>/dev/null || true

part_suffix() {
  local dev="$1"
  if [[ "$dev" =~ [0-9]$ ]]; then
    printf 'p'
  else
    printf ''
  fi
}

compare_part_layout() {
  local src="$1" tgt="$2"
  local src_lines tgt_lines i
  mapfile -t src_lines < <($SUDO lsblk -nrbo START,SIZE,TYPE "$src" | awk '$3=="part"{print $1 ":" $2}')
  mapfile -t tgt_lines < <($SUDO lsblk -nrbo START,SIZE,TYPE "$tgt" | awk '$3=="part"{print $1 ":" $2}')
  if [[ "${#src_lines[@]}" -eq 0 ]]; then
    err "Validation failed: no source partitions found on $src"
  fi
  if [[ "${#src_lines[@]}" -ne "${#tgt_lines[@]}" ]]; then
    err "Validation failed: partition count mismatch (${#src_lines[@]} source vs ${#tgt_lines[@]} target)"
  fi
  for ((i=0; i<${#src_lines[@]}; i++)); do
    if [[ "${src_lines[$i]}" != "${tgt_lines[$i]}" ]]; then
      err "Validation failed: partition layout mismatch at index $i (${src_lines[$i]} != ${tgt_lines[$i]})"
    fi
  done
  info "Partition layout check: OK (${#src_lines[@]} partitions)"
}

validate_fsck_ro() {
  local dev="$1"
  local p part fstype rc
  local suffix
  suffix="$(part_suffix "$dev")"
  for p in 1 2 3 4 5 6 7 8; do
    part="${dev}${suffix}${p}"
    [[ -b "$part" ]] || continue
    fstype="$($SUDO blkid -o value -s TYPE "$part" 2>/dev/null || true)"
    if [[ -z "$fstype" ]]; then
      info "Validation: $part filesystem type unknown (skipping fsck)"
      continue
    fi
    case "$fstype" in
      vfat|fat|fat16|fat32|msdos)
        info "Validation: fsck.vfat -n $part ($fstype)"
        set +e
        $SUDO fsck.vfat -n "$part"
        rc=$?
        set -e
        if [[ "$rc" -eq 1 ]]; then
          info "Validation warning: FAT read-only fsck reported fixable issues on $part (common on live-cloned boot partitions; e.g. dirty bit set)."
          continue
        fi
        ;;
      ext2|ext3|ext4)
        info "Validation: e2fsck -fn $part ($fstype)"
        set +e
        $SUDO e2fsck -fn "$part"
        rc=$?
        set -e
        # e2fsck uses a bitmask. In read-only mode on a live-cloned source, bit 1 ("errors found")
        # can be expected because no writes/journal replay are performed. Treat higher bits as failure.
        if (( (rc & ~1) != 0 )); then
          err "Validation failed: read-only fsck reported issues on $part (exit $rc)"
        fi
        if [[ "$rc" -eq 1 ]]; then
          info "Validation warning: ext read-only fsck found fixable issues on $part (common on live clones); continuing."
          continue
        fi
        ;;
      *)
        info "Validation: unsupported fs type on $part ($fstype), skipping fsck"
        continue
        ;;
    esac
    if [[ "$rc" -ne 0 ]]; then
      err "Validation failed: read-only fsck reported issues on $part (exit $rc)"
    fi
  done
  info "Read-only filesystem checks: OK"
}

progress 90
compare_part_layout "$SRC_DEV" "$TARGET"

progress 95
validate_fsck_ro "$TARGET"

if [[ "$VERIFY_COMPARE" -eq 1 ]]; then
  progress 98
  info "Optional byte-compare: cmp first ${SRC_BYTES} bytes (live source may differ if data changes during clone)..."
  set +e
  $SUDO cmp -n "$SRC_BYTES" "$SRC_DEV" "$TARGET" >/dev/null
  rc=$?
  set -e
  if [[ "$rc" -ne 0 ]]; then
    err "Optional compare failed (cmp exit $rc). Live source writes can cause mismatches."
  fi
  info "Optional byte-compare: OK"
fi

progress 100
info "Done"
