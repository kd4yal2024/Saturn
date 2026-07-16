#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="$REPO_ROOT/scripts/saturn-p2-deploy.sh"
TEST_USER="$(id -un)"
if ! sudo -n true >/dev/null 2>&1; then
  if [[ "${CI:-}" == true ]]; then
    printf 'P2 deployment broker test requires passwordless sudo in CI\n' >&2
    exit 1
  fi
  printf 'SKIP: P2 deployment broker test requires passwordless sudo\n'
  exit 0
fi
TMP_DIR="$(mktemp -d)"
trap 'sudo -n rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR/bin" "$TMP_DIR/runtime"

if "$HELPER" unexpected-argument >/dev/null 2>&1; then
  printf 'P2 deployment broker incorrectly accepted an argument\n' >&2
  exit 1
fi

cat >"$TMP_DIR/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  is-active)
    [[ -f "$MOCK_STATE/active" ]]
    ;;
  stop)
    rm -f "$MOCK_STATE/active"
    ;;
  start)
    [[ "${MOCK_FAIL_START:-0}" != 1 ]] || exit 1
    : >"$MOCK_STATE/active"
    ;;
  *) exit 0 ;;
esac
EOF
chmod 0755 "$TMP_DIR/bin/systemctl"

cp /bin/true "$TMP_DIR/p2app"
chmod 0755 "$TMP_DIR/p2app"
cat >"$TMP_DIR/config" <<EOF
P2APP_SOURCE_BIN='$TMP_DIR/p2app'
P2APP_RUNTIME_BIN='$TMP_DIR/runtime/p2app'
P2APP_SERVICE='p2app.service'
P2APP_BUILD_USER='$TEST_USER'
P2APP_START_TIMEOUT_SECONDS=3
P2APP_STABLE_SECONDS=2
EOF
chmod 0644 "$TMP_DIR/config"
sudo -n chown root:root "$TMP_DIR/config"

run_deploy(){
  sudo -n env \
    PATH="$TMP_DIR/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    MOCK_STATE="$TMP_DIR" \
    MOCK_FAIL_START="${1:-0}" \
    SATURN_P2_DEPLOY_CONFIG="$TMP_DIR/config" \
    "$HELPER"
}

run_deploy 0 >/dev/null
cmp /bin/true "$TMP_DIR/runtime/p2app"

cp /bin/false "$TMP_DIR/p2app"
chmod 0755 "$TMP_DIR/p2app"
if run_deploy 1 >/dev/null 2>&1; then
  printf 'failed P2 start was incorrectly accepted\n' >&2
  exit 1
fi
cmp /bin/true "$TMP_DIR/runtime/p2app"

printf 'P2 deployment broker tests passed\n'
