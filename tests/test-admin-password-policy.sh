#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="$REPO_ROOT/scripts/saturn-admin-password.sh"
if ! sudo -n true >/dev/null 2>&1; then
  if [[ "${CI:-}" == true ]]; then
    printf 'password-policy test requires passwordless sudo in CI\n' >&2
    exit 1
  fi
  printf 'SKIP: password-policy test requires passwordless sudo\n'
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
status_output="$(sudo -n env \
  SATURN_ADMIN_HTPASSWD_FILE="$TMP_DIR/htpasswd" \
  SATURN_ADMIN_DROPIN_DIR="$TMP_DIR/dropin" \
  "$HELPER" status)"
grep -q '^sync_state=in_sync$' <<<"$status_output"

if run_helper ab12 >/dev/null 2>&1; then
  printf 'four-character password was incorrectly accepted\n' >&2
  exit 1
fi
if run_helper abc123 >/dev/null 2>&1; then
  printf 'six-character password was incorrectly accepted\n' >&2
  exit 1
fi

printf 'admin password policy tests passed\n'
