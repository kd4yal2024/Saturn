#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BROKER="$REPO_ROOT/update_manager/scripts/saturn-go-deploy-root.sh"
INSTALLER="$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
export SATURN_GO_DEPLOY_CONFIG="$TMP_ROOT/nonexistent-deploy-config"

make_stage(){
  local stage="$1"
  install -d -m 0755 "$stage/webroot/assets/css" "$stage/scripts"
  printf '#!/bin/sh\nexit 0\n' >"$stage/saturn-go"
  printf '0123456789abcdef0123456789abcdef01234567\n' >"$stage/RELEASE_COMMIT"
  printf '<!doctype html>\n' >"$stage/webroot/index.html"
  printf 'body { color: #fff; }\n' >"$stage/webroot/assets/css/saturn-ui.css"
  printf '#!/bin/sh\nexit 0\n' >"$stage/scripts/status.sh"
  chmod 0755 "$stage/saturn-go" "$stage/scripts/status.sh"
  chmod 0644 "$stage/RELEASE_COMMIT" "$stage/webroot/index.html" "$stage/webroot/assets/css/saturn-ui.css"
  refresh_manifest "$stage"
}

refresh_manifest(){
  local stage="$1" manifest_tmp
  manifest_tmp="$(mktemp "$TMP_ROOT/manifest.XXXXXX")"
  rm -f "$stage/SHA256SUMS"
  (cd "$stage" && find . -type f ! -name SHA256SUMS -printf '%P\0' | \
    sort -z | xargs -0 sha256sum) >"$manifest_tmp"
  mv "$manifest_tmp" "$stage/SHA256SUMS"
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

printf 'not-a-full-commit\n' >"$stage/RELEASE_COMMIT"
refresh_manifest "$stage"
expect_rejected "$stage" "invalid release commit"
printf '0123456789abcdef0123456789abcdef01234567\n' >"$stage/RELEASE_COMMIT"
refresh_manifest "$stage"

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

# The root broker owns migration of persisted Nginx sites. Exercise its pure
# rewrite helper against both legacy and already-canonical redirect lines.
# shellcheck disable=SC1090
source "$BROKER"
running_status="$(render_status_json "running" "Installing payload" "2026-07-30T18:00:00-04:00" "null")"
success_status="$(render_status_json "success" "Installed payload" "2026-07-30T18:01:00-04:00" "0")"
python3 - "$running_status" "$success_status" <<'PY'
import json
import sys

running = json.loads(sys.argv[1])
success = json.loads(sys.argv[2])
assert running["status"] == "running"
assert running["exit_code"] is None
assert success["status"] == "success"
assert success["exit_code"] == 0
PY
expected_commit='0123456789abcdef0123456789abcdef01234567'
expected_ready_url="http://127.0.0.1:8080/readyz?expected_commit=$expected_commit"
actual_ready_url="$(target_ready_url 'http://127.0.0.1:8080/readyz?ignored=1' "$expected_commit")"
[[ "$actual_ready_url" == "$expected_ready_url" ]] || {
  printf 'target readiness URL does not bind the staged commit\n' >&2
  exit 1
}
nginx_source="$TMP_ROOT/nginx-source"
nginx_normalized="$TMP_ROOT/nginx-normalized"
cat >"$nginx_source" <<'EOF'
return 302 https://$host:8443/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1;
return 302 https://$host:8443/remote-next?transport=split&tx_opus=1&tx_cfc=1;
return 302 https://$host:8443/remote-next;
return 302 https://$host:8443/remote;
EOF
normalize_nginx_remote_redirects_file "$nginx_source" "$nginx_normalized"
expected_nginx="$TMP_ROOT/nginx-expected"
cat >"$expected_nginx" <<'EOF'
return 302 https://$host:8443/remote-next;
return 302 https://$host:8443/remote-next;
return 302 https://$host:8443/remote-next;
return 302 https://$host:8443/remote;
EOF
cmp "$expected_nginx" "$nginx_normalized"

if grep -Eq 'return 302 .*remote-next\?' "$INSTALLER"; then
  printf 'fresh installer still emits query-bearing remote-next redirects\n' >&2
  exit 1
fi

printf 'deploy broker tests passed\n'
