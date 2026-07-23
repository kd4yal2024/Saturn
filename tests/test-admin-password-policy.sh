#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="$REPO_ROOT/scripts/saturn-admin-password.sh"
REMOTE_TLS="$REPO_ROOT/update_manager/rust-server/src/remote_tls.rs"
AUTH_RS="$REPO_ROOT/update_manager/rust-server/src/auth.rs"
RUNBOOK="$REPO_ROOT/update_manager/docs/OPERATIONS_RUNBOOK.md"

require_text(){
  local expected="$1"
  local path="$2"
  local description="$3"
  if ! grep -Fq "$expected" "$path"; then
    printf 'missing %s contract in %s\n' "$description" "$path" >&2
    exit 1
  fi
}

require_text \
  'const REMOTE_AUTH_COOKIE_MAX_AGE_SECS: u64 = 365 * 24 * 60 * 60;' \
  "$REMOTE_TLS" \
  'one-year remembered-login'
require_text \
  'fn password_change_invalidates_existing_cookie_token()' \
  "$REMOTE_TLS" \
  'password-change cookie invalidation test'
require_text \
  'local restart_mode="deferred"' \
  "$HELPER" \
  'default deferred Saturn Go restart'
require_text \
  "systemd-run --collect --on-active=2 systemctl try-restart \"\$SERVICE_NAME\"" \
  "$HELPER" \
  'deferred Saturn Go restart'
require_text \
  'All remembered-device logins will be signed out' \
  "$AUTH_RS" \
  'password-change invalidation message'
require_text \
  '**Shared-device risk:**' \
  "$RUNBOOK" \
  'shared-device operator warning'

validate_password(){
  local password="$1"
  bash -c 'source "$1"; validate_password "$2"' _ "$HELPER" "$password"
}

validate_password abc12
validate_password abc123
if validate_password ab12 >/dev/null 2>&1; then
  printf 'four-character password was incorrectly accepted\n' >&2
  exit 1
fi
if validate_password $'abc12\n' >/dev/null 2>&1; then
  printf 'password containing a control character was incorrectly accepted\n' >&2
  exit 1
fi

if ! sudo -n true >/dev/null 2>&1; then
  if [[ "${CI:-}" == true ]]; then
    printf 'password-policy test requires passwordless sudo in CI\n' >&2
    exit 1
  fi
  printf 'password validation tests passed\n'
  printf 'SKIP: backend integration requires passwordless sudo\n'
  exit 0
fi
TMP_DIR="$(mktemp -d)"
trap 'sudo -n rm -rf "$TMP_DIR"' EXIT

run_helper(){
  local password="$1"
  printf '%s\n' "$password" | sudo -n env \
    SATURN_ADMIN_SKIP_SYSTEMD=1 \
    SATURN_ADMIN_HTPASSWD_FILE="$TMP_DIR/htpasswd" \
    SATURN_ADMIN_HTPASSWD_OWNER=root:root \
    SATURN_ADMIN_DROPIN_DIR="$TMP_DIR/dropin" \
    SATURN_ADMIN_INITIAL_LOGIN_FILE="$TMP_DIR/initial-login.txt" \
    "$HELPER" set --restart none
}

run_helper abc12 >/dev/null
printf '%s\n' abc12 | sudo -n htpasswd -vi "$TMP_DIR/htpasswd" admin >/dev/null

run_helper abc123 >/dev/null
printf '%s\n' abc123 | sudo -n htpasswd -vi "$TMP_DIR/htpasswd" admin >/dev/null

status_output="$(sudo -n env \
  SATURN_ADMIN_HTPASSWD_FILE="$TMP_DIR/htpasswd" \
  SATURN_ADMIN_DROPIN_DIR="$TMP_DIR/dropin" \
  "$HELPER" status)"
grep -q '^sync_state=in_sync$' <<<"$status_output"

if run_helper ab12 >/dev/null 2>&1; then
  printf 'four-character password was incorrectly accepted\n' >&2
  exit 1
fi
printf 'admin password policy tests passed\n'
