#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
STATE_TOOL="$REPO_ROOT/update_manager/scripts/saturn-state-compatibility.py"
MANIFEST_TOOL="$REPO_ROOT/update_manager/scripts/saturn-release-manifest.py"
COMPONENTS="$REPO_ROOT/update_manager/release/components-v1.json"
TMP_ROOT="$(mktemp -d)"
STATE_ROOT="$TMP_ROOT/state"
BACKUP_ROOT="$TMP_ROOT/backups"
RELEASES_ROOT="$TMP_ROOT/releases"
V1_COMMIT="1111111111111111111111111111111111111111"
V2_UNSAFE_COMMIT="2222222222222222222222222222222222222222"
V2_ONE_WAY_COMMIT="3333333333333333333333333333333333333333"

cleanup(){ rm -rf -- "$TMP_ROOT"; }
trap cleanup EXIT

create_release(){
  local commit="$1" descriptor="$2" release
  release="$RELEASES_ROOT/$commit"
  local -a args=(
    create --release-root "$release" --repo-root "$REPO_ROOT"
    --components "$descriptor" --commit "$commit"
    --repository fixture://saturn
    --requested-ref fixture --resolved-ref refs/heads/fixture
    --created-at 2026-07-20T12:00:00Z
  )
  mkdir -p "$release/share/release"
  install -m 0644 "$descriptor" "$release/share/release/components-v1.json"
  while IFS=$'\t' read -r relative executable; do
    mkdir -p "$release/$(dirname "$relative")"
    printf 'fixture for %s\n' "$relative" >"$release/$relative"
    if [[ "$executable" == "true" ]]; then
      chmod 0755 "$release/$relative"
    else
      chmod 0644 "$release/$relative"
    fi
  done < <(python3 - "$descriptor" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
for component in value["components"]:
    print(f'{component["path"]}\t{str(bool(component.get("executable"))).lower()}')
PY
  )
  while IFS= read -r result; do
    args+=(--build-result "$result")
  done < <(python3 - "$descriptor" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
for result in value["required_build_results"]:
    print(result)
PY
  )
  find "$release" -type d -exec chmod 0755 {} +
  python3 "$MANIFEST_TOOL" "${args[@]}" >/dev/null
}

mkdir -p "$STATE_ROOT" "$RELEASES_ROOT"
printf '[{"filename":"operator.sh","flags":[]}]\n' >"$STATE_ROOT/custom_scripts.json"
printf '{"activeProfile":"primary","theme":"dark"}\n' >"$STATE_ROOT/remote_settings.json"
chmod 0640 "$STATE_ROOT/custom_scripts.json" "$STATE_ROOT/remote_settings.json"
create_release "$V1_COMMIT" "$COMPONENTS"

plan="$("$STATE_TOOL" preflight \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V1_COMMIT")"
python3 - "$plan" <<'PY'
import json
import sys
value = json.loads(sys.argv[1])
assert value["current_state_schema_version"] == 0
assert value["target_state_schema_version"] == 1
assert value["migration_required"] is True
assert value["rollback_safe"] is True
assert value["one_way_approved"] is False
PY
if "$STATE_TOOL" preflight \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V1_COMMIT" \
  --approve-one-way >/dev/null 2>&1; then
  printf 'unnecessary one-way migration approval unexpectedly passed\n' >&2
  exit 1
fi

result="$("$STATE_TOOL" migrate \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V1_COMMIT" \
  --backup-root "$BACKUP_ROOT" \
  --target-commit "$V1_COMMIT" \
  --state-group "$(id -gn)")"
backup="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["backup_directory"])' <<<"$result")"
[[ -d "$backup" && -f "$backup/backup-manifest.json" ]]
python3 - "$STATE_ROOT/state-schema.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
assert value["format"] == "saturn-persistent-state"
assert value["state_schema_version"] == 1
assert value["migrated_from_state_schema_version"] == 0
PY
grep -Fq 'operator.sh' "$STATE_ROOT/custom_scripts.json"
grep -Fq '"dark"' "$STATE_ROOT/remote_settings.json"

# Simulate writes by the target release, then prove immediate rollback restores
# the complete pre-migration settings and the legacy no-marker state.
printf 'changed\n' >"$STATE_ROOT/custom_scripts.json"
printf '{"profiles":{}}\n' >"$STATE_ROOT/remote_profiles.json"
"$STATE_TOOL" restore \
  --state-root "$STATE_ROOT" \
  --backup-root "$BACKUP_ROOT" \
  --backup-directory "$backup" >/dev/null
grep -Fq 'operator.sh' "$STATE_ROOT/custom_scripts.json"
[[ ! -e "$STATE_ROOT/remote_profiles.json" ]]
[[ ! -e "$STATE_ROOT/state-schema.json" ]]

# Backup checksums are enforced before any destination is replaced.
settings_payload="$(python3 - "$backup/backup-manifest.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
print(value["entries"]["remote_settings.json"]["backup_file"])
PY
)"
printf 'corrupt\n' >>"$backup/$settings_payload"
cp "$STATE_ROOT/remote_settings.json" "$TMP_ROOT/settings-before-corrupt-restore"
if "$STATE_TOOL" restore \
  --state-root "$STATE_ROOT" \
  --backup-root "$BACKUP_ROOT" \
  --backup-directory "$backup" >/dev/null 2>&1; then
  printf 'state restore accepted a corrupt backup payload\n' >&2
  exit 1
fi
cmp "$TMP_ROOT/settings-before-corrupt-restore" "$STATE_ROOT/remote_settings.json"

# Unsafe managed entries fail migration without leaving a schema marker or an
# incomplete backup directory behind.
ln -s /etc/passwd "$STATE_ROOT/remote_profiles.json"
if "$STATE_TOOL" migrate \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V1_COMMIT" \
  --backup-root "$BACKUP_ROOT" \
  --target-commit "$V1_COMMIT" \
  --state-group "$(id -gn)" >/dev/null 2>&1; then
  printf 'state migration accepted a symlinked managed file\n' >&2
  exit 1
fi
rm "$STATE_ROOT/remote_profiles.json"
[[ ! -e "$STATE_ROOT/state-schema.json" ]]
[[ "$(find "$BACKUP_ROOT" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 1 ]]

# Advance to schema 1 again to exercise future rollback-safety policy.
result="$("$STATE_TOOL" migrate \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V1_COMMIT" \
  --backup-root "$BACKUP_ROOT" \
  --target-commit "$V1_COMMIT" \
  --state-group "$(id -gn)")"
python3 -c 'import json,sys; assert json.load(sys.stdin)["migrated"] is True' <<<"$result"

python3 - "$COMPONENTS" "$TMP_ROOT/v2-unsafe.json" "$TMP_ROOT/v2-one-way.json" <<'PY'
import copy
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    base = json.load(handle)
unsafe = copy.deepcopy(base)
contract = unsafe["state_compatibility"]
contract["state_schema_version"] = 2
contract["readable_state_schema_versions"] = [1, 2]
contract["migration"]["from_state_schema_versions"] = [1]
contract["migration"]["documentation"] = "Fixture schema-1 to schema-2 migration."
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    json.dump(unsafe, handle)
one_way = copy.deepcopy(unsafe)
one_way["state_compatibility"]["migration"]["one_way"] = True
one_way["state_compatibility"]["migration"]["documentation"] = (
    "Fixture one-way schema-2 migration; legacy releases cannot read schema 2."
)
with open(sys.argv[3], "w", encoding="utf-8") as handle:
    json.dump(one_way, handle)
PY
create_release "$V2_UNSAFE_COMMIT" "$TMP_ROOT/v2-unsafe.json"
create_release "$V2_ONE_WAY_COMMIT" "$TMP_ROOT/v2-one-way.json"

if "$STATE_TOOL" preflight \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V2_UNSAFE_COMMIT" >/dev/null 2>&1; then
  printf 'undocumented rollback-unsafe migration unexpectedly passed\n' >&2
  exit 1
fi
if "$STATE_TOOL" preflight \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V2_ONE_WAY_COMMIT" >/dev/null 2>&1; then
  printf 'one-way migration without operator approval unexpectedly passed\n' >&2
  exit 1
fi
approved="$("$STATE_TOOL" preflight \
  --state-root "$STATE_ROOT" \
  --target-release "$RELEASES_ROOT/$V2_ONE_WAY_COMMIT" \
  --approve-one-way)"
python3 -c 'import json,sys; value=json.load(sys.stdin); assert value["rollback_safe"] is False; assert value["one_way_approved"] is True' <<<"$approved"

# Restore paths are constrained to real direct children of the configured
# backup root; a symlink cannot redirect root restore writes elsewhere.
ln -s "$backup" "$TMP_ROOT/backup-link"
if "$STATE_TOOL" restore \
  --state-root "$STATE_ROOT" \
  --backup-root "$BACKUP_ROOT" \
  --backup-directory "$TMP_ROOT/backup-link" >/dev/null 2>&1; then
  printf 'state restore accepted a symlinked backup directory\n' >&2
  exit 1
fi

printf 'Saturn persistent-state compatibility tests passed\n'
