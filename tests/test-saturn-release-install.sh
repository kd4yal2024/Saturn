#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
INSTALLER="$REPO_ROOT/update_manager/scripts/saturn-release-install-root.sh"
MANIFEST_TOOL="$REPO_ROOT/update_manager/scripts/saturn-release-manifest.py"
COMPONENTS="$REPO_ROOT/update_manager/release/components-v1.json"
TMP_ROOT="$(mktemp -d)"
STAGING_ROOT="$TMP_ROOT/staging"
RELEASES_ROOT="$TMP_ROOT/opt/saturn/releases"
CONFIG_FILE="$TMP_ROOT/release-install.conf"
COMMIT="0123456789abcdef0123456789abcdef01234567"
SECOND_COMMIT="89abcdef0123456789abcdef0123456789abcdef"
RUN_USER="$(id -un)"
RUN_GROUP="$(id -gn)"

cleanup(){ rm -rf -- "$TMP_ROOT"; }
trap cleanup EXIT

create_fixture_release(){
  local commit="$1" release_root
  release_root="$STAGING_ROOT/$commit"
  local -a args=(
    create
    --release-root "$release_root"
    --repo-root "$REPO_ROOT"
    --components "$COMPONENTS"
    --commit "$commit"
    --repository fixture://saturn
    --created-at 2026-07-18T12:00:00Z
  )

  mkdir -p "$release_root/share/release"
  cp "$COMPONENTS" "$release_root/share/release/components-v1.json"
  chmod 0644 "$release_root/share/release/components-v1.json"
  while IFS=$'\t' read -r relative executable; do
    mkdir -p "$release_root/$(dirname "$relative")"
    printf 'fixture for %s\n' "$relative" >"$release_root/$relative"
    if [[ "$executable" == "true" ]]; then
      chmod 0755 "$release_root/$relative"
    else
      chmod 0644 "$release_root/$relative"
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
  find "$release_root" -type d -exec chmod 0755 {} +
  python3 "$MANIFEST_TOOL" "${args[@]}" >/dev/null
}

expect_rejected(){
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    printf 'release installer accepted invalid case: %s\n' "$description" >&2
    exit 1
  fi
}

refresh_checksum_index(){
  local release_root="$1" temporary
  temporary="$(mktemp "$TMP_ROOT/checksums.XXXXXX")"
  (
    cd "$release_root"
    find . -type f ! -name SHA256SUMS -printf '%P\0' \
      | sort -z \
      | xargs -0 sha256sum
  ) >"$temporary"
  mv "$temporary" "$release_root/SHA256SUMS"
  chmod 0644 "$release_root/SHA256SUMS"
}

mkdir -p "$STAGING_ROOT"
cat >"$CONFIG_FILE" <<EOF
RUN_USER="$RUN_USER"
STAGING_ROOT="$STAGING_ROOT"
RELEASES_ROOT="$RELEASES_ROOT"
MANIFEST_TOOL="$MANIFEST_TOOL"
COMPONENTS_FILE="$COMPONENTS"
INSTALL_OWNER="$RUN_USER"
INSTALL_GROUP="$RUN_GROUP"
EOF
chmod 0644 "$CONFIG_FILE"
export SATURN_RELEASE_INSTALL_CONFIG="$CONFIG_FILE"

create_fixture_release "$COMMIT"
"$INSTALLER" --validate "$STAGING_ROOT/$COMMIT" >/dev/null
"$INSTALLER" "$STAGING_ROOT/$COMMIT" >/dev/null

INSTALLED="$RELEASES_ROOT/$COMMIT"
[[ -d "$INSTALLED" && ! -L "$INSTALLED" ]]
[[ ! -e "$TMP_ROOT/opt/saturn/current" && ! -L "$TMP_ROOT/opt/saturn/current" ]]
python3 "$MANIFEST_TOOL" validate \
  --release-root "$INSTALLED" \
  --components "$COMPONENTS" >/dev/null
[[ -z "$(find "$INSTALLED" -perm /022 -print -quit)" ]]
[[ "$(stat -c '%U:%G' "$INSTALLED")" == "$RUN_USER:$RUN_GROUP" ]]

# Reinstalling an identical valid release is idempotent.
output="$("$INSTALLER" "$STAGING_ROOT/$COMMIT")"
grep -Fq 'release is already installed and valid' <<<"$output"

# A malformed source cannot damage the already-installed release.
printf 'tampered\n' >>"$STAGING_ROOT/$COMMIT/bin/p2app"
expect_rejected "tampered source bundle" "$INSTALLER" --validate "$STAGING_ROOT/$COMMIT"
python3 "$MANIFEST_TOOL" validate \
  --release-root "$INSTALLED" \
  --components "$COMPONENTS" >/dev/null
printf 'fixture for bin/p2app\n' >"$STAGING_ROOT/$COMMIT/bin/p2app"
chmod 0755 "$STAGING_ROOT/$COMMIT/bin/p2app"

# Unsafe staging layouts and modes fail before an install directory is made.
create_fixture_release "$SECOND_COMMIT"
SECOND_RELEASE="$STAGING_ROOT/$SECOND_COMMIT"
ln -s /etc/passwd "$SECOND_RELEASE/unsafe-link"
expect_rejected "symbolic link in source bundle" "$INSTALLER" --validate "$SECOND_RELEASE"
rm "$SECOND_RELEASE/unsafe-link"
chmod 0775 "$SECOND_RELEASE"
expect_rejected "group-writable source bundle" "$INSTALLER" --validate "$SECOND_RELEASE"
chmod 0755 "$SECOND_RELEASE"
chmod 0750 "$SECOND_RELEASE/webroot"
expect_rejected "non-traversable release directory" "$INSTALLER" --validate "$SECOND_RELEASE"
chmod 0755 "$SECOND_RELEASE/webroot"
mkdir "$STAGING_ROOT/nested"
cp -a "$SECOND_RELEASE" "$STAGING_ROOT/nested/$SECOND_COMMIT"
expect_rejected "bundle below a nested staging directory" \
  "$INSTALLER" --validate "$STAGING_ROOT/nested/$SECOND_COMMIT"

# A self-consistent bundle for a different architecture is still rejected.
python3 - "$SECOND_RELEASE/release-manifest.json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as handle:
    manifest = json.load(handle)
manifest["build"]["architecture"] = "definitely-not-this-host"
with open(path, "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
refresh_checksum_index "$SECOND_RELEASE"
expect_rejected "release for a different architecture" "$INSTALLER" --validate "$SECOND_RELEASE"

# An invalid existing destination is never silently overwritten.
printf 'installed destination tamper\n' >>"$INSTALLED/bin/p2app"
expect_rejected "tampered existing installed release" "$INSTALLER" "$STAGING_ROOT/$COMMIT"
grep -Fq 'installed destination tamper' "$INSTALLED/bin/p2app"

# This is intentionally a literal installer assignment.
# shellcheck disable=SC2016
grep -Fq 'SATURN_RELEASES_ROOT="${SATURN_RELEASES_ROOT:-/opt/saturn/releases}"' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
grep -Fq 'INSTALL_OWNER="root"' "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
grep -Fq 'INSTALL_GROUP="root"' "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
if grep -Eq 'NOPASSWD:.*(saturn-release-install-root|SATURN_RELEASE_INSTALL)' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"; then
  printf 'release installer unexpectedly exposed through a repository sudoers file\n' >&2
  exit 1
fi
# These are intentionally literal installer expressions.
# shellcheck disable=SC2016
grep -Fq 'find "$SATURN_STATE_DIR" -path "$SATURN_RELEASE_STAGING_DIR" -prune -o -type d' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
# shellcheck disable=SC2016
grep -Fq 'find "$SATURN_STATE_DIR" -path "$SATURN_RELEASE_STAGING_DIR" -prune -o -type f' \
  "$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"

if find "$INSTALLED" -type f -name saturn-go ! -perm 0755 -print -quit | grep -q .; then
  printf 'installed executable mode was not preserved\n' >&2
  exit 1
fi

printf 'Saturn immutable release install tests passed\n'
