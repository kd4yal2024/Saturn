#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$ROOT_DIR/update_manager/rust-server/src/main.rs"
STATE="$ROOT_DIR/update_manager/rust-server/src/state.rs"
RESTORE="$ROOT_DIR/update_manager/rust-server/src/restore.rs"
INSTALLER="$ROOT_DIR/update_manager/install_saturn_go_nginx.sh"

fail() {
  echo "request body limit test failed: $*" >&2
  exit 1
}

grep -Fq 'pub const JSON_REQUEST_MAX_BYTES: usize = 64 * 1024;' "$STATE" ||
  fail "ordinary request limit is not 64 KiB"
grep -Fq 'pub const CUSTOM_SCRIPT_REQUEST_MAX_BYTES: usize = 64 * 1024;' "$STATE" ||
  fail "custom-script request limit is not independent and 64 KiB"
grep -Fq 'let ordinary = with_request_limit(ordinary, JSON_REQUEST_MAX_BYTES);' "$MAIN" ||
  fail "ordinary router does not apply its request limit"
grep -Fq 'let custom = with_request_limit(custom, CUSTOM_SCRIPT_REQUEST_MAX_BYTES);' "$MAIN" ||
  fail "custom-script router does not apply its request limit"
grep -Fq 'let restore = with_request_limit(restore, restore_request_max_bytes);' "$MAIN" ||
  fail "restore router does not apply its configurable request limit"

if grep -Eq 'SATURN_MAX_BODY_BYTES' "$MAIN" "$INSTALLER"; then
  fail "retired global request-limit setting remains active"
fi

grep -Fq 'SATURN_NGINX_CLIENT_MAX_BODY_SIZE="${SATURN_NGINX_CLIENT_MAX_BODY_SIZE:-64k}"' "$INSTALLER" ||
  fail "nginx ordinary request limit is not 64 KiB"
grep -Fq 'location ~ ^/saturn/(restore_settings|restore_source|restore_full)$ {' "$INSTALLER" ||
  fail "nginx restore routes do not have a dedicated location"

restore_location="$(sed -n '/location ~ \^\/saturn\/(restore_settings|restore_source|restore_full)\$ {/,/^  }/p' "$INSTALLER")"
grep -Fq 'client_max_body_size ${SATURN_NGINX_RESTORE_MAX_BODY_SIZE};' <<<"$restore_location" ||
  fail "nginx restore location lacks its separate body limit"
grep -Fq 'proxy_request_buffering off;' <<<"$restore_location" ||
  fail "nginx restore location buffers upload bodies"

grep -Fq 'upload_disk_budget' "$RESTORE" ||
  fail "restore upload does not enforce a disk budget"
grep -Fq 'saturating_add(reserve_bytes)' "$RESTORE" ||
  fail "restore extraction does not preserve the readiness reserve"

echo "request body limit tests passed"
