#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
TOOL="$REPO_ROOT/update_manager/scripts/saturn-release-manifest.py"
COMPONENTS="$REPO_ROOT/update_manager/release/components-v1.json"
TMP_ROOT="$(mktemp -d)"
RELEASE_ROOT="$TMP_ROOT/release"
CURRENT_LINK="$TMP_ROOT/current"
COMMIT="0123456789abcdef0123456789abcdef01234567"

cleanup(){ rm -rf -- "$TMP_ROOT"; }
trap cleanup EXIT

create_fixture_manifest(){
  local -a args=(
    create
    --release-root "$RELEASE_ROOT"
    --repo-root "$REPO_ROOT"
    --components "$COMPONENTS"
    --commit "$COMMIT"
    --repository fixture://saturn
    --requested-ref main
    --resolved-ref refs/heads/main
    --created-at 2026-07-18T12:00:00Z
  )
  while IFS= read -r result; do
    args+=(--build-result "$result")
  done < <(python3 - "$COMPONENTS" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    descriptor = json.load(handle)
for result in descriptor["required_build_results"]:
    print(result)
PY
  )
  python3 "$TOOL" "${args[@]}"
}

refresh_checksum_index(){
  local temporary
  temporary="$(mktemp "$TMP_ROOT/checksums.XXXXXX")"
  (
    cd "$RELEASE_ROOT"
    find . -type f ! -name SHA256SUMS -printf '%P\0' \
      | sort -z \
      | xargs -0 sha256sum
  ) >"$temporary"
  mv "$temporary" "$RELEASE_ROOT/SHA256SUMS"
  chmod 0644 "$RELEASE_ROOT/SHA256SUMS"
}

mkdir -p "$RELEASE_ROOT/share/release"
install -m 0644 "$COMPONENTS" "$RELEASE_ROOT/share/release/components-v1.json"

while IFS=$'\t' read -r relative executable; do
  mkdir -p "$RELEASE_ROOT/$(dirname "$relative")"
  printf 'fixture for %s\n' "$relative" >"$RELEASE_ROOT/$relative"
  if [[ "$executable" == "true" ]]; then
    chmod 0755 "$RELEASE_ROOT/$relative"
  else
    chmod 0644 "$RELEASE_ROOT/$relative"
  fi
done < <(python3 - "$COMPONENTS" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    descriptor = json.load(handle)
for component in descriptor["components"]:
    print(f'{component["path"]}\t{str(bool(component.get("executable"))).lower()}')
PY
)

if python3 "$TOOL" create \
  --release-root "$RELEASE_ROOT" \
  --repo-root "$REPO_ROOT" \
  --components "$COMPONENTS" \
  --commit "$COMMIT" \
  --repository fixture://saturn \
  --requested-ref fixture \
  --resolved-ref refs/heads/fixture \
  --created-at 2026-07-18T12:00:00Z \
  --build-result fixture-tests >/dev/null 2>&1; then
  printf 'incomplete build-gate set unexpectedly created a release manifest\n' >&2
  exit 1
fi

create_fixture_manifest >/dev/null

python3 "$TOOL" validate \
  --release-root "$RELEASE_ROOT" \
  --components "$COMPONENTS" >/dev/null

chmod 0664 "$RELEASE_ROOT/webroot/saturn-remote-next.js"
if create_fixture_manifest >/dev/null 2>&1; then
  printf 'group-writable release payload unexpectedly created a manifest\n' >&2
  exit 1
fi
chmod 0644 "$RELEASE_ROOT/webroot/saturn-remote-next.js"
create_fixture_manifest >/dev/null

python3 - "$RELEASE_ROOT/release-manifest.json" "$COMMIT" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    manifest = json.load(handle)
assert manifest["format"] == "saturn-application-release"
assert manifest["schema_version"] == 3
assert manifest["source"]["commit"] == sys.argv[2]
assert manifest["source"]["repository"] == "fixture://saturn"
assert manifest["source"]["requested_ref"] == "main"
assert manifest["source"]["resolved_ref"] == "refs/heads/main"
assert manifest["source"]["dirty"] is False
assert manifest["components"]
assert all(item["source_commit"] == sys.argv[2] for item in manifest["components"])
assert all(item["sha256"] and item["version"] for item in manifest["components"])
assert all(item["status"] == "passed" for item in manifest["build"]["results"])
state = manifest["state_compatibility"]
assert state["state_schema_version"] == 1
assert state["readable_state_schema_versions"] == [0, 1]
assert state["migration"]["from_state_schema_versions"] == [0]
assert state["migration"]["one_way"] is False
PY

python3 "$TOOL" state-contract \
  --release-root "$RELEASE_ROOT" \
  --components "$COMPONENTS" \
  | python3 -c 'import json,sys; value=json.load(sys.stdin); assert value["state_schema_version"] == 1; assert value["readable_state_schema_versions"] == [0, 1]'

# New schema-v3 manifests fail closed when exact source provenance is absent.
python3 - "$RELEASE_ROOT/release-manifest.json" <<'PY'
import json
import sys
path = sys.argv[1]
with open(path, encoding="utf-8") as handle:
    value = json.load(handle)
value["source"].pop("resolved_ref")
with open(path, "w", encoding="utf-8") as handle:
    json.dump(value, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
refresh_checksum_index
if python3 "$TOOL" validate \
  --release-root "$RELEASE_ROOT" \
  --components "$COMPONENTS" >/dev/null 2>&1; then
  printf 'schema-v3 manifest without source provenance unexpectedly passed\n' >&2
  exit 1
fi
create_fixture_manifest >/dev/null

# Existing schema-v2 release manifests remain valid without the new source-ref
# provenance fields.
python3 - "$RELEASE_ROOT/release-manifest.json" <<'PY'
import json
import sys
path = sys.argv[1]
with open(path, encoding="utf-8") as handle:
    value = json.load(handle)
value["schema_version"] = 2
value["source"].pop("requested_ref")
value["source"].pop("resolved_ref")
with open(path, "w", encoding="utf-8") as handle:
    json.dump(value, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
refresh_checksum_index
python3 "$TOOL" validate --release-root "$RELEASE_ROOT" --components "$COMPONENTS" >/dev/null
create_fixture_manifest >/dev/null

# Existing schema-v1 release manifests remain valid and receive the explicit
# legacy state contract so installed rollback releases are not stranded.
python3 - "$RELEASE_ROOT/release-manifest.json" <<'PY'
import json
import sys
path = sys.argv[1]
with open(path, encoding="utf-8") as handle:
    value = json.load(handle)
value["schema_version"] = 1
value.pop("state_compatibility", None)
value["build"]["results"] = [
    item for item in value["build"]["results"]
    if item["name"] != "state-compatibility-tests"
]
with open(path, "w", encoding="utf-8") as handle:
    json.dump(value, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
refresh_checksum_index
python3 "$TOOL" validate --release-root "$RELEASE_ROOT" --components "$COMPONENTS" >/dev/null
python3 "$TOOL" state-contract --release-root "$RELEASE_ROOT" --components "$COMPONENTS" \
  | python3 -c 'import json,sys; value=json.load(sys.stdin); assert value["legacy_manifest"] is True; assert value["state_schema_version"] == 0; assert value["readable_state_schema_versions"] == [0, 1]'
create_fixture_manifest >/dev/null

# A release policy that omits immediate predecessor readability is rejected.
python3 - "$COMPONENTS" "$TMP_ROOT/incompatible-components.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
value["state_compatibility"]["readable_state_schema_versions"] = [1]
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    json.dump(value, handle)
PY
bad_args=(
  create --release-root "$RELEASE_ROOT" --repo-root "$REPO_ROOT"
  --components "$TMP_ROOT/incompatible-components.json" --commit "$COMMIT"
  --repository fixture://saturn
  --requested-ref fixture --resolved-ref refs/heads/fixture
)
while IFS= read -r result; do
  bad_args+=(--build-result "$result")
done < <(python3 - "$COMPONENTS" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
for result in value["required_build_results"]:
    print(result)
PY
)
if python3 "$TOOL" "${bad_args[@]}" >/dev/null 2>&1; then
  printf 'release policy without predecessor readability unexpectedly passed\n' >&2
  exit 1
fi

# The schema marker is owned by the migration helper and cannot be disguised
# as an ordinary managed setting in a release policy.
python3 - "$COMPONENTS" "$TMP_ROOT/marker-components.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
value["state_compatibility"]["managed_paths"].append("state-schema.json")
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    json.dump(value, handle)
PY
bad_args[6]="$TMP_ROOT/marker-components.json"
if python3 "$TOOL" "${bad_args[@]}" >/dev/null 2>&1; then
  printf 'release policy treated the state marker as an ordinary setting\n' >&2
  exit 1
fi

mkdir "$TMP_ROOT/old-release"
ln -s "$TMP_ROOT/old-release" "$CURRENT_LINK"
printf 'tampered\n' >>"$RELEASE_ROOT/bin/p2app"
if python3 "$TOOL" validate --release-root "$RELEASE_ROOT" --components "$COMPONENTS" >/dev/null 2>&1; then
  printf 'tampered release unexpectedly validated\n' >&2
  exit 1
fi
[[ "$(readlink -f "$CURRENT_LINK")" == "$TMP_ROOT/old-release" ]]

printf 'fixture for bin/p2app\n' >"$RELEASE_ROOT/bin/p2app"
chmod 0755 "$RELEASE_ROOT/bin/p2app"
create_fixture_manifest >/dev/null

ln -s bin/p2app "$RELEASE_ROOT/unsafe-link"
if python3 "$TOOL" validate --release-root "$RELEASE_ROOT" --components "$COMPONENTS" >/dev/null 2>&1; then
  printf 'release containing a symlink unexpectedly validated\n' >&2
  exit 1
fi
[[ "$(readlink -f "$CURRENT_LINK")" == "$TMP_ROOT/old-release" ]]

printf 'Saturn release manifest tests passed\n'
