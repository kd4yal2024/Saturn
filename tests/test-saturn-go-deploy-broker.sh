#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BROKER="$REPO_ROOT/update_manager/scripts/saturn-go-deploy-root.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
export SATURN_GO_DEPLOY_CONFIG="$TMP_ROOT/nonexistent-deploy-config"

make_stage(){
  local stage="$1"
  install -d -m 0755 "$stage/webroot/assets/css" "$stage/scripts"
  printf '#!/bin/sh\nexit 0\n' >"$stage/saturn-go"
  printf '<!doctype html>\n' >"$stage/webroot/index.html"
  printf 'body { color: #fff; }\n' >"$stage/webroot/assets/css/saturn-ui.css"
  printf '#!/bin/sh\nexit 0\n' >"$stage/scripts/status.sh"
  chmod 0755 "$stage/saturn-go" "$stage/scripts/status.sh"
  chmod 0644 "$stage/webroot/index.html" "$stage/webroot/assets/css/saturn-ui.css"
  refresh_manifest "$stage"
}

refresh_manifest(){
  local stage="$1"
  rm -f "$stage/SHA256SUMS"
  (cd "$stage" && find . -type f ! -name SHA256SUMS -printf '%P\0' | \
    sort -z | xargs -0 sha256sum >SHA256SUMS)
  chmod 0644 "$stage/SHA256SUMS"
}

expect_rejected(){
  local stage="$1" reason="$2"
  if "$BROKER" --validate "$stage" >/dev/null 2>&1; then
    printf 'broker accepted invalid stage: %s\n' "$reason" >&2
    exit 1
  fi
}

stage="$TMP_ROOT/valid"
make_stage "$stage"
"$BROKER" --validate "$stage" >/dev/null

chmod 0775 "$stage"
expect_rejected "$stage" "group-writable stage directory"
chmod 0755 "$stage"

printf 'not checksummed\n' >"$stage/webroot/extra.html"
expect_rejected "$stage" "manifest omission"
rm -f "$stage/webroot/extra.html"

ln -s /etc/passwd "$stage/scripts/link.sh"
expect_rejected "$stage" "symbolic link"
rm -f "$stage/scripts/link.sh"

install -d -m 0755 "$stage/webroot/private"
printf 'not a public asset\n' >"$stage/webroot/private/config.txt"
chmod 0644 "$stage/webroot/private/config.txt"
refresh_manifest "$stage"
expect_rejected "$stage" "nested webroot directory outside assets"
rm -rf "$stage/webroot/private"
refresh_manifest "$stage"

printf '#!/bin/sh\nexit 0\n' >"$stage/deploy-root-helper.sh"
chmod 0755 "$stage/deploy-root-helper.sh"
(cd "$stage" && sha256sum deploy-root-helper.sh >>SHA256SUMS)
expect_rejected "$stage" "staged executable helper"

printf 'deploy broker tests passed\n'
