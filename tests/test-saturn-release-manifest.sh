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

mkdir -p "$RELEASE_ROOT/share/release"
cp "$COMPONENTS" "$RELEASE_ROOT/share/release/components-v1.json"

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
  --created-at 2026-07-18T12:00:00Z \
  --build-result fixture-tests >/dev/null 2>&1; then
  printf 'incomplete build-gate set unexpectedly created a release manifest\n' >&2
  exit 1
fi

create_fixture_manifest >/dev/null

python3 "$TOOL" validate \
  --release-root "$RELEASE_ROOT" \
  --components "$COMPONENTS" >/dev/null

python3 - "$RELEASE_ROOT/release-manifest.json" "$COMMIT" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    manifest = json.load(handle)
assert manifest["format"] == "saturn-application-release"
assert manifest["schema_version"] == 1
assert manifest["source"]["commit"] == sys.argv[2]
assert manifest["source"]["dirty"] is False
assert manifest["components"]
assert all(item["source_commit"] == sys.argv[2] for item in manifest["components"])
assert all(item["sha256"] and item["version"] for item in manifest["components"])
assert all(item["status"] == "passed" for item in manifest["build"]["results"])
PY

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
