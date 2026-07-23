#!/usr/bin/env bash
set -Eeuo pipefail

# Build one complete, inactive Saturn application release from an exact clean
# Git commit. This script never writes /opt/saturn/current and never restarts a
# service; activation belongs to the root deployment transaction.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="${SATURN_RELEASE_REPO_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd -P)}"
OUTPUT_ROOT="${SATURN_RELEASE_OUTPUT_ROOT:-/var/lib/saturn-state/release-staging}"
SOURCE_WORKTREE_ROOT="${SATURN_RELEASE_SOURCE_WORKTREE_ROOT:-}"
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
RESOLVE_ONLY=0
SOURCE_REMOTE=""
SOURCE_REF=""
SOURCE_WORKTREE=""
FINAL_DIR=""
TEMP_STAGE=""
declare -a TRACKED_BUILD_OUTPUTS=()

log(){ printf '[saturn-release-build] %s\n' "$*"; }
die(){ printf '[saturn-release-build] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage: saturn-release-build.sh [--output-root DIR] [--dry-run]
       saturn-release-build.sh --source-remote URL --source-ref REF [--output-root DIR] [--dry-run]
       saturn-release-build.sh --source-remote URL --source-ref REF --resolve-only

Without source options, builds the current clean Git commit. With source
options, resolves the requested remote branch/tag once, fetches and verifies
that exact commit, then builds from a detached temporary worktree. The
completed bundle is written to OUTPUT_ROOT/<full-commit>. The script does not
install, activate, restart, or roll back services.
EOF
}

cleanup(){
  local rc="$?"
  local relative
  set +e
  if (( rc != 0 )) && [[ -n "$TEMP_STAGE" && -d "$TEMP_STAGE" ]]; then
    rm -rf -- "$TEMP_STAGE"
  fi
  if [[ -n "$SOURCE_WORKTREE" ]]; then
    git -C "$REPO_ROOT" worktree remove --force "$SOURCE_WORKTREE" >/dev/null 2>&1 \
      || rm -rf -- "$SOURCE_WORKTREE"
    git -C "$REPO_ROOT" worktree prune >/dev/null 2>&1 || true
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
    --source-remote)
      [[ $# -ge 2 ]] || die "--source-remote requires a repository URL"
      SOURCE_REMOTE="$2"
      shift 2
      ;;
    --source-ref)
      [[ $# -ge 2 ]] || die "--source-ref requires a branch or tag"
      SOURCE_REF="$2"
      shift 2
      ;;
    --resolve-only) RESOLVE_ONLY=1; shift ;;
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

resolve_remote_source(){
  local probe detail canonical exact_commit peeled_commit
  local -a patterns=()
  local -a matching_refs=()

  [[ -n "$SOURCE_REMOTE" ]] || die "--source-remote is required with --source-ref"
  [[ -n "$SOURCE_REF" ]] || die "--source-ref is required with --source-remote"
  [[ "$SOURCE_REMOTE" != -* && "$SOURCE_REMOTE" != *$'\n'* && "$SOURCE_REMOTE" != *$'\r'* ]] \
    || die "invalid source remote"
  [[ "$SOURCE_REF" != -* && "$SOURCE_REF" != *$'\n'* && "$SOURCE_REF" != *$'\r'* ]] \
    || die "invalid source ref"

  case "$SOURCE_REF" in
    refs/heads/*|refs/tags/*)
      git check-ref-format "$SOURCE_REF" >/dev/null 2>&1 || die "invalid source ref: $SOURCE_REF"
      patterns=("$SOURCE_REF")
      ;;
    refs/*)
      die "source ref must identify a branch or tag: $SOURCE_REF"
      ;;
    *)
      git check-ref-format "refs/heads/$SOURCE_REF" >/dev/null 2>&1 \
        || die "invalid source ref: $SOURCE_REF"
      patterns=("refs/heads/$SOURCE_REF" "refs/tags/$SOURCE_REF")
      ;;
  esac

  if ! probe="$(GIT_TERMINAL_PROMPT=0 git ls-remote --exit-code --refs \
      "$SOURCE_REMOTE" "${patterns[@]}" 2>&1)"; then
    die "cannot resolve source ref '$SOURCE_REF' from '$SOURCE_REMOTE': $probe"
  fi
  while IFS=$'\t' read -r _ ref; do
    [[ -n "$ref" ]] && matching_refs+=("$ref")
  done <<<"$probe"
  [[ "${#matching_refs[@]}" -eq 1 ]] \
    || die "source ref '$SOURCE_REF' is ambiguous between a branch and tag; use a full refs/... name"
  canonical="${matching_refs[0]}"

  if ! detail="$(GIT_TERMINAL_PROMPT=0 git ls-remote --exit-code \
      "$SOURCE_REMOTE" "$canonical" "${canonical}^{}" 2>&1)"; then
    die "cannot resolve exact source commit for '$canonical': $detail"
  fi
  exact_commit=""
  peeled_commit=""
  while IFS=$'\t' read -r commit ref; do
    case "$ref" in
      "$canonical") exact_commit="$commit" ;;
      "${canonical}^{}") peeled_commit="$commit" ;;
    esac
  done <<<"$detail"
  RESOLVED_SOURCE_COMMIT="${peeled_commit:-$exact_commit}"
  [[ "$RESOLVED_SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]] \
    || die "source ref did not resolve to one full commit: $canonical"
  RESOLVED_SOURCE_REF="$canonical"
}

build_resolved_remote_source(){
  local fetched_commit child_rc
  local -a child_args=(--output-root "$OUTPUT_ROOT")

  resolve_remote_source
  if (( RESOLVE_ONLY )); then
    printf 'source_remote=%s\n' "$SOURCE_REMOTE"
    printf 'requested_ref=%s\n' "$SOURCE_REF"
    printf 'resolved_ref=%s\n' "$RESOLVED_SOURCE_REF"
    printf 'resolved_commit=%s\n' "$RESOLVED_SOURCE_COMMIT"
    return 0
  fi

  log "Resolved $SOURCE_REMOTE $SOURCE_REF -> $RESOLVED_SOURCE_REF @ $RESOLVED_SOURCE_COMMIT"
  GIT_TERMINAL_PROMPT=0 git -C "$REPO_ROOT" fetch --no-tags \
    "$SOURCE_REMOTE" "$RESOLVED_SOURCE_REF"
  fetched_commit="$(git -C "$REPO_ROOT" rev-parse 'FETCH_HEAD^{commit}' 2>/dev/null || true)"
  fetched_commit="${fetched_commit,,}"
  [[ "$fetched_commit" == "$RESOLVED_SOURCE_COMMIT" ]] \
    || die "source ref moved while preparing the build; resolved $RESOLVED_SOURCE_COMMIT but fetched ${fetched_commit:-unknown}"

  SOURCE_WORKTREE_ROOT="${SOURCE_WORKTREE_ROOT:-$OUTPUT_ROOT/.source-worktrees}"
  mkdir -p "$SOURCE_WORKTREE_ROOT"
  SOURCE_WORKTREE="$(mktemp -d "$SOURCE_WORKTREE_ROOT/saturn-release-source.XXXXXX")"
  rmdir "$SOURCE_WORKTREE"
  git -C "$REPO_ROOT" worktree add --detach "$SOURCE_WORKTREE" "$RESOLVED_SOURCE_COMMIT"
  (( DRY_RUN )) && child_args+=(--dry-run)

  set +e
  SATURN_RELEASE_REPO_ROOT="$SOURCE_WORKTREE" \
  SATURN_RELEASE_EXPECTED_COMMIT="$RESOLVED_SOURCE_COMMIT" \
  SATURN_RELEASE_SOURCE_REMOTE="$SOURCE_REMOTE" \
  SATURN_RELEASE_REQUESTED_REF="$SOURCE_REF" \
  SATURN_RELEASE_RESOLVED_REF="$RESOLVED_SOURCE_REF" \
    "$SOURCE_WORKTREE/update_manager/scripts/saturn-release-build.sh" "${child_args[@]}"
  child_rc="$?"
  set -e
  return "$child_rc"
}

ensure_clean_exact_source(){
  local status current_branch
  [[ -d "$REPO_ROOT/.git" || -f "$REPO_ROOT/.git" ]] || die "not a Git checkout: $REPO_ROOT"
  RELEASE_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || true)"
  [[ "$RELEASE_COMMIT" =~ ^[0-9a-fA-F]{40}$ ]] || die "cannot resolve full source commit"
  RELEASE_COMMIT="${RELEASE_COMMIT,,}"
  if [[ -n "${SATURN_RELEASE_EXPECTED_COMMIT:-}" ]]; then
    [[ "${SATURN_RELEASE_EXPECTED_COMMIT,,}" == "$RELEASE_COMMIT" ]] \
      || die "source checkout commit differs from resolved commit: expected ${SATURN_RELEASE_EXPECTED_COMMIT,,}, got $RELEASE_COMMIT"
  fi
  status="$(git -C "$REPO_ROOT" status --porcelain=v1 --untracked-files=normal)"
  [[ -z "$status" ]] || die "release source tree is not clean; commit or remove all changes first"
  REPOSITORY_URL="${SATURN_RELEASE_SOURCE_REMOTE:-$(git -C "$REPO_ROOT" config --get remote.origin.url 2>/dev/null || printf 'unknown')}"
  current_branch="$(git -C "$REPO_ROOT" symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
  REQUESTED_SOURCE_REF="${SATURN_RELEASE_REQUESTED_REF:-${current_branch:-$RELEASE_COMMIT}}"
  RESOLVED_SOURCE_REF="${SATURN_RELEASE_RESOLVED_REF:-$RELEASE_COMMIT}"
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

  log "Running persistent-state compatibility tests"
  run "$REPO_ROOT/tests/test-saturn-state-compatibility.sh"
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

normalize_release_permissions(){
  log "Normalizing inactive release permissions"
  if (( DRY_RUN )); then
    log "[dry-run] remove group/world write bits from the complete release payload"
    return 0
  fi
  find "$TEMP_STAGE" -type d -exec chmod 0755 {} +
  find "$TEMP_STAGE" -type f -exec chmod go-w {} +
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
    --requested-ref "$REQUESTED_SOURCE_REF" \
    --resolved-ref "$RESOLVED_SOURCE_REF" \
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
    --build-result release-manifest-validation \
    --build-result state-compatibility-tests
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
if [[ -n "$SOURCE_REMOTE" || -n "$SOURCE_REF" || "$RESOLVE_ONLY" == "1" ]]; then
  build_resolved_remote_source
  exit $?
fi
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
log "Source repository: $REPOSITORY_URL"
log "Requested source ref: $REQUESTED_SOURCE_REF"
log "Resolved source ref: $RESOLVED_SOURCE_REF"
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
normalize_release_permissions
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
