#!/usr/bin/env bash
set -Eeuo pipefail

# Install only the approved privileged activator from a validated immutable
# release. This does not enable activation, switch releases, or touch services.
die(){ printf '[saturn-release-helper-install] ERROR: %s\n' "$*" >&2; exit 1; }

[[ $# -eq 2 ]] || die "usage: $0 <full-commit> <activator-sha256>"
commit="$1"
expected_sha="$2"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "expected one lowercase full Git commit"
[[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || die "expected one lowercase SHA-256"

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
else
  (( EUID == 0 )) || die "run as root for production installation"
  releases_root=/opt/saturn/releases
  helper=/usr/local/lib/saturn-go/scripts/saturn-release-activate-root.sh
  config=/etc/default/saturn-release-activate
  backup_parent=/var/lib/saturn-state/deployments
  manifest_tool=/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py
  components=/usr/local/lib/saturn-go/release/components-v1.json
fi
release="$releases_root/$commit"
source_helper="$release/scripts/saturn-release-activate-root.sh"
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

# Backups must be complete and verified before replacing the installed helper.
install -d -m 0700 "$backup" || die "cannot create backup directory"
cp -a -- "$helper" "$backup/activator.before" || die "activator backup failed"
cp -a -- "$config" "$backup/config.before" || die "config backup failed"
cmp -s "$helper" "$backup/activator.before" || die "activator backup verification failed"
cmp -s "$config" "$backup/config.before" || die "config backup verification failed"
old_helper_sha="$(sha256sum "$backup/activator.before" | cut -d' ' -f1)" || die "cannot hash activator backup"
old_config_sha="$(sha256sum "$backup/config.before" | cut -d' ' -f1)" || die "cannot hash config backup"
cmp -s "$config" "$backup/config.before" || die "activation config changed before installation"

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
printf '[saturn-release-helper-install] installed activator %s\n' "$expected_sha"
printf '[saturn-release-helper-install] previous activator %s\n' "$old_helper_sha"
printf '[saturn-release-helper-install] unchanged config %s\n' "$old_config_sha"
printf '[saturn-release-helper-install] verified backup %s\n' "$backup"
