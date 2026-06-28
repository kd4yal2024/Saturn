#!/usr/bin/env bash
# Shared Saturn Go web asset manifest and copy helpers.

SATURN_GO_HTML_ASSETS=(
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

  (
    cd "$remote_web_dir"
    npm ci
    npm run build
  ) || return 1

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
