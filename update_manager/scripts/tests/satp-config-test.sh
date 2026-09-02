#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
HELPER="$SCRIPT_DIR/saturn-satp-config.sh"
TEST_ROOT="$(mktemp -d /tmp/saturn-satp-config.XXXXXX)"
trap 'rm -rf -- "$TEST_ROOT"' EXIT
mkdir -p "$TEST_ROOT/bin" "$TEST_ROOT/systemd" "$TEST_ROOT/state"

cat >"$TEST_ROOT/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  is-active) [[ -f "$MOCK_SATP_STATE/bridge-active" ]] ;;
  daemon-reload) ;;
  show)
    dropin="$MOCK_SATP_SYSTEMD_ROOT/saturn-bridge.service.d/satp.conf"
    [[ -f "$dropin" ]] && sed -n 's/^Environment=//p' "$dropin" | paste -sd' ' -
    ;;
  *) exit 2 ;;
esac
EOF
chmod 0755 "$TEST_ROOT/bin/systemctl"

cat >"$TEST_ROOT/backend-helper" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$MOCK_SATP_BACKEND_LOG"
[[ "${MOCK_BACKEND_FAIL:-0}" != "1" ]]
EOF
chmod 0755 "$TEST_ROOT/backend-helper"

run_helper() {
  env PATH="$TEST_ROOT/bin:$PATH" \
    MOCK_SATP_STATE="$TEST_ROOT/state" \
    MOCK_SATP_SYSTEMD_ROOT="$TEST_ROOT/systemd" \
    MOCK_SATP_BACKEND_LOG="$TEST_ROOT/backend.log" \
    MOCK_BACKEND_FAIL="${MOCK_BACKEND_FAIL:-0}" \
    SATURN_SATP_CONFIG_TEST_MODE=1 \
    SATURN_SATP_CONFIG_SYSTEMD_ROOT="$TEST_ROOT/systemd" \
    SATURN_SATP_CONFIG_BACKEND_HELPER="$TEST_ROOT/backend-helper" \
    "$HELPER" "$@"
}

run_helper set 1 192.168.1.50 50100 192.168.1.20 512 4096 250 satp | grep -q 'remains inactive'
dropin="$TEST_ROOT/systemd/saturn-bridge.service.d/satp.conf"
grep -Fqx 'Environment=SATURN_BRIDGE_SATP_ENABLED=1' "$dropin"
grep -Fqx 'Environment=SATURN_BRIDGE_SATP_ALLOWED_SOURCE_IP=192.168.1.20' "$dropin"
grep -Fqx 'Environment=SATURN_BRIDGE_TX_AUDIO_SOURCE=satp' "$dropin"

if run_helper set 0 192.168.1.50 50100 - 512 4096 250 satp >/dev/null 2>&1; then
  echo 'expected disabled SATP receiver with SATP TX source to fail' >&2
  exit 1
fi

touch "$TEST_ROOT/state/bridge-active"
run_helper set 0 127.0.0.1 50101 - 512 4096 250 tci | grep -q 'ownership broker'
grep -Fqx 'restart bridge' "$TEST_ROOT/backend.log"
cp "$dropin" "$TEST_ROOT/known-good.conf"

MOCK_BACKEND_FAIL=1
export MOCK_BACKEND_FAIL
if run_helper set 1 192.168.1.51 50102 - 512 4096 250 satp >/dev/null 2>&1; then
  echo 'expected failed restart to reject SATP configuration' >&2
  exit 1
fi
cmp "$TEST_ROOT/known-good.conf" "$dropin"

if run_helper set 1 192.168.1.50 50100 - 500 4096 250 >/dev/null 2>&1; then
  echo 'expected non-packet-aligned jitter target to fail' >&2
  exit 1
fi

echo 'SATP config helper test passed'
