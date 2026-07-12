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
  "saturn-remote.html"
  "saturn-remote-next.html"
  "saturn-remote-next.js"
  "saturn-remote-next.js.sha256"
)

SATURN_GO_OPTIONAL_TEMPLATE_ASSETS=(
  "saturn-remote-storage.js"
  "saturn-remote-session.js"
  "saturn-remote-tci.js"
  "saturn-remote-transport.js"
  "saturn-remote-browser.js"
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

saturn_go_copy_optional_web_assets() {
  local templates_dir="$1"
  local repo_dir="$2"
  local dest_dir="$3"
  local name

  for name in "${SATURN_GO_OPTIONAL_TEMPLATE_ASSETS[@]}"; do
    saturn_go_copy_template_asset "$templates_dir" "$repo_dir" "$dest_dir" "$name" || true
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
