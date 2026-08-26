#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="$REPO_ROOT/update_manager/scripts/saturn-fftw-wisdom.sh"
INSTALLER="$REPO_ROOT/update_manager/scripts/install-saturn-bridge.sh"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT

fail() {
  printf 'Saturn FFTW wisdom test failed: %s\n' "$*" >&2
  exit 1
}

mkdir -p "$TEST_ROOT/bin" "$TEST_ROOT/cache" "$TEST_ROOT/run"
cat >"$TEST_ROOT/cpuinfo" <<'EOF'
model name : Saturn test CPU
Features : neon test
EOF

cat >"$TEST_ROOT/bin/saturn-bridge" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
case "${1:-}" in
  --generate-fftw-wisdom)
    printf '(fftw-3.3.10 saturn-test-wisdom)\n' >"$2"
    count_file="${SATURN_TEST_GENERATION_COUNT:?}"
    count=0
    [[ ! -r "$count_file" ]] || count="$(cat "$count_file")"
    printf '%s\n' "$((count + 1))" >"$count_file"
    ;;
  --validate-fftw-wisdom)
    grep -Fq 'saturn-test-wisdom' "$2"
    ;;
  *) exit 64 ;;
esac
EOF
chmod 0755 "$TEST_ROOT/bin/saturn-bridge"

export SATURN_FFTW_WISDOM_BRIDGE_BIN="$TEST_ROOT/bin/saturn-bridge"
export SATURN_FFTW_WISDOM_CACHE_DIR="$TEST_ROOT/cache"
export SATURN_FFTW_WISDOM_CPUINFO_FILE="$TEST_ROOT/cpuinfo"
export SATURN_BRIDGE_FFTW_WISDOM_MAX_SIZE=64
export SATURN_TEST_GENERATION_COUNT="$TEST_ROOT/generation-count"

bash -n "$HELPER"
grep -Fq 'install_fftw_wisdom_maintenance' "$INSTALLER" \
  || fail "bridge installer does not install wisdom maintenance"
grep -Fq 'Persistent=true' "$INSTALLER" \
  || fail "wisdom timer is not persistent"
grep -Fq 'RandomizedDelaySec=45m' "$INSTALLER" \
  || fail "wisdom timer lacks randomized scheduling"
grep -Fq 'SATURN_BRIDGE_FFTW_WISDOM_PATH=' "$INSTALLER" \
  || fail "bridge service does not import installed wisdom"
grep -Fq 'Nice=19' "$INSTALLER" \
  || fail "wisdom service is not low priority"
grep -Fq 'ConditionFileIsExecutable=' "$INSTALLER" \
  || fail "wisdom service does not use the supported executable condition"
if grep -Fq 'ConditionPathIsExecutable=' "$INSTALLER"; then
  fail "wisdom service uses the invalid ConditionPathIsExecutable directive"
fi
bash "$HELPER" --check
[[ "$(cat "$SATURN_TEST_GENERATION_COUNT")" == "1" ]] || fail "first check did not generate"
[[ -s "$TEST_ROOT/cache/wdspWisdom01" ]] || fail "wisdom file missing"
[[ -s "$TEST_ROOT/cache/fingerprint.sha256" ]] || fail "fingerprint missing"
[[ -f "$TEST_ROOT/cache/.wisdom.lock" ]] || fail "default lock was not created in the writable cache"

bash "$HELPER" --check
[[ "$(cat "$SATURN_TEST_GENERATION_COUNT")" == "1" ]] || fail "fresh cache regenerated"
bash "$HELPER" --status | grep -Fq 'state=fresh' || fail "fresh status not reported"

printf '\n# changed bridge identity\n' >>"$TEST_ROOT/bin/saturn-bridge"
bash "$HELPER" --check
[[ "$(cat "$SATURN_TEST_GENERATION_COUNT")" == "2" ]] || fail "changed fingerprint did not regenerate"

printf 'corrupt\n' >"$TEST_ROOT/cache/wdspWisdom01"
bash "$HELPER" --check
[[ "$(cat "$SATURN_TEST_GENERATION_COUNT")" == "3" ]] || fail "invalid wisdom did not regenerate"

bash "$HELPER" --rebuild
[[ "$(cat "$SATURN_TEST_GENERATION_COUNT")" == "4" ]] || fail "forced rebuild did not generate"

printf 'Saturn FFTW wisdom test passed\n'
