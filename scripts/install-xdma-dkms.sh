#!/usr/bin/env bash
# Install the Saturn XDMA driver as a DKMS-managed module source.

set -euo pipefail

PACKAGE_NAME="${SATURN_XDMA_DKMS_NAME:-saturn-xdma}"
PACKAGE_VERSION="${SATURN_XDMA_DKMS_VERSION:-}"
TARGET_KERNEL="${SATURN_XDMA_DKMS_KERNEL:-$(uname -r)}"
MANUAL_POSTINST_HOOK="${SATURN_XDMA_MANUAL_POSTINST_HOOK:-/etc/kernel/postinst.d/saturn-xdma}"
MODPROBE_CONFIG="${SATURN_XDMA_MODPROBE_CONFIG:-/etc/modprobe.d/saturn-xdma.conf}"
DRY_RUN=0
FORCE=0
UNINSTALL=0
KEEP_MANUAL_POSTINST=0
PRUNE_OLD="${SATURN_XDMA_PRUNE_OLD:-0}"
PRINT_SOURCE_VERSION=0

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

file_digest(){
  sha256sum "$1" | awk '{print $1}'
}

driver_payload_revision(){
  local driver_dir="$1" include_dir="$2" src relative digest
  {
    while IFS= read -r -d '' src; do
      if [[ "$src" == "$driver_dir/"* ]]; then
        relative="xdma/${src#"$driver_dir/"}"
      else
        relative="include/${src#"$include_dir/"}"
      fi
      digest="$(file_digest "$src")"
      printf '%s\0%s\0' "$relative" "$digest"
    done < <(
      find "$driver_dir" "$include_dir" -type f \
        \( -name '*.c' -o -name '*.h' -o -name Makefile \) \
        ! -name '*.mod.c' -print0 | sort -z
    )
  } | sha256sum | awk '{print $1}'
}

source_revision(){
  local payload_digest template_digest
  payload_digest="$(driver_payload_revision "$DRIVER_DIR" "$INCLUDE_DIR")"
  template_digest="$(file_digest "$DKMS_TEMPLATE")"
  printf 'payload\0%s\0dkms.conf\0%s\0' "$payload_digest" "$template_digest" \
    | sha256sum | cut -c1-12
}

equivalent_installed_version(){
  local wanted status version installed_root installed_digest
  wanted="$(driver_payload_revision "$DRIVER_DIR" "$INCLUDE_DIR")"
  while IFS= read -r status; do
    [[ "$status" == *", ${TARGET_KERNEL},"* && "$status" == *": installed"* ]] || continue
    version="$(sed -n "s#^${PACKAGE_NAME}/\([^,]*\),.*#\1#p" <<<"$status")"
    [[ -n "$version" ]] || continue
    installed_root="/usr/src/${PACKAGE_NAME}-${version}"
    [[ -d "$installed_root/xdma" && -d "$installed_root/include" ]] || continue
    installed_digest="$(driver_payload_revision "$installed_root/xdma" "$installed_root/include")"
    if [[ "$installed_digest" == "$wanted" ]]; then
      printf '%s\n' "$version"
      return 0
    fi
  done < <(dkms status -m "$PACKAGE_NAME" 2>/dev/null || true)
  return 1
}

install_dkms_for_kernel(){
  local -a command=(dkms install -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL")
  if dkms status -m "$PACKAGE_NAME" 2>/dev/null \
      | grep -F ", ${TARGET_KERNEL}," \
      | grep -q ': installed'; then
    warn "Replacing the previously installed XDMA package only after the new module built successfully"
    command+=(--force)
  fi
  run "${command[@]}"
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

install_module_options(){
  [[ -f "$MODULE_OPTIONS_SOURCE" ]] || \
    die "Saturn XDMA module options not found: $MODULE_OPTIONS_SOURCE"
  run install -d -m 0755 "$(dirname "$MODPROBE_CONFIG")"
  run install -m 0644 "$MODULE_OPTIONS_SOURCE" "$MODPROBE_CONFIG"
  ok "Installed Saturn XDMA module options: $MODPROBE_CONFIG"
}

remove_module_options(){
  [[ -e "$MODPROBE_CONFIG" || -L "$MODPROBE_CONFIG" ]] || return 0
  if [[ -f "$MODPROBE_CONFIG" ]] && cmp -s "$MODULE_OPTIONS_SOURCE" "$MODPROBE_CONFIG"; then
    run rm -f "$MODPROBE_CONFIG"
    ok "Removed Saturn XDMA module options: $MODPROBE_CONFIG"
  else
    warn "Preserving modified XDMA module options: $MODPROBE_CONFIG"
  fi
}

finalize_install(){
  install_module_options
  disable_manual_postinst_hook
  prune_old_versions
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
  remove_module_options
  run depmod "$TARGET_KERNEL"
  local disabled_hook="${MANUAL_POSTINST_HOOK}.disabled-by-dkms"
  if [[ ! -e "$MANUAL_POSTINST_HOOK" && -e "$disabled_hook" ]]; then
    run mv "$disabled_hook" "$MANUAL_POSTINST_HOOK"
    ok "Restored legacy manual postinst hook: $MANUAL_POSTINST_HOOK"
  fi
  ok "DKMS package removed."
}

prune_old_versions(){
  local status version
  [[ "$PRUNE_OLD" == "1" ]] || return 0
  while IFS= read -r status; do
    version="$(sed -n "s#^${PACKAGE_NAME}/\([^,]*\),.*#\1#p" <<<"$status")"
    [[ -n "$version" && "$version" != "$PACKAGE_VERSION" ]] || continue
    warn "Pruning older verified DKMS version: ${PACKAGE_NAME}/${version}"
    run dkms remove -m "$PACKAGE_NAME" -v "$version" --all
    [[ -d "/usr/src/${PACKAGE_NAME}-${version}" ]] && run rm -rf "/usr/src/${PACKAGE_NAME}-${version}"
  done < <(dkms status -m "$PACKAGE_NAME" 2>/dev/null || true)
}

usage(){
  cat <<EOF
Usage:
  sudo bash scripts/install-xdma-dkms.sh [options]

Options:
  --kernel <release>   Build/install for the selected kernel (default: uname -r)
  --force              Rebuild this version for the selected kernel
  --uninstall          Remove this DKMS package/version and its /usr/src source
  --keep-manual-postinst
                       Keep /etc/kernel/postinst.d/saturn-xdma active after install
  --dry-run            Print actions without changing the system
  --print-source-version
                       Print the stable source-derived DKMS version and exit
  -h, --help           Show this help

By default, a successful DKMS install disables the legacy manual kernel
postinst hook at /etc/kernel/postinst.d/saturn-xdma. This prevents both DKMS
and the older fix-xdma.sh hook from rebuilding/installing XDMA on the same
kernel package update.

Environment:
  SATURN_XDMA_DKMS_NAME       Package name (default: saturn-xdma)
  SATURN_XDMA_DKMS_VERSION    Package version (default: source-derived)
  SATURN_XDMA_DKMS_KERNEL     Target kernel release
  SATURN_XDMA_PRUNE_OLD       Set to 1 to prune older versions after success
  SATURN_XDMA_MANUAL_POSTINST_HOOK
                                Legacy manual kernel hook path override
  SATURN_XDMA_MODPROBE_CONFIG   Persistent module-options path override
                                (default: /etc/modprobe.d/saturn-xdma.conf)
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
    --print-source-version)
      PRINT_SOURCE_VERSION=1
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

if [[ "$DRY_RUN" -ne 1 && "$PRINT_SOURCE_VERSION" -ne 1 && "$(id -u)" -ne 0 ]]; then
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
MODULE_OPTIONS_SOURCE="$REPO_ROOT/linuxdriver/etc/modprobe.d/saturn-xdma.conf"
if [[ -z "$PACKAGE_VERSION" ]]; then
  SOURCE_REV="$(source_revision)"
  PACKAGE_VERSION="2020.1.8-saturn.${SOURCE_REV}"
fi
SRC_ROOT="/usr/src/${PACKAGE_NAME}-${PACKAGE_VERSION}"
SRC_STAGE="${SRC_ROOT}.stage.$$"
trap '[[ -n "${SRC_STAGE:-}" && -d "${SRC_STAGE:-}" ]] && rm -rf "$SRC_STAGE"' EXIT

[[ -f "$DRIVER_DIR/Makefile" ]] || die "XDMA driver source not found: $DRIVER_DIR"
[[ -f "$INCLUDE_DIR/libxdma_api.h" ]] || die "XDMA include source not found: $INCLUDE_DIR/libxdma_api.h"
[[ -f "$DKMS_TEMPLATE" ]] || die "DKMS template not found: $DKMS_TEMPLATE"

if [[ "$PRINT_SOURCE_VERSION" -eq 1 ]]; then
  printf '%s\n' "$PACKAGE_VERSION"
  exit 0
fi
command -v dkms >/dev/null 2>&1 || die "dkms is not installed. Install the dkms package first."

info "Repo root: $REPO_ROOT"
info "DKMS package: ${PACKAGE_NAME}/${PACKAGE_VERSION}"
info "Target kernel: $TARGET_KERNEL"
info "Source root: $SRC_ROOT"

if [[ "$UNINSTALL" -eq 1 ]]; then
  uninstall_dkms
  exit 0
fi

if equivalent_version="$(equivalent_installed_version)" \
    && [[ "$equivalent_version" != "$PACKAGE_VERSION" ]]; then
  ok "Equivalent XDMA source is already installed for ${TARGET_KERNEL}: ${PACKAGE_NAME}/${equivalent_version}"
  finalize_install
  exit 0
fi

if dkms_registered; then
  if [[ "$FORCE" -eq 1 ]]; then
    if dkms status -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL" 2>/dev/null | grep -q 'installed'; then
      die "Refusing to remove an installed module before a replacement is healthy; use a new source-derived version"
    fi
    warn "Removing only ${TARGET_KERNEL} from existing ${PACKAGE_NAME}/${PACKAGE_VERSION} registration"
    run dkms remove -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL"
  elif dkms status -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL" 2>/dev/null | grep -q 'installed'; then
    ok "DKMS package is already installed for ${TARGET_KERNEL}."
    finalize_install
    exit 0
  fi

  [[ -d "$SRC_ROOT" ]] || die "Registered DKMS source is missing: $SRC_ROOT"
  run dkms build -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL"
  install_dkms_for_kernel
  run depmod "$TARGET_KERNEL"
  dkms status -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL" | grep -q 'installed' || \
    die "DKMS did not report an installed module for ${TARGET_KERNEL}"
  finalize_install
  ok "DKMS-managed XDMA installed for ${TARGET_KERNEL}."
  exit 0
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
install_dkms_for_kernel
run depmod "$TARGET_KERNEL"
if [[ "$DRY_RUN" -ne 1 ]]; then
  dkms status -m "$PACKAGE_NAME" -v "$PACKAGE_VERSION" -k "$TARGET_KERNEL" | grep -q 'installed' || \
    die "DKMS did not report an installed module for ${TARGET_KERNEL}"
fi
finalize_install

ok "DKMS-managed XDMA installed for ${TARGET_KERNEL}."
ok "Future kernel installs can rebuild with: dkms autoinstall -m ${PACKAGE_NAME}/${PACKAGE_VERSION}"
