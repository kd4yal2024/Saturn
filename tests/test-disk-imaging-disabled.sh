#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN_RS="$REPO_ROOT/update_manager/rust-server/src/main.rs"
BACKUP_HTML="$REPO_ROOT/update_manager/templates/backup.html"
INSTALLER="$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
RUNBOOK="$REPO_ROOT/update_manager/docs/OPERATIONS_RUNBOOK.md"

disabled_route_count="$(grep -Ec '\.route\("/(pi_image|pi_clone|pi_devices|pi_wipe_target)' "$MAIN_RS")"
[[ "$disabled_route_count" -eq 9 ]]
[[ "$(grep -c 'disk_imaging_disabled' "$MAIN_RS")" -ge 10 ]]

if grep -Eq 'mod (clone|image);|crate::(clone|image)' "$MAIN_RS"; then
  echo "disk imaging modules remain connected to Saturn Go" >&2
  exit 1
fi

if grep -Eq "id=\"(pi-|clone-)|fetch\('./(pi_image|pi_clone|pi_devices|pi_wipe_target)" "$BACKUP_HTML"; then
  echo "disk imaging controls remain in backup.html" >&2
  exit 1
fi
grep -Fq 'intentionally disabled in Saturn Go' "$BACKUP_HTML"

if grep -Eq 'NOPASSWD: .*\/(make_pi_image|clone_pi_to_device|saturn-pi-wipe-target)\.sh' "$INSTALLER"; then
  echo "disk imaging helper remains in Saturn Go sudoers policy" >&2
  exit 1
fi
# These patterns intentionally match literal shell variable references.
# shellcheck disable=SC2016
grep -Fq '"$PRIVILEGED_SCRIPTS_DIR/make_pi_image.sh"' "$INSTALLER"
# shellcheck disable=SC2016
grep -Fq '"$PRIVILEGED_SCRIPTS_DIR/clone_pi_to_device.sh"' "$INSTALLER"
# shellcheck disable=SC2016
grep -Fq '"$PRIVILEGED_SCRIPTS_DIR/saturn-pi-wipe-target.sh"' "$INSTALLER"

grep -Fq '### Manual Whole-Disk Imaging' "$RUNBOOK"
grep -Fq 'sudo ./update_manager/scripts/make_pi_image.sh' "$RUNBOOK"
grep -Fq 'sudo ./update_manager/scripts/clone_pi_to_device.sh' "$RUNBOOK"

printf 'browser disk-imaging disablement tests passed\n'
