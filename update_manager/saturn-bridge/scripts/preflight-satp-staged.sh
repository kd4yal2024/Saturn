#!/usr/bin/env bash
# Read-only preflight. This script has no install/restart/key-up mode.
set -Eeuo pipefail
if [[ $# != 1 || $1 != --check ]]; then
    echo "Usage: bash $0 --check (read-only; no sudo required)" >&2
    exit 2
fi
stage=$(cd -- "$(dirname -- "$0")" && pwd)
cd "$stage"
fail() { echo "Preflight FAILED: $*" >&2; exit 1; }
for tool in sha256sum file readelf ldd python3 systemctl pgrep df; do
    command -v "$tool" >/dev/null || fail "Missing tool: $tool"
done
[[ $(uname -m) == aarch64 ]] || fail 'This bundle requires ARM64 G2.'
test -s SHA256SUMS || fail 'Build/test has not completed; sealed manifest is missing.'
sha256sum --check --quiet SHA256SUMS || fail 'Staged bundle checksum mismatch.'
sha256sum --check --quiet SOURCE-SHA256SUMS || fail 'Staged source changed after build.'
sha256sum --check --quiet installed-before.sha256 || fail 'Installed bridge/UI changed since staging; re-stage before deploying.'
test -x saturn-bridge || fail 'Staged bridge is not executable.'
file saturn-bridge | grep -q 'ARM aarch64' || fail 'Staged bridge is not ARM64.'
readelf -h saturn-bridge >/dev/null || fail 'Staged bridge is not a valid ELF.'
dependencies=$(ldd ./saturn-bridge) || fail 'Cannot inspect shared libraries.'
[[ "$dependencies" != *'not found'* ]] || fail 'A required shared library is missing.'
[[ "$dependencies" == *libfftw3* ]] || fail 'Real WDSP/FFTW linkage is absent; do not deploy a native-stub test build.'
(cd update_manager/remote-web/dist && sha256sum --check --quiet saturn-remote-next.js.sha256) || fail 'Browser JS checksum mismatch.'
if systemctl is-active --quiet p2app.service || pgrep -x p2app >/dev/null; then
    fail 'P2/Thetis owns G2; close Thetis and select XDMA / TCI.'
fi
systemctl is-active --quiet saturn-bridge.service || fail 'Installed bridge is not active.'
pid=$(systemctl show saturn-bridge.service -p MainPID --value)
[[ "$pid" =~ ^[1-9][0-9]*$ ]] || fail 'Installed bridge has no main process.'
cmp /opt/saturn-go/bin/saturn-bridge "/proc/$pid/exe" || fail 'Running bridge differs from installed binary (or cannot be verified).'
python3 - <<'PY'
import json, time
with open('/run/saturn-bridge/xdma-ready.json') as f:
    state = json.load(f)
def require(ok, message):
    if not ok:
        raise SystemExit('Preflight FAILED: ' + message)
require(state.get('backend') == 'xdma', 'Bridge is not using XDMA.')
require(state.get('status') == 'ready', 'Bridge is not ready.')
require(0 <= time.time() * 1000 - state['updated_at_ms'] < 5000, 'Readiness is stale.')
metrics = state['metrics']
require(metrics.get('tx_keyed') is False, 'TX is keyed: release PTT/MOX.')
require(metrics.get('tx_stream_active') is False, 'TX stream is active.')
print('RX readiness: OK (XDMA, fresh, TX unkeyed and inactive)')
PY
# Allow room for the matched payload and an eventual rollback copy.
for directory in /opt/saturn-go /var/lib/saturn-web; do
    available=$(df -Pk "$directory" | awk 'NR == 2 {print $4}')
    [[ "$available" =~ ^[0-9]+$ && "$available" -ge 262144 ]] || fail "Less than 256 MiB free at $directory."
done
printf '%s\n' 'Bundle, installed baseline, architecture, and libraries: OK'
printf '%s\n' 'Preflight passed. Nothing installed, restarted, configured, or keyed.'
printf '%s\n' 'Windows client: updated SATP v2 ASIO build is required; this check does not verify Windows or live headphone/RF audio.'
