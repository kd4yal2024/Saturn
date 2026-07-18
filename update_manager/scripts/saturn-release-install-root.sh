#!/usr/bin/env bash
set -Eeuo pipefail

# Validate and install one completed application release into an immutable,
# versioned directory. This helper never changes /opt/saturn/current and never
# starts, stops, or restarts a service.

CONFIG_FILE="${SATURN_RELEASE_INSTALL_CONFIG:-/etc/default/saturn-release-install}"
VALIDATE_ONLY=0
RUN_USER="pi"
STAGING_ROOT="/var/lib/saturn-state/release-staging"
RELEASES_ROOT="/opt/saturn/releases"
MANIFEST_TOOL="/usr/local/lib/saturn-go/scripts/saturn-release-manifest.py"
COMPONENTS_FILE="/usr/local/lib/saturn-go/release/components-v1.json"
INSTALL_OWNER="root"
INSTALL_GROUP="root"
TEMP_INSTALL=""

log(){ printf '[saturn-release-install] %s\n' "$*"; }
die(){ printf '[saturn-release-install] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage:
  saturn-release-install-root.sh <bundle-directory>
  saturn-release-install-root.sh --validate <bundle-directory>

The install form copies a validated bundle into
/opt/saturn/releases/<full-commit>. It does not activate the release or restart
services. --validate checks the source bundle without installing it.
EOF
}

cleanup(){
  local rc="$?"
  set +e
  if [[ -n "$TEMP_INSTALL" && -d "$TEMP_INSTALL" ]]; then
    rm -rf -- "$TEMP_INSTALL"
  fi
  return "$rc"
}
trap cleanup EXIT

trim_config_value(){
  local value="$1"
  value="${value#\"}"
  value="${value%\"}"
  value="${value#\'}"
  value="${value%\'}"
  printf '%s' "$value"
}

load_config(){
  [[ -f "$CONFIG_FILE" ]] || return 0
  local owner mode line key value
  if (( EUID == 0 )); then
    owner="$(stat -c '%u' "$CONFIG_FILE")"
    mode="$(stat -c '%a' "$CONFIG_FILE")"
    [[ "$owner" == "0" ]] || die "install config is not root-owned: $CONFIG_FILE"
    (( (8#$mode & 8#022) == 0 )) || die "install config is group/world writable: $CONFIG_FILE"
  fi

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    [[ "$line" == *=* ]] || die "invalid install config line: $line"
    key="${line%%=*}"
    value="$(trim_config_value "${line#*=}")"
    case "$key" in
      RUN_USER) RUN_USER="$value" ;;
      STAGING_ROOT) STAGING_ROOT="$value" ;;
      RELEASES_ROOT) RELEASES_ROOT="$value" ;;
      MANIFEST_TOOL) MANIFEST_TOOL="$value" ;;
      COMPONENTS_FILE) COMPONENTS_FILE="$value" ;;
      INSTALL_OWNER) INSTALL_OWNER="$value" ;;
      INSTALL_GROUP) INSTALL_GROUP="$value" ;;
      *) die "unsupported install config key: $key" ;;
    esac
  done <"$CONFIG_FILE"
}

release_identity(){
  python3 - "$1/release-manifest.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    manifest = json.load(handle)
source = manifest.get("source", {})
build = manifest.get("build", {})
print(f'{source.get("commit", "")}\t{build.get("architecture", "")}')
PY
}

validate_payload_shape(){
  local root="$1" bad
  bad="$(find "$root" -xdev -type l -print -quit)"
  [[ -z "$bad" ]] || die "symbolic links are not permitted in a release: $bad"
  bad="$(find "$root" -xdev ! -type d ! -type f -print -quit)"
  [[ -z "$bad" ]] || die "non-regular release entry rejected: $bad"
  bad="$(find "$root" -xdev -perm /022 -print -quit)"
  [[ -z "$bad" ]] || die "group/world-writable release entry rejected: $bad"
  bad="$(find "$root" -xdev -type d ! -perm 0755 -print -quit)"
  [[ -z "$bad" ]] || die "release directory must use mode 0755: $bad"
}

validate_source_bundle(){
  local input="$1" staging bundle run_uid bad commit architecture leaf
  [[ -d "$STAGING_ROOT" ]] || die "release staging root not found: $STAGING_ROOT"
  staging="$(realpath -e "$STAGING_ROOT")"
  [[ -d "$input" ]] || die "release bundle not found: $input"
  bundle="$(realpath -e "$input")"
  [[ "$(dirname "$bundle")" == "$staging" ]] \
    || die "release bundle must be a direct child of the trusted staging root: $bundle"

  validate_payload_shape "$bundle"
  run_uid="$(id -u "$RUN_USER")"
  bad="$(find "$bundle" -xdev ! -uid "$run_uid" -print -quit)"
  [[ -z "$bad" ]] || die "staged release entry is not owned by $RUN_USER: $bad"

  python3 "$MANIFEST_TOOL" validate \
    --release-root "$bundle" \
    --components "$COMPONENTS_FILE" >/dev/null
  IFS=$'\t' read -r commit architecture < <(release_identity "$bundle")
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "release manifest has no valid full commit"
  leaf="$(basename "$bundle")"
  [[ "$leaf" == "$commit" ]] || die "bundle directory does not match manifest commit: $leaf"
  [[ "$architecture" == "$(uname -m)" ]] \
    || die "release architecture $architecture does not match host $(uname -m)"
  printf '%s\t%s\n' "$bundle" "$commit"
}

validate_installed_release(){
  local release="$1" expected_commit="$2" commit architecture bad
  [[ -d "$release" && ! -L "$release" ]] || die "installed release is not a real directory: $release"
  validate_payload_shape "$release"
  python3 "$MANIFEST_TOOL" validate \
    --release-root "$release" \
    --components "$COMPONENTS_FILE" >/dev/null
  IFS=$'\t' read -r commit architecture < <(release_identity "$release")
  [[ "$commit" == "$expected_commit" ]] || die "installed release commit mismatch: $commit"
  [[ "$architecture" == "$(uname -m)" ]] || die "installed release architecture mismatch: $architecture"

  if (( EUID == 0 )); then
    bad="$(find "$release" -xdev \( ! -user "$INSTALL_OWNER" -o ! -group "$INSTALL_GROUP" \) -print -quit)"
    [[ -z "$bad" ]] || die "installed release ownership mismatch: $bad"
  fi
}

install_release(){
  local bundle="$1" commit="$2" destination
  destination="$RELEASES_ROOT/$commit"

  if [[ -e "$destination" || -L "$destination" ]]; then
    validate_installed_release "$destination" "$commit"
    log "release is already installed and valid: $destination"
    return 0
  fi

  if (( EUID != 0 )) && [[ "$RELEASES_ROOT" == "/opt/saturn/releases" ]]; then
    die "release installation must run as root"
  fi

  if (( EUID == 0 )); then
    install -d -m 0755 -o root -g root "$(dirname "$RELEASES_ROOT")" "$RELEASES_ROOT"
  else
    install -d -m 0755 "$RELEASES_ROOT"
  fi
  [[ ! -L "$RELEASES_ROOT" ]] || die "releases root must not be a symbolic link: $RELEASES_ROOT"

  TEMP_INSTALL="$(mktemp -d "$RELEASES_ROOT/.${commit}.install.XXXXXX")"
  chmod 0700 "$TEMP_INSTALL"
  cp -a --no-dereference "$bundle/." "$TEMP_INSTALL/"
  validate_payload_shape "$TEMP_INSTALL"
  if (( EUID == 0 )); then
    chown -R "$INSTALL_OWNER:$INSTALL_GROUP" "$TEMP_INSTALL"
  fi
  chmod 0755 "$TEMP_INSTALL"
  validate_installed_release "$TEMP_INSTALL" "$commit"
  sync -f "$TEMP_INSTALL"
  mv -T "$TEMP_INSTALL" "$destination"
  TEMP_INSTALL=""
  sync -f "$RELEASES_ROOT"
  validate_installed_release "$destination" "$commit"
  log "installed inactive release: $destination"
  log "active release was not changed"
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi

if [[ "${1:-}" == "--validate" ]]; then
  VALIDATE_ONLY=1
  shift
fi
[[ $# -eq 1 ]] || { usage >&2; exit 2; }

need_cmd cp
need_cmd find
need_cmd install
need_cmd python3
need_cmd realpath
need_cmd sync
load_config
[[ -x "$MANIFEST_TOOL" ]] || die "trusted manifest validator not executable: $MANIFEST_TOOL"
[[ -f "$COMPONENTS_FILE" ]] || die "trusted component policy missing: $COMPONENTS_FILE"
id "$RUN_USER" >/dev/null 2>&1 || die "configured build user does not exist: $RUN_USER"
if (( EUID == 0 )); then
  [[ "$INSTALL_OWNER" == "root" && "$INSTALL_GROUP" == "root" ]] \
    || die "production release ownership must remain root:root"
fi

IFS=$'\t' read -r BUNDLE COMMIT < <(validate_source_bundle "$1")
if (( VALIDATE_ONLY )); then
  log "release bundle validation passed: $BUNDLE"
  exit 0
fi
install_release "$BUNDLE" "$COMMIT"
