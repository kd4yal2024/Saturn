#!/usr/bin/env bash
# shellcheck disable=SC2016 # Fixed source literals are intentionally not expanded.
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
INVENTORY="$REPO_ROOT/update_manager/release/state-inventory-v1.json"
DOCUMENTATION="$REPO_ROOT/update_manager/docs/STATE_INVENTORY.md"
STATE_CONTRACT="$REPO_ROOT/update_manager/release/components-v1.json"

python3 - "$INVENTORY" "$DOCUMENTATION" "$STATE_CONTRACT" <<'PY'
import json
import pathlib
import sys

inventory_path = pathlib.Path(sys.argv[1])
documentation_path = pathlib.Path(sys.argv[2])
contract_path = pathlib.Path(sys.argv[3])

inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
documentation = documentation_path.read_text(encoding="utf-8")
contract = json.loads(contract_path.read_text(encoding="utf-8"))

assert inventory["format"] == "saturn-state-inventory"
assert inventory["schema_version"] == 1
entries = inventory["entries"]
assert len(entries) >= 20

ids = [entry["id"] for entry in entries]
assert len(ids) == len(set(ids)), "inventory entry IDs must be unique"
by_id = {entry["id"]: entry for entry in entries}

required_ids = {
    "saturn-admin-auth",
    "linux-account-auth",
    "remembered-device-secret",
    "remote-tls-identity",
    "remote-radio-settings",
    "remote-radio-profiles",
    "pihpsdr-radio-properties",
    "deskhpsdr-radio-properties",
    "custom-script-registry",
    "custom-script-content",
    "saturn-go-state-schema",
    "provisioned-hardware-profile",
    "boot-firmware-configuration",
    "networkmanager-connections",
    "tailscale-node-state",
    "host-machine-identity",
    "operator-access-credentials",
    "deployment-transaction-history",
    "installed-application-releases",
    "fpga-artifacts-and-hardware-state",
}
missing = sorted(required_ids - set(by_id))
assert not missing, f"missing required state classes: {missing}"

portability_values = {
    "portable",
    "review-before-transfer",
    "same-device-only",
    "device-specific",
    "regenerable",
    "external",
    "diagnostic",
}
sensitivity_values = {"public", "sensitive", "secret"}
recovery_values = {"critical", "important", "optional", "none"}
backup_values = {
    "portable-settings",
    "portable-settings-selected-files",
    "same-device-disaster-recovery-only",
    "source-or-release-backup",
    "whole-disk-only",
    "exclude",
}
support_values = {"include", "metadata-only", "sanitized", "exclude"}

for entry in entries:
    assert entry["category"].strip()
    assert entry["paths"] and all(isinstance(path, str) and path for path in entry["paths"])
    assert entry["portability"] in portability_values
    assert entry["sensitivity"] in sensitivity_values
    assert entry["recovery_priority"] in recovery_values
    assert entry["recommended_backup_scope"] in backup_values
    assert entry["support_bundle"] in support_values
    assert entry["owner"].strip()
    assert entry["description"].strip()
    assert entry["restore_notes"].strip()
    if entry["sensitivity"] == "secret":
        assert entry["support_bundle"] in {"exclude", "metadata-only"}, (
            f'secret entry {entry["id"]} exposes content to support bundles'
        )
    if entry["portability"] in {"same-device-only", "device-specific"}:
        assert entry["recommended_backup_scope"] not in {
            "portable-settings",
            "portable-settings-selected-files",
        }, f'device-specific entry {entry["id"]} marked portable'

# Every direct state file managed by the REM-0205 migration contract must be
# classified by REM-0301, even if it belongs to update policy instead of user
# radio settings.
managed = set(contract["state_compatibility"]["managed_paths"])
inventoried_state_files = {
    pathlib.PurePosixPath(path).name
    for entry in entries
    for path in entry["paths"]
    if path.startswith("/var/lib/saturn-state/") and not path.endswith("/")
}
assert managed <= inventoried_state_files, (
    f"REM-0205 managed files absent from inventory: {sorted(managed - inventoried_state_files)}"
)

# Current backup names must be described precisely so an operator is not led
# to treat a source-tree archive as complete appliance recovery.
for required_text in (
    "GET /backup_full",
    "saturn-backup-*",
    "pihpsdr-backup-*",
    "deskhpsdr-backup-*",
    "REM-0205 pre-migration backup",
    "Manual whole-disk image",
    "It is not a full appliance backup.",
    "repository archive",
):
    assert required_text in documentation, f"backup documentation missing: {required_text}"

for required_exclusion in (
    "initial-login.txt",
    "remembered-device secret",
    "TLS private key",
    "Tailscale state",
    "custom-script content",
):
    assert required_exclusion in documentation, (
        f"support-bundle exclusions missing: {required_exclusion}"
    )
PY

# Keep critical code paths tied to the reviewed inventory. If ownership moves,
# this test forces the inventory and documentation to move with it.
grep -Fq 'const REMOTE_AUTH_COOKIE_SECRET_FILE: &str = "remote-tls/cookie.secret";' \
  "$REPO_ROOT/update_manager/rust-server/src/remote_tls.rs"
grep -Fq 'INITIAL_LOGIN_FILE="${SATURN_ADMIN_INITIAL_LOGIN_FILE:-/var/lib/saturn-state/initial-login.txt}"' \
  "$REPO_ROOT/scripts/saturn-admin-password.sh"
grep -Fq 'HTPASSWD_FILE="${SATURN_ADMIN_HTPASSWD_FILE:-/etc/nginx/.htpasswd}"' \
  "$REPO_ROOT/scripts/saturn-admin-password.sh"
grep -Fq 'find /var/lib/tailscale' "$REPO_ROOT/scripts/seal-saturn-image.sh"
grep -Fq '/etc/NetworkManager/system-connections' "$REPO_ROOT/scripts/seal-saturn-image.sh"
grep -Fq 'SATURN_PROFILE_ENV_FILE="${SATURN_PROFILE_ENV_FILE:-${SATURN_STATE_DIR}/profile.env}"' \
  "$REPO_ROOT/provision/cloud-init/provision-saturn.sh"

printf 'Saturn persistent-state inventory tests passed\n'
