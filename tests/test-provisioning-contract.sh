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

# Canonical provisioning installs Saturn Go before Saturn Bridge. The nested
# installer must defer readiness, preserve the outer Bridge requirement in the
# service environment, and the orchestrator must perform the exact-commit
# readiness check after installing Bridge.
PROVISIONER="$REPO_ROOT/provision/cloud-init/provision-saturn.sh"
MANAGER_INSTALLER="$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
grep -Fq 'SATURN_DEFER_FINAL_READINESS=1' "$PROVISIONER"
grep -Fq "SATURN_READY_REQUIRE_BRIDGE=\"\$SATURN_REQUIRE_SATURN_BRIDGE\"" "$PROVISIONER"
grep -Fq 'verify_saturn_go_target_readiness' "$PROVISIONER"
grep -Fq "Environment=SATURN_READY_REQUIRE_BRIDGE=\${SATURN_READY_REQUIRE_BRIDGE}" "$MANAGER_INSTALLER"
grep -Fq "if env_flag_enabled \"\$SATURN_DEFER_FINAL_READINESS\"" "$MANAGER_INSTALLER"

# XDMA device nodes are group-restricted. Canonical and manual installers must
# grant the configured desktop operator access so piHPSDR and deskHPSDR can
# open /dev/xdma0_user after p2app.service is stopped.
grep -Fq "SATURN_USER=\"\$SATURN_USER\"" "$REPO_ROOT/provision/cloud-init/provision-saturn.sh"
grep -Fq "ensure_operator_xdma_access \"\$OPERATOR_USER\"" "$REPO_ROOT/rules/install-rules.sh"
grep -Fq "ensure_operator_xdma_access \"\$operator_user\"" "$REPO_ROOT/scripts/install-udev-rules-on-current-image.sh"
grep -Fq "usermod -a -G \"\$P2APP_SERVICE_GROUP\" \"\$CONTROL_USER\"" "$REPO_ROOT/sw_tools/p2app-control/install.sh"

printf 'provisioning contract tests passed\n'
