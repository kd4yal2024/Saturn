#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_USER="$(id -un)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

output="$("$REPO_ROOT/install.sh" --dry-run --user "$TEST_USER")"
grep -Fq 'profile: appliance' <<<"$output"
grep -Fq 'packages=1 driver=1 p2=1 saturn-go=1 verify=hardware' <<<"$output"
grep -Fq 'pihpsdr-installer=1' <<<"$output"

output="$(SATURN_PIHPSDR_INSTALLER_ENABLED=0 "$REPO_ROOT/install.sh" --dry-run --user "$TEST_USER")"
grep -Fq 'pihpsdr-installer=0' <<<"$output"

output="$("$REPO_ROOT/install.sh" --dry-run --user "$TEST_USER" --profile image-factory --skip-packages)"
grep -Fq 'profile: image-factory' <<<"$output"
grep -Fq 'packages=0 driver=1 p2=1 saturn-go=1 verify=software' <<<"$output"

output="$("$REPO_ROOT/install.sh" --dry-run --user "$TEST_USER" --profile image-factory --verify hardware)"
grep -Fq 'packages=1 driver=1 p2=1 saturn-go=1 verify=hardware' <<<"$output"

# A checkout is supported outside /home/<user>/github/Saturn, including paths
# containing spaces. A symlink keeps this test fast while exercising discovery.
ln -s "$REPO_ROOT" "$TMP_DIR/Saturn checkout"
output="$("$TMP_DIR/Saturn checkout/install.sh" --dry-run --user "$TEST_USER")"
grep -Fq "repository: $TMP_DIR/Saturn checkout" <<<"$output"

printf 'provisioning contract tests passed\n'
