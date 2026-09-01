#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
HELPER="$SCRIPT_DIR/saturn-tci-bind.sh"
TEST_ROOT="$(mktemp -d /tmp/saturn-tci-bind.XXXXXX)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT

mkdir -p "$TEST_ROOT/bin" "$TEST_ROOT/systemd" "$TEST_ROOT/state"

cat >"$TEST_ROOT/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  is-active)
    [[ -f "$MOCK_TCI_STATE/bridge-active" ]]
    ;;
  daemon-reload)
    ;;
  show)
    dropin="$MOCK_TCI_SYSTEMD_ROOT/saturn-bridge.service.d/tci-bind.conf"
    [[ -f "$dropin" ]] && sed -n 's/^Environment=//p' "$dropin" | paste -sd' ' -
    ;;
  *)
    printf 'unexpected mock systemctl command: %s\n' "$*" >&2
    exit 2
    ;;
esac
EOF
chmod 0755 "$TEST_ROOT/bin/systemctl"

cat >"$TEST_ROOT/backend-helper" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$MOCK_TCI_BACKEND_LOG"
[[ "${MOCK_BACKEND_FAIL:-0}" != "1" ]]
EOF
chmod 0755 "$TEST_ROOT/backend-helper"

run_helper() {
  env \
    PATH="$TEST_ROOT/bin:$PATH" \
    MOCK_TCI_STATE="$TEST_ROOT/state" \
    MOCK_TCI_SYSTEMD_ROOT="$TEST_ROOT/systemd" \
    MOCK_TCI_BACKEND_LOG="$TEST_ROOT/backend.log" \
    MOCK_BACKEND_FAIL="${MOCK_BACKEND_FAIL:-0}" \
    SATURN_TCI_BIND_TEST_MODE=1 \
    SATURN_TCI_BIND_SYSTEMD_ROOT="$TEST_ROOT/systemd" \
    SATURN_TCI_BIND_BACKEND_HELPER="$TEST_ROOT/backend-helper" \
    "$HELPER" "$@"
}

inactive_output="$(run_helper set 0.0.0.0 50001)"
[[ "$inactive_output" == *"remains inactive"* ]]
[[ ! -s "$TEST_ROOT/backend.log" ]]
grep -Fqx 'Environment=SATURN_BRIDGE_TCI_HOST=0.0.0.0' \
  "$TEST_ROOT/systemd/saturn-bridge.service.d/tci-bind.conf"

touch "$TEST_ROOT/state/bridge-active"
active_output="$(run_helper set 192.168.1.10 50002)"
[[ "$active_output" == *"ownership broker"* ]]
grep -Fqx 'restart bridge' "$TEST_ROOT/backend.log"
cp "$TEST_ROOT/systemd/saturn-bridge.service.d/tci-bind.conf" "$TEST_ROOT/known-good.conf"

MOCK_BACKEND_FAIL=1
export MOCK_BACKEND_FAIL
if run_helper set 192.168.1.11 50003 >/dev/null 2>&1; then
  printf 'expected failed broker restart to reject the TCI bind update\n' >&2
  exit 1
fi
cmp "$TEST_ROOT/known-good.conf" \
  "$TEST_ROOT/systemd/saturn-bridge.service.d/tci-bind.conf"

if run_helper set 999.1.1.1 50001 >/dev/null 2>&1; then
  printf 'expected invalid IPv4 address to be rejected\n' >&2
  exit 1
fi

printf 'TCI bind helper test passed\n'
