#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALLER="$REPO_ROOT/scripts/install-xdma-dkms.sh"
MODULE_OPTIONS="$REPO_ROOT/linuxdriver/etc/modprobe.d/saturn-xdma.conf"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

fail(){
  printf 'XDMA DKMS idempotency contract failed: %s\n' "$*" >&2
  exit 1
}

make_fixture(){
  local root="$1"
  install -d -m 0755 "$root/linuxdriver/xdma" "$root/linuxdriver/include" "$root/linuxdriver/dkms"
  printf 'obj-m += xdma.o\n' >"$root/linuxdriver/xdma/Makefile"
  printf 'int saturn_xdma_fixture(void) { return 1; }\n' >"$root/linuxdriver/xdma/driver.c"
  printf '#define XDMA_FIXTURE_VERSION 1\n' >"$root/linuxdriver/xdma/version.h"
  printf '#pragma once\n' >"$root/linuxdriver/include/libxdma_api.h"
  printf 'PACKAGE_NAME="saturn-xdma"\nPACKAGE_VERSION="fixture"\n' >"$root/linuxdriver/dkms/dkms.conf"
}

fixture_a="$TMP_ROOT/a"
fixture_b="$TMP_ROOT/b"
make_fixture "$fixture_a"

version_a="$(SATURN_REPO_DIR="$fixture_a" "$INSTALLER" --print-source-version)"
[[ "$version_a" =~ ^2020\.1\.8-saturn\.[0-9a-f]{12}$ ]] \
  || fail "source-derived version has an unexpected format: $version_a"

printf 'generated module metadata one\n' >"$fixture_a/linuxdriver/xdma/xdma.mod.c"
version_generated_one="$(SATURN_REPO_DIR="$fixture_a" "$INSTALLER" --print-source-version)"
printf 'generated module metadata two\n' >"$fixture_a/linuxdriver/xdma/xdma.mod.c"
version_generated_two="$(SATURN_REPO_DIR="$fixture_a" "$INSTALLER" --print-source-version)"
[[ "$version_generated_one" == "$version_a" && "$version_generated_two" == "$version_a" ]] \
  || fail "generated xdma.mod.c changed the source-derived version"

cp -a "$fixture_a" "$fixture_b"
version_b="$(SATURN_REPO_DIR="$fixture_b" "$INSTALLER" --print-source-version)"
[[ "$version_b" == "$version_a" ]] \
  || fail "moving an identical checkout changed the source-derived version"

printf 'int saturn_xdma_fixture(void) { return 2; }\n' >"$fixture_b/linuxdriver/xdma/driver.c"
version_changed="$(SATURN_REPO_DIR="$fixture_b" "$INSTALLER" --print-source-version)"
[[ "$version_changed" != "$version_a" ]] \
  || fail "changing driver source did not change the source-derived version"

grep -Fq 'command+=(--force)' "$INSTALLER" \
  || fail "a successfully built replacement cannot override an older installed DKMS package"
grep -Fq 'install_module_options' "$INSTALLER" \
  || fail "DKMS installation does not persist Saturn module options"
grep -Fq 'SATURN_XDMA_MODPROBE_CONFIG' "$INSTALLER" \
  || fail "persistent module-options destination is not configurable"
grep -Fq 'options xdma completion_kthread_priority=20 transfer_latency_warn_us=5000' \
  "$MODULE_OPTIONS" \
  || fail "validated Saturn XDMA module options are missing"

printf 'XDMA DKMS idempotency contract passed\n'
