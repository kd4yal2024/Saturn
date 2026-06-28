#!/usr/bin/env bash
# Install the Saturn XDMA driver as a DKMS-managed module source.

set -euo pipefail

PACKAGE_NAME="${SATURN_XDMA_DKMS_NAME:-saturn-xdma}"
PACKAGE_VERSION="${SATURN_XDMA_DKMS_VERSION:-2020.1.8-saturn}"
TARGET_KERNEL="${SATURN_XDMA_DKMS_KERNEL:-$(uname -r)}"
MANUAL_POSTINST_HOOK="${SATURN_XDMA_MANUAL_POSTINST_HOOK:-/etc/kernel/postinst.d/saturn-xdma}"
DRY_RUN=0
FORCE=0
UNINSTALL=0
KEEP_MANUAL_POSTINST=0

info(){ printf '[INFO] %s\n' "$*"; }
ok(){ printf '[ OK ] %s\n' "$*"; }
warn(){ printf '[WARN] %s\n' "$*" >&2; }
die(){ printf '[ERR ] %s\n' "$*" >&2; exit 1; }
run(){
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '[dry-run]'
    printf ' %q' "$@"
    printf '\n'
    return 0
  fi
  "$@"
}

copy_driver_sources(){
  local src
  # Keep the DKMS source payload limited to the supported active driver inputs.
  for src in \
    "$DRIVER_DIR/Makefile" \
    "$DRIVER_DIR/CHANGELOG.md" \
    "$DRIVER_DIR/readme.md" \
    "$DRIVER_DIR"/*.c \
    "$DRIVER_DIR"/*.h
  do
    [[ -e "$src" ]] || continue
    [[ "$(basename "$src")" != *.mod.c ]] || continue
    run cp -a "$src" "$SRC_STAGE/xdma/"
  done
}

dkms_registered(){
  [[ -n "$(dkms status -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" 2>/dev/null || true)" ]]
}

disable_manual_postinst_hook(){
  local disabled_hook="${MANUAL_POSTINST_HOOK}.disabled-by-dkms"

  if [[ "$KEEP_MANUAL_POSTINST" -eq 1 ]]; then
    warn "Leaving legacy manual kernel postinst hook active: $MANUAL_POSTINST_HOOK"
    return 0
  fi

  if [[ -e "$MANUAL_POSTINST_HOOK" || -L "$MANUAL_POSTINST_HOOK" ]]; then
    warn "Disabling legacy manual XDMA kernel postinst hook: $MANUAL_POSTINST_HOOK"
    if [[ -e "$disabled_hook" || -L "$disabled_hook" ]]; then
      run rm -f "$disabled_hook"
    fi
    run mv "$MANUAL_POSTINST_HOOK" "$disabled_hook"
    ok "Legacy manual hook disabled: $disabled_hook"
  else
    ok "Legacy manual XDMA kernel postinst hook is not active."
  fi
}

uninstall_dkms(){
  info "Uninstalling DKMS package: ${PACKAGE_NAME}/${PACKAGE_VERSION}"
  if dkms_registered; then
    run dkms remove -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" --all
  else
    warn "DKMS package is not registered: ${PACKAGE_NAME}/${PACKAGE_VERSION}"
  fi

  if [[ -d "$SRC_ROOT" || -L "$SRC_ROOT" ]]; then
    run rm -rf "$SRC_ROOT"
  fi
  run depmod "$TARGET_KERNEL"
  ok "DKMS package removed. The legacy manual postinst hook was not restored."
}

usage(){
  cat <<EOF
Usage:
  sudo bash scripts/install-xdma-dkms.sh [options]

Options:
  --kernel <release>   Build/install for the selected kernel (default: uname -r)
  --force              Remove any existing DKMS registration for this version first
  --uninstall          Remove this DKMS package/version and its /usr/src source
  --keep-manual-postinst
                       Keep /etc/kernel/postinst.d/saturn-xdma active after install
  --dry-run            Print actions without changing the system
  -h, --help           Show this help

By default, a successful DKMS install disables the legacy manual kernel
postinst hook at /etc/kernel/postinst.d/saturn-xdma. This prevents both DKMS
and the older fix-xdma.sh hook from rebuilding/installing XDMA on the same
kernel package update.

Environment:
  SATURN_XDMA_DKMS_NAME       Package name (default: saturn-xdma)
  SATURN_XDMA_DKMS_VERSION    Package version (default: 2020.1.8-saturn)
  SATURN_XDMA_DKMS_KERNEL     Target kernel release
  SATURN_XDMA_MANUAL_POSTINST_HOOK
                                Legacy manual kernel hook path override
  SATURN_REPO_DIR             Saturn repo root override
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --kernel)
      [[ $# -ge 2 && -n "${2:-}" ]] || die "--kernel requires a release"
      TARGET_KERNEL="$2"
      shift 2
      ;;
    --force)
      FORCE=1
      shift
      ;;
    --uninstall)
      UNINSTALL=1
      shift
      ;;
    --keep-manual-postinst)
      KEEP_MANUAL_POSTINST=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "Unknown option: $1"
      ;;
  esac
done

if [[ "$DRY_RUN" -ne 1 && "$(id -u)" -ne 0 ]]; then
  die "Run as root, or use --dry-run for inspection."
fi

if [[ -n "${SATURN_REPO_DIR:-}" ]]; then
  REPO_ROOT="$SATURN_REPO_DIR"
else
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

DRIVER_DIR="$REPO_ROOT/linuxdriver/xdma"
INCLUDE_DIR="$REPO_ROOT/linuxdriver/include"
DKMS_TEMPLATE="$REPO_ROOT/linuxdriver/dkms/dkms.conf"
SRC_ROOT="/usr/src/${PACKAGE_NAME}-${PACKAGE_VERSION}"
SRC_STAGE="${SRC_ROOT}.stage.$$"

[[ -f "$DRIVER_DIR/Makefile" ]] || die "XDMA driver source not found: $DRIVER_DIR"
[[ -f "$INCLUDE_DIR/libxdma_api.h" ]] || die "XDMA include source not found: $INCLUDE_DIR/libxdma_api.h"
[[ -f "$DKMS_TEMPLATE" ]] || die "DKMS template not found: $DKMS_TEMPLATE"
command -v dkms >/dev/null 2>&1 || die "dkms is not installed. Install the dkms package first."

info "Repo root: $REPO_ROOT"
info "DKMS package: ${PACKAGE_NAME}/${PACKAGE_VERSION}"
info "Target kernel: $TARGET_KERNEL"
info "Source root: $SRC_ROOT"

if [[ "$UNINSTALL" -eq 1 ]]; then
  uninstall_dkms
  exit 0
fi

if dkms_registered; then
  if [[ "$FORCE" -eq 1 ]]; then
    warn "Removing existing DKMS registration for ${PACKAGE_NAME}/${PACKAGE_VERSION}"
    run dkms remove -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" --all
  else
    die "DKMS package is already registered. Rerun with --force or bump SATURN_XDMA_DKMS_VERSION."
  fi
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  run mkdir -p "$SRC_STAGE/xdma" "$SRC_STAGE/include"
else
  rm -rf "$SRC_STAGE"
  install -d -m 0755 "$SRC_STAGE/xdma" "$SRC_STAGE/include"
fi

copy_driver_sources
run cp -a "$INCLUDE_DIR/." "$SRC_STAGE/include/"
run cp -a "$DKMS_TEMPLATE" "$SRC_STAGE/dkms.conf"
run sed -i "s/^PACKAGE_NAME=.*/PACKAGE_NAME=\"${PACKAGE_NAME}\"/" "$SRC_STAGE/dkms.conf"
run sed -i "s/^PACKAGE_VERSION=.*/PACKAGE_VERSION=\"${PACKAGE_VERSION}\"/" "$SRC_STAGE/dkms.conf"

if [[ "$DRY_RUN" -eq 1 ]]; then
  run rm -rf "$SRC_ROOT"
  run mv "$SRC_STAGE" "$SRC_ROOT"
else
  rm -rf "$SRC_ROOT"
  mv "$SRC_STAGE" "$SRC_ROOT"
  chown -R root:root "$SRC_ROOT"
fi

run dkms add -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION"
run dkms build -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL"
run dkms install -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL"
run depmod "$TARGET_KERNEL"
disable_manual_postinst_hook

ok "DKMS-managed XDMA installed for ${TARGET_KERNEL}."
ok "Future kernel installs can rebuild with: dkms autoinstall -m ${PACKAGE_NAME}/${PACKAGE_VERSION}"
