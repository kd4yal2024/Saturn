#!/usr/bin/env bash
set -Eeuo pipefail

# Install only the approved privileged activator from a validated immutable
# release. This does not enable activation, switch releases, or touch services.
#
# With two extra hashes it also installs the receive validator and its receive-only guard script, which the activator runs as root to prove
# reception itself. Each file is pinned by the hash the operator supplies; nothing else from the release is trusted.
die(){ printf '[saturn-release-helper-install] ERROR: %s\n' "$*" >&2; exit 1; }

[[ $# -eq 2 || $# -eq 4 ]] || die "usage: $0 <full-commit> <activator-sha256> [<validator-sha256> <guard-sha256>]"
commit="$1"
expected_sha="$2"
validator_sha="${3:-}"
guard_sha="${4:-}"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "expected one lowercase full Git commit"
[[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || die "expected one lowercase SHA-256"
if [[ $# -eq 4 ]]; then
  [[ "$validator_sha" =~ ^[0-9a-f]{64}$ ]] || die "expected one lowercase SHA-256 for the validator"
  [[ "$guard_sha" =~ ^[0-9a-f]{64}$ ]] || die "expected one lowercase SHA-256 for the guard script"
fi

test_root="${SATURN_RELEASE_HELPER_INSTALL_TEST_ROOT:-}"
if [[ -n "$test_root" ]]; then
  (( EUID != 0 )) || die "test-root override is forbidden as root"
  [[ "$test_root" == /* && "$test_root" != / && -d "$test_root" && ! -L "$test_root" ]] \
    || die "unsafe test root"
  releases_root="$test_root/opt/saturn/releases"
  helper="$test_root/usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh"
  config="$test_root/etc/default/saturn-release-activate"
  backup_parent="$test_root/var/lib/saturn-state/deployments"
  manifest_tool="$test_root/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py"
  components="$test_root/usr/local/lib/saturn-go/release/components-v1.json"
  validator="$test_root/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py"
  guard="$test_root/usr/local/lib/saturn-go/scripts/saturn-remote-receive-guard.js"
else
  (( EUID == 0 )) || die "run as root for production installation"
  releases_root=/opt/saturn/releases
  helper=/usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh
  config=/etc/default/saturn-release-activate
  backup_parent=/var/lib/saturn-state/deployments
  manifest_tool=/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py
  components=/usr/local/lib/saturn-go/release/components-v1.json
  validator=/usr/local/lib/saturn-go/scripts/saturn-remote-validate.py
  guard=/usr/local/lib/saturn-go/scripts/saturn-remote-receive-guard.js
fi
release="$releases_root/$commit"
source_helper="$release/scripts/saturn-release-activate-root.sh"
source_validator="$release/scripts/saturn-remote-validate.py"
source_guard="$release/scripts/saturn-remote-receive-guard.js"
backup="$backup_parent/pre-activation-$commit"

# Every prerequisite is checked before the first write. Do not use a chain of
# `test A && test B` here: each absent input must stop installation.
[[ -d "$release" && ! -L "$release" ]] || die "release missing or unsafe: $release"
[[ -f "$source_helper" && ! -L "$source_helper" ]] || die "release activator missing or unsafe"
[[ -f "$helper" && ! -L "$helper" ]] || die "installed activator missing or unsafe"
[[ -f "$config" && ! -L "$config" ]] || die "activation config missing or unsafe"
[[ -d "$backup_parent" && ! -L "$backup_parent" ]] || die "backup parent missing or unsafe"
[[ ! -e "$backup" && ! -L "$backup" ]] || die "backup already exists: $backup"
[[ -f "$components" && ! -L "$components" ]] || die "component policy missing or unsafe"
[[ -x "$manifest_tool" && ! -L "$manifest_tool" ]] || die "manifest validator missing or unsafe"
activation_lines="$(grep -c '^ACTIVATION_ENABLED=' "$config" || true)"
[[ "$activation_lines" == 1 ]] || die "activation config must have exactly one activation setting"
grep -Fxq 'ACTIVATION_ENABLED="0"' "$config" || die "activation config is not disabled"
if [[ -z "$test_root" ]]; then
  for path in "$release" "$source_helper" "$helper" "$config" "$backup_parent" "$manifest_tool" "$components"; do
    [[ "$(stat -c '%u' "$path")" == 0 ]] || die "non-root-owned prerequisite: $path"
    mode="$(stat -c '%a' "$path")"
    (( (8#$mode & 8#022) == 0 )) || die "writable prerequisite: $path"
  done
fi
python3 "$manifest_tool" validate --release-root "$release" --components "$components" >/dev/null \
  || die "release manifest validation failed"
printf '%s  %s\n' "$expected_sha" "$source_helper" | sha256sum -c - >/dev/null \
  || die "release activator hash mismatch"
bash -n "$source_helper" || die "release activator syntax check failed"
if [[ -n "$validator_sha" ]]; then
  [[ -f "$source_validator" && ! -L "$source_validator" ]] || die "release validator missing or unsafe"
  [[ -f "$source_guard" && ! -L "$source_guard" ]] || die "release guard script missing or unsafe"
  for installed in "$validator" "$guard"; do
    [[ ! -L "$installed" && ( ! -e "$installed" || -f "$installed" ) ]] || die "installed validator component is unsafe: $installed"
  done
  if [[ -z "$test_root" ]]; then
    for installed in "$source_validator" "$source_guard" "$validator" "$guard"; do
      [[ ! -e "$installed" ]] && continue
      [[ "$(stat -c '%u' "$installed")" == 0 ]] || die "non-root-owned validator component: $installed"
      mode="$(stat -c '%a' "$installed")"
      (( (8#$mode & 8#022) == 0 )) || die "writable validator component: $installed"
    done
  fi
  printf '%s  %s\n' "$validator_sha" "$source_validator" | sha256sum -c - >/dev/null \
    || die "release validator hash mismatch"
  printf '%s  %s\n' "$guard_sha" "$source_guard" | sha256sum -c - >/dev/null \
    || die "release guard script hash mismatch"
  python3 -c 'import ast, sys; ast.parse(open(sys.argv[1], encoding="utf-8").read())' "$source_validator" \
    || die "release validator syntax check failed"
fi

# Backups must be complete and verified before replacing the installed helper.
install -d -m 0700 "$backup" || die "cannot create backup directory"
cp -a -- "$helper" "$backup/activator.before" || die "activator backup failed"
cp -a -- "$config" "$backup/config.before" || die "config backup failed"
cmp -s "$helper" "$backup/activator.before" || die "activator backup verification failed"
cmp -s "$config" "$backup/config.before" || die "config backup verification failed"
old_helper_sha="$(sha256sum "$backup/activator.before" | cut -d' ' -f1)" || die "cannot hash activator backup"
old_config_sha="$(sha256sum "$backup/config.before" | cut -d' ' -f1)" || die "cannot hash config backup"
cmp -s "$config" "$backup/config.before" || die "activation config changed before installation"
if [[ -n "$validator_sha" ]]; then
  for pair in "validator:$validator" "guard:$guard"; do
    name="${pair%%:*}"
    destination="${pair#*:}"
    if [[ -e "$destination" ]]; then
      cp -a -- "$destination" "$backup/$name.before" || die "$name backup failed"
      cmp -s "$destination" "$backup/$name.before" || die "$name backup verification failed"
    else
      : >"$backup/$name.absent" || die "cannot record that no $name was installed"
    fi
  done
fi

# The validator and its guard go in first: an activator must never be left running without them.
if [[ -n "$validator_sha" ]]; then
  if [[ -n "$test_root" ]]; then
    install -m 0755 "$source_validator" "$validator" || die "validator installation failed"
    install -m 0644 "$source_guard" "$guard" || die "guard script installation failed"
  else
    install -o root -g root -m 0755 "$source_validator" "$validator" || die "validator installation failed"
    install -o root -g root -m 0644 "$source_guard" "$guard" || die "guard script installation failed"
  fi
  printf '%s  %s\n' "$validator_sha" "$validator" | sha256sum -c - >/dev/null \
    || die "installed validator hash mismatch; restore $backup/validator.before or remove the file"
  printf '%s  %s\n' "$guard_sha" "$guard" | sha256sum -c - >/dev/null \
    || die "installed guard hash mismatch; restore $backup/guard.before or remove the file"
fi
if [[ -n "$test_root" ]]; then
  install -m 0755 "$source_helper" "$helper" || die "activator installation failed"
else
  install -o root -g root -m 0755 "$source_helper" "$helper" || die "activator installation failed"
fi
printf '%s  %s\n' "$expected_sha" "$helper" | sha256sum -c - >/dev/null \
  || die "installed activator hash mismatch; restore $backup/activator.before"
cmp -s "$config" "$backup/config.before" \
  || die "activation config changed; restore $backup/config.before"
bash -n "$helper" || die "installed activator syntax check failed"
if [[ -n "$validator_sha" ]]; then
  printf '[saturn-release-helper-install] installed validator %s\n' "$validator_sha"
  printf '[saturn-release-helper-install] installed guard script %s\n' "$guard_sha"
fi
printf '[saturn-release-helper-install] installed activator %s\n' "$expected_sha"
printf '[saturn-release-helper-install] previous activator %s\n' "$old_helper_sha"
printf '[saturn-release-helper-install] unchanged config %s\n' "$old_config_sha"
printf '[saturn-release-helper-install] verified backup %s\n' "$backup"
