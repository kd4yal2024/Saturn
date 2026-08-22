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

# Resume markers must track the actual checkout content, including dirty and
# untracked work, and hardware verification must accept either healthy radio
# owner while still rejecting mixed or inactive ownership.
# shellcheck disable=SC1090
source "$PROVISIONER"
FINGERPRINT_REPO="$TMP_DIR/fingerprint-repo"
mkdir -p "$FINGERPRINT_REPO"
git -C "$FINGERPRINT_REPO" init -q
git -C "$FINGERPRINT_REPO" config user.name Saturn-Test
git -C "$FINGERPRINT_REPO" config user.email saturn-test@example.invalid
printf 'initial\n' >"$FINGERPRINT_REPO/tracked.txt"
git -C "$FINGERPRINT_REPO" add tracked.txt
git -C "$FINGERPRINT_REPO" commit -qm initial
SATURN_REPO_DIR="$FINGERPRINT_REPO"
# Consumed by repo_git() from the sourced provisioner.
# shellcheck disable=SC2034
SATURN_USER="$TEST_USER"
fingerprint_clean="$(repository_source_fingerprint)"
printf 'dirty\n' >>"$FINGERPRINT_REPO/tracked.txt"
fingerprint_dirty="$(repository_source_fingerprint)"
[[ "$fingerprint_clean" != "$fingerprint_dirty" ]]
printf 'untracked\n' >"$FINGERPRINT_REPO/new-source.txt"
fingerprint_untracked="$(repository_source_fingerprint)"
[[ "$fingerprint_dirty" != "$fingerprint_untracked" ]]

radio_backend_status_is_ready <<'JSON'
{"selected":"p2","runtime":"p2","services":{"p2app":"active","saturn_bridge":"active"},"mutual_exclusion_ok":true}
JSON
radio_backend_status_is_ready <<'JSON'
{"selected":"xdma","runtime":"xdma","services":{"p2app":"inactive","saturn_bridge":"active"},"mutual_exclusion_ok":true}
JSON
if radio_backend_status_is_ready <<'JSON'
{"selected":"xdma","runtime":"xdma","services":{"p2app":"active","saturn_bridge":"active"},"mutual_exclusion_ok":false}
JSON
then
  printf 'mixed P2/XDMA ownership unexpectedly passed verification\n' >&2
  exit 1
fi
if radio_backend_status_is_ready <<'JSON'
{"selected":"xdma","runtime":"xdma","services":{"p2app":"inactive","saturn_bridge":"inactive"},"mutual_exclusion_ok":true}
JSON
then
  printf 'inactive selected backend unexpectedly passed verification\n' >&2
  exit 1
fi
# Restored for any later sourced provisioner helpers.
# shellcheck disable=SC2034
SATURN_REPO_DIR="$REPO_ROOT"

# XDMA device nodes are group-restricted. Canonical and manual installers must
# grant the configured desktop operator access so piHPSDR and deskHPSDR can
# open /dev/xdma0_user after p2app.service is stopped.
grep -Fq "SATURN_USER=\"\$SATURN_USER\"" "$REPO_ROOT/provision/cloud-init/provision-saturn.sh"
grep -Fq "ensure_operator_xdma_access \"\$OPERATOR_USER\"" "$REPO_ROOT/rules/install-rules.sh"
grep -Fq "ensure_operator_xdma_access \"\$operator_user\"" "$REPO_ROOT/scripts/install-udev-rules-on-current-image.sh"
grep -Fq "usermod -a -G \"\$P2APP_SERVICE_GROUP\" \"\$CONTROL_USER\"" "$REPO_ROOT/sw_tools/p2app-control/install.sh"

printf 'provisioning contract tests passed\n'
