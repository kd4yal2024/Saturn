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
  "saturn-remote.html"
  "saturn-remote-next.html"
  "saturn-remote-next.js"
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
  local bundle_path="$remote_web_dir/dist/saturn-remote-next.js"

  if [[ ! -f "$remote_web_dir/package.json" ]]; then
    echo "[ERR] remote-web project not found: $remote_web_dir" >&2
    return 1
  fi
  if ! command -v npm >/dev/null 2>&1; then
    echo "[ERR] npm is required to build $bundle_path" >&2
    return 1
  fi

  (
    cd "$remote_web_dir"
    if [[ ! -d node_modules ]]; then
      if [[ -f package-lock.json ]]; then
        npm ci
      else
        npm install
      fi
    fi
    npm run build
  ) || return 1

  if [[ ! -s "$bundle_path" ]]; then
    echo "[ERR] remote-web build did not produce $bundle_path" >&2
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
