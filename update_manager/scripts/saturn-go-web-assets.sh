#!/usr/bin/env bash
# Shared Saturn Go web asset manifest and copy helpers.

SATURN_GO_HTML_ASSETS=(
  "overview.html"
  "index.html"
  "monitor.html"
  "backup.html"
  "update.html"
  "saturngo.html"
  "p23test.html"
  "fpga.html"
  "pihpsdr.html"
  "deskhpsdr.html"
  "tailscale.html"
  "settings.html"
  "saturn-remote-next.html"
  "saturn-remote-next.js"
  "saturn-remote-next.js.sha256"
)

saturn_go_build_remote_web_assets() {
  local repo_dir="$1"
  local remote_web_dir="$repo_dir/remote-web"
  local dist_dir="$remote_web_dir/dist"
  local bundle_path="$remote_web_dir/dist/saturn-remote-next.js"
  local checksum_path="$bundle_path.sha256"
  local build_uid build_user

  if [[ ! -f "$remote_web_dir/package.json" ]]; then
    echo "[ERR] remote-web project not found: $remote_web_dir" >&2
    return 1
  fi
  if [[ ! -f "$remote_web_dir/package-lock.json" ]]; then
    echo "[ERR] remote-web package-lock.json is required for reproducible builds" >&2
    return 1
  fi
  if ! command -v npm >/dev/null 2>&1; then
    echo "[ERR] npm is required to build $bundle_path" >&2
    return 1
  fi
  if ! command -v sha256sum >/dev/null 2>&1; then
    echo "[ERR] sha256sum is required to verify $bundle_path" >&2
    return 1
  fi
  build_uid="$(stat -c '%u' "$remote_web_dir" 2>/dev/null || printf '0')"
  build_user="$(getent passwd "$build_uid" | cut -d: -f1 || true)"
  if [[ "$(id -u)" -eq 0 && -n "$build_user" && "$build_uid" != "0" ]]; then
    if [[ -d "$remote_web_dir/node_modules" ]] && ! runuser -u "$build_user" -- test -w "$remote_web_dir/node_modules"; then
      echo "[ERR] remote-web node_modules is not writable by $build_user: $remote_web_dir/node_modules" >&2
      echo "[ERR] Fix ownership, for example: sudo chown -R $build_user:$build_user '$remote_web_dir/node_modules' '$dist_dir'" >&2
      return 1
    fi
    if [[ -d "$dist_dir" ]] && ! runuser -u "$build_user" -- test -w "$dist_dir"; then
      echo "[ERR] remote-web dist is not writable by $build_user: $dist_dir" >&2
      echo "[ERR] Fix ownership, for example: sudo chown -R $build_user:$build_user '$remote_web_dir/node_modules' '$dist_dir'" >&2
      return 1
    fi
  else
    if [[ -d "$remote_web_dir/node_modules" && ! -w "$remote_web_dir/node_modules" ]]; then
      echo "[ERR] remote-web node_modules is not writable by $(id -un): $remote_web_dir/node_modules" >&2
      echo "[ERR] Fix ownership, for example: sudo chown -R $(id -un):$(id -gn) '$remote_web_dir/node_modules' '$dist_dir'" >&2
      return 1
    fi
    if [[ -d "$dist_dir" && ! -w "$dist_dir" ]]; then
      echo "[ERR] remote-web dist is not writable by $(id -un): $dist_dir" >&2
      echo "[ERR] Fix ownership, for example: sudo chown -R $(id -un):$(id -gn) '$remote_web_dir/node_modules' '$dist_dir'" >&2
      return 1
    fi
  fi

  if [[ "$(id -u)" -eq 0 && -n "$build_user" && "$build_uid" != "0" ]]; then
    runuser -u "$build_user" -- sh -c 'cd "$1" && npm ci && npm run build' sh "$remote_web_dir"
  else
    (
      cd "$remote_web_dir"
      npm ci
      npm run build
    )
  fi || return 1

  if [[ ! -s "$bundle_path" ]]; then
    echo "[ERR] remote-web build did not produce $bundle_path" >&2
    return 1
  fi

  (
    cd "$dist_dir"
    sha256sum "saturn-remote-next.js" >"$(basename "$checksum_path")"
  ) || return 1
  if [[ ! -s "$checksum_path" ]]; then
    echo "[ERR] remote-web build did not produce $checksum_path" >&2
    return 1
  fi
}

saturn_go_copy_template_asset() {
  local templates_dir="$1"
  local repo_dir="$2"
  local dest_dir="$3"
  local name="$4"
  local from_template="$templates_dir/$name"
  local from_repo="$repo_dir/$name"
  local from_remote_web_dist="$repo_dir/remote-web/dist/$name"

  case "$name" in
    saturn-remote-next.js|saturn-remote-next.js.sha256)
      if [[ -f "$from_remote_web_dist" ]]; then
        cp -f "$from_remote_web_dist" "$dest_dir/$name"
        return 0
      fi
      return 1
      ;;
  esac

  if [[ -f "$from_template" ]]; then
    cp -f "$from_template" "$dest_dir/$name"
  elif [[ -f "$from_repo" ]]; then
    cp -f "$from_repo" "$dest_dir/$name"
  elif [[ -f "$from_remote_web_dist" ]]; then
    cp -f "$from_remote_web_dist" "$dest_dir/$name"
  else
    return 1
  fi
}

saturn_go_verify_remote_web_bundle() {
  local dest_dir="$1"
  local bundle="$dest_dir/saturn-remote-next.js"
  local checksum="$dest_dir/saturn-remote-next.js.sha256"

  if [[ ! -s "$bundle" ]]; then
    echo "[ERR] deployed remote-web bundle missing: $bundle" >&2
    return 1
  fi
  if [[ ! -s "$checksum" ]]; then
    echo "[ERR] deployed remote-web checksum missing: $checksum" >&2
    return 1
  fi

  (
    cd "$dest_dir"
    sha256sum -c "saturn-remote-next.js.sha256"
  ) >/dev/null

  saturn_go_assert_remote_web_features "$dest_dir"
}

# Features that must be present in every shipped remote-next asset set.
#
# On 2026-09-25 a `sudo ./install.sh` on a live appliance deployed these assets
# from a checkout parked on a stale side branch, which silently removed the
# High-Res 3D waterfall and its display settings from the served page. Asserting
# the content here turns that class of downgrade into a failed install.
#
# Override only for a deliberately reduced UI: SATURN_ALLOW_LEGACY_WEB_ASSETS=1
SATURN_GO_REQUIRED_TEMPLATE_MARKERS=("terrain-canvas" "view-3d")
SATURN_GO_REQUIRED_BUNDLE_MARKERS=("TerrainRenderer")

saturn_go_assert_remote_web_features() {
  local dest_dir="$1"
  local html="$dest_dir/saturn-remote-next.html"
  local bundle="$dest_dir/saturn-remote-next.js"
  local marker failures=0

  case "${SATURN_ALLOW_LEGACY_WEB_ASSETS:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
  esac

  for marker in "${SATURN_GO_REQUIRED_TEMPLATE_MARKERS[@]}"; do
    if ! grep -q -- "$marker" "$html" 2>/dev/null; then
      echo "[ERR] $html is missing the High-Res 3D UI marker '$marker'" >&2
      failures=$((failures + 1))
    fi
  done
  for marker in "${SATURN_GO_REQUIRED_BUNDLE_MARKERS[@]}"; do
    if ! grep -q -- "$marker" "$bundle" 2>/dev/null; then
      echo "[ERR] $bundle is missing '$marker'" >&2
      failures=$((failures + 1))
    fi
  done

  if (( failures > 0 )); then
    cat >&2 <<'EOF'
[ERR] Refusing to ship a remote-next UI without the High-Res 3D waterfall.
[ERR] The install source is most likely a stale checkout or a stale dist bundle.
[ERR] Fix:  git -C <install-repo> fetch origin
[ERR]       git -C <install-repo> checkout main && git -C <install-repo> pull --ff-only
[ERR] Then re-run the install.
[ERR] Override for a deliberate legacy build: SATURN_ALLOW_LEGACY_WEB_ASSETS=1
EOF
    return 1
  fi
  return 0
}

# Git toplevel for a source directory, or empty when it is not a checkout.
saturn_go_source_repo_root() {
  local source_dir="$1"
  [[ -n "$source_dir" && -d "$source_dir" ]] || return 1
  command -v git >/dev/null 2>&1 || return 1
  git -C "$source_dir" rev-parse --show-toplevel 2>/dev/null
}

# Warn when the install source is missing commits from the release ref
# (default origin/main). This is the failure that removed the 3D waterfall: the
# install ran from a checkpoint branch 52 commits behind main.
#
# A hard failure here would also trip on a legitimately rebased/diverged local
# branch whose content is fine, so the hard guarantee comes from
# saturn_go_assert_remote_web_features (deployed content) instead. Production
# installs that must track the release branch can opt in to strictness with
# SATURN_REQUIRE_CURRENT_CHECKOUT=1; pinned/rollback installs can silence the
# warning with SATURN_ALLOW_STALE_CHECKOUT=1.
saturn_go_check_source_checkout() {
  local source_dir="$1"
  local repo_root branch release_ref counts ahead behind

  case "${SATURN_ALLOW_STALE_CHECKOUT:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
  esac

  repo_root="$(saturn_go_source_repo_root "$source_dir")" || return 0
  release_ref="${SATURN_INSTALL_RELEASE_REF:-origin/main}"

  if ! git -C "$repo_root" rev-parse --verify --quiet "${release_ref}^{commit}" >/dev/null; then
    echo "[WARN] install source $repo_root has no '$release_ref' ref; freshness not verified" >&2
    return 0
  fi

  counts="$(git -C "$repo_root" rev-list --left-right --count "HEAD...${release_ref}" 2>/dev/null || true)"
  [[ -n "$counts" ]] || return 0
  ahead="${counts%%[[:space:]]*}"
  behind="${counts##*[[:space:]]}"
  [[ "$behind" =~ ^[0-9]+$ && "$ahead" =~ ^[0-9]+$ ]] || return 0

  branch="$(git -C "$repo_root" rev-parse --abbrev-ref HEAD 2>/dev/null || printf 'detached')"
  if (( behind > 0 )); then
    # A rebase (e.g. dropping a trailer) leaves HEAD "behind" the release ref
    # while shipping byte-identical content. Only warn when the trees differ.
    if git -C "$repo_root" diff --quiet HEAD "$release_ref" 2>/dev/null; then
      echo "[INFO] install source is $behind commit(s) behind $release_ref but the tree matches; treating as current" >&2
      return 0
    fi
    echo "[WARN] Install source may be stale: $repo_root" >&2
    echo "[WARN]   branch      : $branch" >&2
    echo "[WARN]   HEAD        : $(git -C "$repo_root" log -1 --format='%h %ad %s' --date=short 2>/dev/null)" >&2
    echo "[WARN]   $release_ref : $(git -C "$repo_root" log -1 --format='%h %ad %s' --date=short "$release_ref" 2>/dev/null)" >&2
    echo "[WARN]   behind=$behind commit(s), ahead=$ahead" >&2
    echo "[WARN] Update it to install the released UI and backend:" >&2
    echo "[WARN]   git -C $repo_root fetch origin" >&2
    echo "[WARN]   git -C $repo_root checkout main && git -C $repo_root pull --ff-only" >&2
    echo "[WARN] Silence (pinned/rollback): SATURN_ALLOW_STALE_CHECKOUT=1" >&2
    case "${SATURN_REQUIRE_CURRENT_CHECKOUT:-0}" in
      1|true|TRUE|yes|YES|on|ON)
        echo "[ERR] SATURN_REQUIRE_CURRENT_CHECKOUT=1 and the install source is $behind commit(s) behind $release_ref" >&2
        return 1
        ;;
    esac
  fi
  return 0
}

# Record what was actually deployed so the live revision is always discoverable.
saturn_go_record_source_revision() {
  local source_dir="$1"
  local dest_dir="$2"
  local repo_root branch commit worktree

  [[ -d "$dest_dir" ]] || return 0
  repo_root="$(saturn_go_source_repo_root "$source_dir")" || return 0
  branch="$(git -C "$repo_root" rev-parse --abbrev-ref HEAD 2>/dev/null || printf 'detached')"
  commit="$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || printf 'unknown')"
  if [[ -n "$(git -C "$repo_root" status --porcelain 2>/dev/null)" ]]; then
    worktree="dirty"
  else
    worktree="clean"
  fi

  {
    printf 'branch=%s\n' "$branch"
    printf 'commit=%s\n' "$commit"
    printf 'worktree=%s\n' "$worktree"
    printf 'release_ref=%s\n' "${SATURN_INSTALL_RELEASE_REF:-origin/main}"
    printf 'installed_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$dest_dir/.saturn-web-revision" 2>/dev/null || true
}

saturn_go_copy_required_web_assets() {
  local templates_dir="$1"
  local repo_dir="$2"
  local dest_dir="$3"
  local name

  for name in "${SATURN_GO_HTML_ASSETS[@]}"; do
    saturn_go_copy_template_asset "$templates_dir" "$repo_dir" "$dest_dir" "$name" || return 1
  done
}

# Shared appliance-shell assets (saturn-ui.css, saturn-shell.js, vendored
# CDN libraries, self-hosted fonts) live in templates/assets/ and are copied
# as a directory tree to dest_dir/assets/.
saturn_go_copy_shared_assets() {
  local templates_dir="$1"
  local dest_dir="$2"
  local src_dir="$templates_dir/assets"

  if [[ ! -d "$src_dir" ]]; then
    echo "[ERR] shared web assets directory not found: $src_dir" >&2
    return 1
  fi

  mkdir -p "$dest_dir/assets"
  cp -rf "$src_dir/." "$dest_dir/assets/"
}
