#!/usr/bin/env bash
set -euo pipefail
: "${SATURN_ARM_SYSROOT:?explicit read-only ARM sysroot required}"
: "${SATURN_ARM_LLD:?explicit rust-lld path required}"
ARM_LINK_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec /usr/bin/clang --target=aarch64-linux-gnu --sysroot="$SATURN_ARM_SYSROOT" \
  -B "$SATURN_ARM_SYSROOT/usr/lib/gcc/aarch64-linux-gnu/14" \
  -L "$SATURN_ARM_SYSROOT/usr/lib/gcc/aarch64-linux-gnu/14" \
  -L "$SATURN_ARM_SYSROOT/usr/lib/aarch64-linux-gnu" \
  -Wl,-rpath-link,"$SATURN_ARM_SYSROOT/usr/lib/aarch64-linux-gnu" "$@" \
  -fuse-ld="$ARM_LINK_SCRIPT_DIR/reviewed-arm-lld.sh"
