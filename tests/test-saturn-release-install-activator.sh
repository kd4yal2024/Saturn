#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
INSTALLER="$REPO_ROOT/update_manager/scripts/saturn-release-install-activator-root.sh"
ACTIVATOR="$REPO_ROOT/update_manager/scripts/saturn-release-activate-root.sh"
TMP_ROOT="$(mktemp -d)"
COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
EXPECTED_SHA="$(sha256sum "$ACTIVATOR" | cut -d' ' -f1)"
REAL_CP="$(command -v cp)"
trap 'rm -rf -- "$TMP_ROOT"' EXIT

new_fixture(){
  TEST_ROOT="$(mktemp -d "$TMP_ROOT/case.XXXXXX")"
  RELEASE="$TEST_ROOT/opt/saturn/releases/$COMMIT"
  SOURCE_HELPER="$RELEASE/scripts/saturn-release-activate-root.sh"
  HELPER="$TEST_ROOT/usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh"
  CONFIG="$TEST_ROOT/etc/default/saturn-release-activate"
  BACKUP="$TEST_ROOT/var/lib/saturn-state/deployments/pre-activation-$COMMIT"
  MANIFEST_TOOL="$TEST_ROOT/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py"
  mkdir -p "$(dirname "$SOURCE_HELPER")" "$(dirname "$HELPER")" \
    "$(dirname "$CONFIG")" "$(dirname "$BACKUP")" \
    "$TEST_ROOT/usr/local/lib/saturn-go/release"
  cp "$ACTIVATOR" "$SOURCE_HELPER"
  printf '#!/bin/bash\necho previous\n' >"$HELPER"
  chmod 0755 "$SOURCE_HELPER" "$HELPER"
  printf 'ACTIVATION_ENABLED="0"\n' >"$CONFIG"
  printf '{}\n' >"$TEST_ROOT/usr/local/lib/saturn-go/release/components-v1.json"
  printf '#!/usr/bin/env python3\nimport sys\nsys.exit(0 if sys.argv[1] == "validate" else 1)\n' \
    >"$MANIFEST_TOOL"
  chmod 0755 "$MANIFEST_TOOL"
}

run_installer(){
  SATURN_RELEASE_HELPER_INSTALL_TEST_ROOT="$TEST_ROOT" \
    bash "$INSTALLER" "$COMMIT" "$1"
}

expect_rejected_unchanged(){
  local label="$1" sha="$2" output
  output="$TMP_ROOT/$label.log"
  if run_installer "$sha" >"$output" 2>&1; then
    printf '%s unexpectedly installed the helper\n' "$label" >&2
    exit 1
  fi
  grep -Fxq '#!/bin/bash' "$HELPER"
  grep -Fxq 'ACTIVATION_ENABLED="0"' "$CONFIG" || {
    [[ "$label" == enabled-config ]]
    grep -Fxq 'ACTIVATION_ENABLED="1"' "$CONFIG"
  }
}

new_fixture
expect_rejected_unchanged wrong-source-hash "$(printf '0%.0s' {1..64})"
[[ ! -e "$BACKUP" ]]

new_fixture
printf 'ACTIVATION_ENABLED="1"\n' >"$CONFIG"
expect_rejected_unchanged enabled-config "$EXPECTED_SHA"
[[ ! -e "$BACKUP" ]]

new_fixture
printf 'ACTIVATION_ENABLED="1"\n' >>"$CONFIG"
if run_installer "$EXPECTED_SHA" >"$TMP_ROOT/duplicate-enabled-config.log" 2>&1; then
  printf 'duplicate enabled config unexpectedly passed\n' >&2
  exit 1
fi
grep -Fxq '#!/bin/bash' "$HELPER"
[[ ! -e "$BACKUP" ]]

new_fixture
mkdir "$BACKUP"
expect_rejected_unchanged existing-backup "$EXPECTED_SHA"

new_fixture
rm "$HELPER"
if run_installer "$EXPECTED_SHA" >"$TMP_ROOT/missing-helper.log" 2>&1; then
  printf 'missing installed helper unexpectedly passed\n' >&2
  exit 1
fi
[[ ! -e "$BACKUP" && ! -e "$HELPER" ]]

new_fixture
rm "$CONFIG"
if run_installer "$EXPECTED_SHA" >"$TMP_ROOT/missing-config.log" 2>&1; then
  printf 'missing config unexpectedly passed\n' >&2
  exit 1
fi
[[ ! -e "$BACKUP" && ! -e "$CONFIG" ]]

new_fixture
printf '#!/usr/bin/env python3\nraise SystemExit(1)\n' >"$MANIFEST_TOOL"
expect_rejected_unchanged failed-manifest "$EXPECTED_SHA"
[[ ! -e "$BACKUP" ]]

new_fixture
mkdir "$TEST_ROOT/bin"
printf '#!/bin/bash\ncase " $* " in *config.before*) exit 9;; esac\nexec "%s" "$@"\n' \
  "$REAL_CP" >"$TEST_ROOT/bin/cp"
chmod 0755 "$TEST_ROOT/bin/cp"
if PATH="$TEST_ROOT/bin:$PATH" run_installer "$EXPECTED_SHA" \
    >"$TMP_ROOT/failed-backup.log" 2>&1; then
  printf 'failed backup copy unexpectedly installed the helper\n' >&2
  exit 1
fi
grep -Fxq '#!/bin/bash' "$HELPER"
grep -Fxq 'ACTIVATION_ENABLED="0"' "$CONFIG"
[[ -e "$BACKUP/activator.before" && ! -e "$BACKUP/config.before" ]] || {
  sed -n '1,80p' "$TMP_ROOT/failed-backup.log" >&2
  exit 1
}

new_fixture
run_installer "$EXPECTED_SHA" >"$TMP_ROOT/success.log"
printf '%s  %s\n' "$EXPECTED_SHA" "$HELPER" | sha256sum -c - >/dev/null
grep -Fxq 'ACTIVATION_ENABLED="0"' "$CONFIG"
grep -Fxq '#!/bin/bash' "$BACKUP/activator.before"
cmp "$CONFIG" "$BACKUP/config.before"
grep -Fq 'verified backup' "$TMP_ROOT/success.log"

grep -Fq 'saturn-release-install-activator-root.sh' \
  "$REPO_ROOT/update_manager/release/rxc1/README.md"
printf 'Saturn release activator installation guards passed\n'
