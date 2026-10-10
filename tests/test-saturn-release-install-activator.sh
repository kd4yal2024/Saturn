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

# --- Receive validator and guard script: pinned by their own hashes, installed before the activator, all checks before the first write.
VALIDATOR_SRC="$REPO_ROOT/update_manager/scripts/saturn-remote-validate.py"
GUARD_SRC="$REPO_ROOT/update_manager/scripts/saturn-remote-receive-guard.js"
VALIDATOR_SHA="$(sha256sum "$VALIDATOR_SRC" | cut -d' ' -f1)"
GUARD_SHA="$(sha256sum "$GUARD_SRC" | cut -d' ' -f1)"
ZERO_SHA="$(printf '0%.0s' {1..64})"

add_validator_sources(){
  cp "$VALIDATOR_SRC" "$RELEASE/scripts/saturn-remote-validate.py"
  cp "$GUARD_SRC" "$RELEASE/scripts/saturn-remote-receive-guard.js"
  chmod 0755 "$RELEASE/scripts/saturn-remote-validate.py"
  chmod 0644 "$RELEASE/scripts/saturn-remote-receive-guard.js"
  INSTALLED_VALIDATOR="$TEST_ROOT/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py"
  INSTALLED_GUARD="$TEST_ROOT/usr/local/lib/saturn-go/scripts/saturn-remote-receive-guard.js"
}
run_installer_full(){
  SATURN_RELEASE_HELPER_INSTALL_TEST_ROOT="$TEST_ROOT" \
    bash "$INSTALLER" "$COMMIT" "$EXPECTED_SHA" "$1" "$2"
}
expect_full_rejected_unchanged(){
  local label="$1"
  shift
  if run_installer_full "$@" >"$TMP_ROOT/$label.log" 2>&1; then
    printf '%s unexpectedly installed\n' "$label" >&2
    exit 1
  fi
  grep -Fxq '#!/bin/bash' "$HELPER"
  [[ ! -e "$INSTALLED_VALIDATOR" && ! -e "$INSTALLED_GUARD" && ! -e "$BACKUP" ]] || {
    printf '%s changed the host before failing\n' "$label" >&2
    exit 1
  }
}

new_fixture
add_validator_sources
run_installer_full "$VALIDATOR_SHA" "$GUARD_SHA" >"$TMP_ROOT/full-success.log"
printf '%s  %s\n' "$VALIDATOR_SHA" "$INSTALLED_VALIDATOR" | sha256sum -c - >/dev/null
printf '%s  %s\n' "$GUARD_SHA" "$INSTALLED_GUARD" | sha256sum -c - >/dev/null
printf '%s  %s\n' "$EXPECTED_SHA" "$HELPER" | sha256sum -c - >/dev/null
[[ "$(stat -c '%a' "$INSTALLED_VALIDATOR")" == 755 && "$(stat -c '%a' "$INSTALLED_GUARD")" == 644 ]]
[[ -e "$BACKUP/validator.absent" && -e "$BACKUP/guard.absent" ]]      # nothing was there to back up, and that is recorded
grep -Fxq 'ACTIVATION_ENABLED="0"' "$CONFIG"
grep -Fq "installed validator $VALIDATOR_SHA" "$TMP_ROOT/full-success.log"
grep -Fq "installed guard script $GUARD_SHA" "$TMP_ROOT/full-success.log"

new_fixture
add_validator_sources
printf '#!/usr/bin/env python3\n# previous validator\n' >"$INSTALLED_VALIDATOR"
printf '// previous guard\n' >"$INSTALLED_GUARD"
run_installer_full "$VALIDATOR_SHA" "$GUARD_SHA" >"$TMP_ROOT/full-upgrade.log"
grep -Fxq '# previous validator' "$BACKUP/validator.before"
grep -Fxq '// previous guard' "$BACKUP/guard.before"
[[ ! -e "$BACKUP/validator.absent" ]]
cmp "$VALIDATOR_SRC" "$INSTALLED_VALIDATOR"
cmp "$GUARD_SRC" "$INSTALLED_GUARD"

new_fixture
add_validator_sources
expect_full_rejected_unchanged wrong-validator-hash "$ZERO_SHA" "$GUARD_SHA"
grep -Fq 'release validator hash mismatch' "$TMP_ROOT/wrong-validator-hash.log"

new_fixture
add_validator_sources
expect_full_rejected_unchanged wrong-guard-hash "$VALIDATOR_SHA" "$ZERO_SHA"
grep -Fq 'release guard script hash mismatch' "$TMP_ROOT/wrong-guard-hash.log"

new_fixture
add_validator_sources
rm "$RELEASE/scripts/saturn-remote-validate.py"
expect_full_rejected_unchanged missing-validator "$VALIDATOR_SHA" "$GUARD_SHA"
grep -Fq 'release validator missing or unsafe' "$TMP_ROOT/missing-validator.log"

new_fixture
add_validator_sources
rm "$RELEASE/scripts/saturn-remote-receive-guard.js"
expect_full_rejected_unchanged missing-guard "$VALIDATOR_SHA" "$GUARD_SHA"
grep -Fq 'release guard script missing or unsafe' "$TMP_ROOT/missing-guard.log"

new_fixture
add_validator_sources
printf 'def broken(:\n' >"$RELEASE/scripts/saturn-remote-validate.py"
BROKEN_SHA="$(sha256sum "$RELEASE/scripts/saturn-remote-validate.py" | cut -d' ' -f1)"
expect_full_rejected_unchanged unparsable-validator "$BROKEN_SHA" "$GUARD_SHA"
grep -Fq 'release validator syntax check failed' "$TMP_ROOT/unparsable-validator.log"

new_fixture
add_validator_sources
mv "$RELEASE/scripts/saturn-remote-validate.py" "$TMP_ROOT/real-validator.py"
ln -s "$TMP_ROOT/real-validator.py" "$RELEASE/scripts/saturn-remote-validate.py"
expect_full_rejected_unchanged symlinked-validator "$VALIDATOR_SHA" "$GUARD_SHA"

new_fixture
add_validator_sources
ln -s /etc/passwd "$INSTALLED_VALIDATOR"
if run_installer_full "$VALIDATOR_SHA" "$GUARD_SHA" >"$TMP_ROOT/symlinked-installed.log" 2>&1; then
  printf 'a symbolic link at the installed validator unexpectedly passed\n' >&2
  exit 1
fi
grep -Fq 'installed validator component is unsafe' "$TMP_ROOT/symlinked-installed.log"
grep -Fxq '#!/bin/bash' "$HELPER"
[[ ! -e "$BACKUP" ]]

new_fixture
add_validator_sources
if SATURN_RELEASE_HELPER_INSTALL_TEST_ROOT="$TEST_ROOT" bash "$INSTALLER" "$COMMIT" "$EXPECTED_SHA" "$VALIDATOR_SHA" \
    >"$TMP_ROOT/three-arguments.log" 2>&1; then
  printf 'three arguments unexpectedly passed\n' >&2
  exit 1
fi
grep -Fq 'usage:' "$TMP_ROOT/three-arguments.log"
grep -Fxq '#!/bin/bash' "$HELPER"

# the two-argument form is unchanged: it installs the activator and touches nothing else
new_fixture
add_validator_sources
printf '#!/usr/bin/env python3\n# previous validator\n' >"$INSTALLED_VALIDATOR"
run_installer "$EXPECTED_SHA" >"$TMP_ROOT/two-argument.log"
grep -Fxq '# previous validator' "$INSTALLED_VALIDATOR"
[[ ! -e "$INSTALLED_GUARD" && ! -e "$BACKUP/validator.before" && ! -e "$BACKUP/validator.absent" ]]

# a failed backup of the previous validator stops everything before a single file is installed
new_fixture
add_validator_sources
printf '#!/usr/bin/env python3\n# previous validator\n' >"$INSTALLED_VALIDATOR"
mkdir "$TEST_ROOT/bin"
printf '#!/bin/bash\ncase " $* " in *validator.before*) exit 9;; esac\nexec "%s" "$@"\n' "$REAL_CP" >"$TEST_ROOT/bin/cp"
chmod 0755 "$TEST_ROOT/bin/cp"
if PATH="$TEST_ROOT/bin:$PATH" run_installer_full "$VALIDATOR_SHA" "$GUARD_SHA" >"$TMP_ROOT/failed-validator-backup.log" 2>&1; then
  printf 'a failed validator backup unexpectedly installed\n' >&2
  exit 1
fi
grep -Fxq '#!/bin/bash' "$HELPER"
grep -Fxq '# previous validator' "$INSTALLED_VALIDATOR"
[[ ! -e "$INSTALLED_GUARD" ]]

# a malformed hash is refused as such, before anything else is looked at
new_fixture
add_validator_sources
expect_full_rejected_unchanged malformed-validator-hash "NOTAHASH" "$GUARD_SHA"
grep -Fq 'expected one lowercase SHA-256 for the validator' "$TMP_ROOT/malformed-validator-hash.log"
new_fixture
add_validator_sources
expect_full_rejected_unchanged malformed-guard-hash "$VALIDATOR_SHA" "ABC123"
grep -Fq 'expected one lowercase SHA-256 for the guard script' "$TMP_ROOT/malformed-guard-hash.log"

# an install that does not produce the pinned bytes is caught after the fact, with the way back named
new_fixture
add_validator_sources
mkdir "$TEST_ROOT/bin"
REAL_INSTALL="$(command -v install)"
# shellcheck disable=SC2016  # the single quotes are deliberate: the wrapper script is written out verbatim
printf '#!/bin/bash\n"%s" "$@"\ncase " $* " in *saturn-remote-validate.py*) printf "# corrupted\\n" >> "${@: -1}";; esac\n' "$REAL_INSTALL" >"$TEST_ROOT/bin/install"
chmod 0755 "$TEST_ROOT/bin/install"
if PATH="$TEST_ROOT/bin:$PATH" run_installer_full "$VALIDATOR_SHA" "$GUARD_SHA" >"$TMP_ROOT/corrupted-install.log" 2>&1; then
  printf 'a corrupted validator install unexpectedly passed\n' >&2
  exit 1
fi
grep -Fq 'installed validator hash mismatch; restore' "$TMP_ROOT/corrupted-install.log"
grep -Fxq '#!/bin/bash' "$HELPER"      # the activator was not replaced after the validator failed to verify

grep -Fq 'saturn-release-install-activator-root.sh' \
  "$REPO_ROOT/update_manager/release/rxc1/README.md"
printf 'Saturn release activator installation guards passed\n'
