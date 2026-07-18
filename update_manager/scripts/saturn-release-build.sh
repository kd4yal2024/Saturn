#!/usr/bin/env bash
set -Eeuo pipefail

# Build one complete, inactive Saturn application release from an exact clean
# Git commit. This script never writes /opt/saturn/current and never restarts a
# service; activation belongs to the root deployment transaction.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="${SATURN_RELEASE_REPO_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd -P)}"
OUTPUT_ROOT="${SATURN_RELEASE_OUTPUT_ROOT:-/var/lib/saturn-state/release-staging}"
COMPONENTS_FILE="${SATURN_RELEASE_COMPONENTS_FILE:-$REPO_ROOT/update_manager/release/components-v1.json}"
MANIFEST_TOOL="${SATURN_RELEASE_MANIFEST_TOOL:-$REPO_ROOT/update_manager/scripts/saturn-release-manifest.py}"
BUILD_JOBS="${SATURN_RELEASE_BUILD_JOBS:-1}"
BUILD_NICE="${SATURN_RELEASE_BUILD_NICE:-15}"
BUILD_IONICE_CLASS="${SATURN_RELEASE_BUILD_IONICE_CLASS:-3}"
BUILD_USER="${SATURN_RELEASE_BUILD_USER:-$(id -un)}"
BUILD_SWAP_FILE="${SATURN_SATURNGO_BUILD_SWAP_FILE:-$(getent passwd "$BUILD_USER" | cut -d: -f6)/saturn-build.swap}"
BUILD_SWAP_MIB="${SATURN_SATURNGO_BUILD_SWAP_MIB:-2048}"
BUILD_PREFLIGHT_SOURCE="$REPO_ROOT/update_manager/scripts/saturn-go-build-preflight.sh"
BUILD_PREFLIGHT_INSTALLED="${SATURN_RELEASE_BUILD_PREFLIGHT_HELPER:-/usr/local/lib/saturn-go/scripts/saturn-go-build-preflight.sh}"
WEB_ASSET_HELPERS="$REPO_ROOT/update_manager/scripts/saturn-go-web-assets.sh"
BRIDGE_INSTALLER="$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"
DRY_RUN=0
FINAL_DIR=""
TEMP_STAGE=""
declare -a TRACKED_BUILD_OUTPUTS=()

log(){ printf '[saturn-release-build] %s\n' "$*"; }
die(){ printf '[saturn-release-build] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage: saturn-release-build.sh [--output-root DIR] [--dry-run]

Builds and validates a complete inactive application release from the current
clean Git commit. The completed bundle is written to OUTPUT_ROOT/<full-commit>.
The script does not install, activate, restart, or roll back services.
EOF
}

cleanup(){
  local rc="$?"
  local relative
  set +e
  if (( rc != 0 )) && [[ -n "$TEMP_STAGE" && -d "$TEMP_STAGE" ]]; then
    rm -rf -- "$TEMP_STAGE"
  fi
  for relative in "${TRACKED_BUILD_OUTPUTS[@]}"; do
    git -C "$REPO_ROOT" checkout-index -f -- "$relative"
  done
  return "$rc"
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-root)
      [[ $# -ge 2 ]] || die "--output-root requires a directory"
      OUTPUT_ROOT="$2"
      shift 2
      ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

run(){
  if (( DRY_RUN )); then
    printf '[dry-run]'
    printf ' %q' "$@"
    printf '\n'
    return 0
  fi
  "$@"
}

run_low_memory(){
  if (( DRY_RUN )); then
    printf '[dry-run] CARGO_BUILD_JOBS=%q TMPDIR=%q CARGO_TARGET_DIR=%q nice -n %q ionice -c %q' \
      "$BUILD_JOBS" "$1" "$2" "$BUILD_NICE" "$BUILD_IONICE_CLASS"
    shift 2
    printf ' %q' "$@"
    printf '\n'
    return 0
  fi
  local tmpdir="$1" target_dir="$2"
  shift 2
  CARGO_BUILD_JOBS="$BUILD_JOBS" \
  TMPDIR="$tmpdir" \
  CARGO_TARGET_DIR="$target_dir" \
    nice -n "$BUILD_NICE" ionice -c "$BUILD_IONICE_CLASS" "$@"
}

require_positive_integer(){
  [[ "$2" =~ ^[1-9][0-9]*$ ]] || die "$1 must be a positive integer, got: $2"
}

ensure_clean_exact_source(){
  local status
  [[ -d "$REPO_ROOT/.git" || -f "$REPO_ROOT/.git" ]] || die "not a Git checkout: $REPO_ROOT"
  RELEASE_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || true)"
  [[ "$RELEASE_COMMIT" =~ ^[0-9a-fA-F]{40}$ ]] || die "cannot resolve full source commit"
  RELEASE_COMMIT="${RELEASE_COMMIT,,}"
  status="$(git -C "$REPO_ROOT" status --porcelain=v1 --untracked-files=normal)"
  [[ -z "$status" ]] || die "release source tree is not clean; commit or remove all changes first"
  REPOSITORY_URL="$(git -C "$REPO_ROOT" config --get remote.origin.url 2>/dev/null || printf 'unknown')"
}

record_tracked_build_outputs(){
  local source
  while IFS= read -r source; do
    if git -C "$REPO_ROOT" ls-files --error-unmatch "$source" >/dev/null 2>&1; then
      TRACKED_BUILD_OUTPUTS+=("$source")
    fi
  done < <(python3 - "$COMPONENTS_FILE" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    descriptor = json.load(handle)
for component in descriptor["components"]:
    print(component["source"])
PY
  )
}

ensure_low_memory_capacity(){
  local total_mib
  total_mib="$(awk '$1 == "MemTotal:" { print int($2 / 1024); exit }' /proc/meminfo)"
  [[ "$total_mib" =~ ^[0-9]+$ ]] || die "cannot determine total memory"
  if (( total_mib >= 2048 )); then
    log "RAM ${total_mib} MiB; dedicated build swap is optional"
    return 0
  fi
  log "RAM ${total_mib} MiB; requiring ${BUILD_SWAP_MIB} MiB disk-backed build swap"
  if (( DRY_RUN )); then
    log "[dry-run] ensure build swap: $BUILD_SWAP_FILE"
    return 0
  fi
  if [[ -x "$BUILD_PREFLIGHT_INSTALLED" ]]; then
    SATURN_SATURNGO_BUILD_USER="$BUILD_USER" \
    SATURN_SATURNGO_BUILD_SWAP_FILE="$BUILD_SWAP_FILE" \
    SATURN_SATURNGO_BUILD_SWAP_MIB="$BUILD_SWAP_MIB" \
      sudo -n "$BUILD_PREFLIGHT_INSTALLED" ensure-swap
  elif (( EUID == 0 )); then
    SATURN_SATURNGO_BUILD_USER="$BUILD_USER" \
    SATURN_SATURNGO_BUILD_SWAP_FILE="$BUILD_SWAP_FILE" \
    SATURN_SATURNGO_BUILD_SWAP_MIB="$BUILD_SWAP_MIB" \
      "$BUILD_PREFLIGHT_SOURCE" ensure-swap
  else
    die "low-memory release build requires the installed privileged build-preflight helper"
  fi
}

run_test_gates(){
  local server_tmp="$REPO_ROOT/update_manager/rust-server/.tmp"
  local server_target="$REPO_ROOT/update_manager/rust-server/target-local"
  local bridge_tmp="$REPO_ROOT/update_manager/saturn-bridge/.tmp"
  local bridge_target="$REPO_ROOT/update_manager/saturn-bridge/target-local/stub-tests"
  local remote_web="$REPO_ROOT/update_manager/remote-web"

  log "Running Rust server tests"
  run mkdir -p "$server_tmp" "$server_target"
  if (( DRY_RUN )); then
    run_low_memory "$server_tmp" "$server_target" cargo test --locked -j "$BUILD_JOBS" --manifest-path "$REPO_ROOT/update_manager/rust-server/Cargo.toml"
  else
    SATURN_BUILD_COMMIT="$RELEASE_COMMIT" \
      run_low_memory "$server_tmp" "$server_target" cargo test --locked -j "$BUILD_JOBS" --manifest-path "$REPO_ROOT/update_manager/rust-server/Cargo.toml"
  fi

  log "Running Saturn Bridge tests with native DSP stubs"
  run mkdir -p "$bridge_tmp" "$bridge_target"
  if (( DRY_RUN )); then
    run_low_memory "$bridge_tmp" "$bridge_target" cargo test --locked -j "$BUILD_JOBS" --manifest-path "$REPO_ROOT/update_manager/saturn-bridge/Cargo.toml"
  else
    SATURN_BRIDGE_STUB_NATIVE=1 \
      run_low_memory "$bridge_tmp" "$bridge_target" cargo test --locked -j "$BUILD_JOBS" --manifest-path "$REPO_ROOT/update_manager/saturn-bridge/Cargo.toml"
  fi

  log "Running Remote web lockfile, type, seam, unit, and production-bundle gates"
  if (( DRY_RUN )); then
    log "[dry-run] npm ci; npm run typecheck; npm run check:seam; npm test; npm run build (cwd=$remote_web)"
  else
    (
      cd "$remote_web"
      npm ci
      npm run typecheck
      npm run check:seam
      npm test
      npm run build
      (
        cd dist
        sha256sum saturn-remote-next.js >saturn-remote-next.js.sha256
      )
    )
  fi

  log "Running Protocol 2 native boundary tests"
  run make -C "$REPO_ROOT/sw_projects/P2_app" test
}

build_release_components(){
  local server_tmp="$REPO_ROOT/update_manager/rust-server/.tmp"
  local server_target="$REPO_ROOT/update_manager/rust-server/target-local"

  log "Building Saturn Go release binary"
  if (( DRY_RUN )); then
    run_low_memory "$server_tmp" "$server_target" cargo build --locked --release -j "$BUILD_JOBS" --manifest-path "$REPO_ROOT/update_manager/rust-server/Cargo.toml"
  else
    SATURN_BUILD_COMMIT="$RELEASE_COMMIT" \
      run_low_memory "$server_tmp" "$server_target" cargo build --locked --release -j "$BUILD_JOBS" --manifest-path "$REPO_ROOT/update_manager/rust-server/Cargo.toml"
  fi

  log "Building Saturn Bridge release binary with pinned WDSP"
  if (( DRY_RUN )); then
    log "[dry-run] SATURN_BRIDGE_BUILD_ONLY=1 $BRIDGE_INSTALLER"
  else
    SATURN_USER="$BUILD_USER" \
    SATURN_REPO_ROOT="$REPO_ROOT" \
    SATURN_BRIDGE_SOURCE_DIR="$REPO_ROOT/update_manager/saturn-bridge" \
    SATURN_BRIDGE_BUILD_ONLY=1 \
    SATURN_BRIDGE_OUTPUT_BIN="$REPO_ROOT/update_manager/saturn-bridge/target-local/staged/saturn-bridge" \
    SATURN_BRIDGE_BUILD_JOBS="$BUILD_JOBS" \
      bash "$BRIDGE_INSTALLER"
  fi

  log "Building Protocol 2 and normal-release native tools"
  run make -C "$REPO_ROOT/sw_projects/P2_app" clean
  run make -C "$REPO_ROOT/sw_projects/P2_app" all -j "$BUILD_JOBS"
  local directory
  for directory in \
    sw_projects/audiotest \
    sw_projects/biascheck \
    sw_projects/codectest \
    sw_tools/axi_rw \
    sw_tools/FPGAVersion \
    sw_tools/IQdmatest \
    sw_tools/codecwrite \
    sw_tools/spiadcread \
    linuxdriver/tools
  do
    run make -C "$REPO_ROOT/$directory" clean
    run make -C "$REPO_ROOT/$directory" all -j "$BUILD_JOBS"
  done
}

copy_release_payload(){
  local source relative executable destination
  log "Assembling inactive release payload"
  run mkdir -p "$TEMP_STAGE/bin" "$TEMP_STAGE/webroot" "$TEMP_STAGE/scripts" "$TEMP_STAGE/share/release"
  if (( DRY_RUN )); then
    log "[dry-run] copy declared component binaries, web assets, and maintenance scripts"
    return 0
  fi

  while IFS=$'\t' read -r source relative executable; do
    [[ -n "$source" && -n "$relative" ]] || die "invalid component descriptor row"
    [[ -f "$REPO_ROOT/$source" ]] || die "built component missing: $source"
    destination="$TEMP_STAGE/$relative"
    mkdir -p "$(dirname "$destination")"
    install -m "$([[ "$executable" == "true" ]] && printf '0755' || printf '0644')" "$REPO_ROOT/$source" "$destination"
  done < <(python3 - "$COMPONENTS_FILE" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
for item in value["components"]:
    print("\t".join([
        item["source"],
        item["path"],
        "true" if item.get("executable") else "false",
    ]))
PY
  )

  saturn_go_copy_required_web_assets "$REPO_ROOT/update_manager/templates" "$REPO_ROOT/update_manager" "$TEMP_STAGE/webroot"
  saturn_go_copy_shared_assets "$REPO_ROOT/update_manager/templates" "$TEMP_STAGE/webroot"
  saturn_go_verify_remote_web_bundle "$TEMP_STAGE/webroot"
  install -m 0644 "$REPO_ROOT/update_manager/scripts/config.json" "$TEMP_STAGE/webroot/config.json"
  install -m 0644 "$REPO_ROOT/update_manager/scripts/themes.json" "$TEMP_STAGE/webroot/themes.json"

  find "$REPO_ROOT/update_manager/scripts" -maxdepth 1 -type f ! -name '.*' -print0 \
    | while IFS= read -r -d '' source; do
        case "$source" in
          *.sh|*.py) install -m 0755 "$source" "$TEMP_STAGE/scripts/$(basename "$source")" ;;
          *) install -m 0644 "$source" "$TEMP_STAGE/scripts/$(basename "$source")" ;;
        esac
      done
  local extra_script
  for extra_script in \
    scripts/fix-LED-power-button.sh \
    scripts/install-shutdown-waiter-service.sh \
    scripts/shutdown-waiter.sh \
    scripts/setup-eth-fallback.sh
  do
    install -m 0755 "$REPO_ROOT/$extra_script" "$TEMP_STAGE/scripts/$(basename "$extra_script")"
  done
  install -m 0644 "$COMPONENTS_FILE" "$TEMP_STAGE/share/release/components-v1.json"
}

create_manifest(){
  if (( DRY_RUN )); then
    log "[dry-run] create and validate release-manifest.json and SHA256SUMS"
    return 0
  fi
  log "Creating and validating release manifest"
  python3 "$MANIFEST_TOOL" create \
    --release-root "$TEMP_STAGE" \
    --repo-root "$REPO_ROOT" \
    --components "$COMPONENTS_FILE" \
    --commit "$RELEASE_COMMIT" \
    --repository "$REPOSITORY_URL" \
    --build-result rust-server-tests \
    --build-result bridge-stub-tests \
    --build-result remote-web-typecheck \
    --build-result remote-web-template-seam \
    --build-result remote-web-tests \
    --build-result remote-web-production-bundle \
    --build-result protocol2-boundary-tests \
    --build-result saturn-go-release-build \
    --build-result saturn-bridge-release-build \
    --build-result native-release-build \
    --build-result release-manifest-validation
}

require_positive_integer SATURN_RELEASE_BUILD_JOBS "$BUILD_JOBS"
need_cmd awk
need_cmd cargo
need_cmd git
need_cmd ionice
need_cmd make
need_cmd nice
need_cmd npm
need_cmd python3
need_cmd sha256sum
[[ -f "$COMPONENTS_FILE" ]] || die "component descriptor not found: $COMPONENTS_FILE"
[[ -x "$MANIFEST_TOOL" ]] || die "manifest tool is not executable: $MANIFEST_TOOL"
[[ -f "$WEB_ASSET_HELPERS" ]] || die "web asset helper not found: $WEB_ASSET_HELPERS"
[[ -x "$BRIDGE_INSTALLER" ]] || die "bridge installer is not executable: $BRIDGE_INSTALLER"
# shellcheck disable=SC1090
source "$WEB_ASSET_HELPERS"

ensure_clean_exact_source
record_tracked_build_outputs
FINAL_DIR="$OUTPUT_ROOT/$RELEASE_COMMIT"
log "Source commit: $RELEASE_COMMIT"
log "Inactive release destination: $FINAL_DIR"
[[ "$FINAL_DIR" != "/opt/saturn/current" ]] || die "release builder must never target the active pointer"
if (( DRY_RUN )); then
  TEMP_STAGE="$OUTPUT_ROOT/.${RELEASE_COMMIT}.build.DRYRUN"
else
  mkdir -p "$OUTPUT_ROOT"
  [[ ! -e "$FINAL_DIR" ]] || die "release bundle already exists: $FINAL_DIR"
  TEMP_STAGE="$(mktemp -d "$OUTPUT_ROOT/.${RELEASE_COMMIT}.build.XXXXXX")"
  chmod 0755 "$TEMP_STAGE"
fi

ensure_low_memory_capacity
run_test_gates
build_release_components
copy_release_payload
create_manifest

if (( DRY_RUN )); then
  log "Dry run complete; active release was not changed"
  exit 0
fi

find "$TEMP_STAGE" -type d -exec chmod 0755 {} +
mv "$TEMP_STAGE" "$FINAL_DIR"
TEMP_STAGE=""
log "Release bundle complete: $FINAL_DIR"
log "Active release was not changed"
