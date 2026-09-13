#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PAGE="${ROOT}/update_manager/templates/p23test.html"
VERSION_SCRIPT="${ROOT}/update_manager/scripts/g2-version-info.sh"

grep -Fq '#define P2APPVERSION 51' "${ROOT}/sw_projects/P2_app/p2app.c"
grep -Fq 'SCRIPT_VERSION="1.7"' "${VERSION_SCRIPT}"
make -C "${ROOT}/sw_projects/P2_app" test-fpga-fifo-v29

node - "${PAGE}" <<'JS'
const assert = require('assert');
const fs = require('fs');
const vm = require('vm');
const source = fs.readFileSync(process.argv[2], 'utf8');
const begin = source.indexOf('/* FPGA_FIFO_V29_PRESENTATION_BEGIN */');
const end = source.indexOf('/* FPGA_FIFO_V29_PRESENTATION_END */');
assert(begin >= 0 && end > begin, 'V29 presentation helper markers missing');
const context = {};
vm.createContext(context);
vm.runInContext(source.slice(begin, end), context);

assert.strictEqual(context.fpgaFifoV29Presentation(undefined, 28).state, 'unsupported');
assert.strictEqual(context.fpgaFifoV29Presentation(undefined, 29).state, 'unavailable');
const mismatch = context.fpgaFifoV29Presentation({
  available: false,
  status: 'marker_mismatch',
  build_id: 0x56323800,
}, 29);
assert.strictEqual(mismatch.state, 'marker_mismatch');
assert(mismatch.detail.includes('0x56323800'));
const available = context.fpgaFifoV29Presentation({
  available: true,
  status: 'available',
  snapshot_valid: true,
  snapshot_generation: 7,
  snapshot_timeout_count: 2,
  occupancy_words: {ddc: 1, duc: 2, mic: 3, speaker: 4},
  minimum_words: {ddc: 0, duc: 0, mic: 0, speaker: 0},
  maximum_words: {ddc: 11, duc: 12, mic: 13, speaker: 14},
  event_transitions: {ddc: 21, duc: 22, mic: 23, speaker: 24},
}, 29);
assert.strictEqual(available.state, 'available');
assert(available.detail.includes('occupancy words: ddc 1'));
assert(available.detail.includes('event transitions: ddc 21'));
assert(source.includes('fpga_fifo_v29: jsonSafe('), 'captured export omits V29 telemetry');
console.log('Web Manager V29 compatibility tests passed');
JS

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "${TEST_TMP}"' EXIT
mkdir -p "${TEST_TMP}/bin" "${TEST_TMP}/repo/scripts"

cat > "${TEST_TMP}/perf.json" <<'JSON'
{"perf":{"app_telemetry":{"current":{"fpga":{"available":true,"firmware_version":29,"product_version":3,"date_code_hex":"09122026"}}}}}
JSON
cat > "${TEST_TMP}/bin/curl" <<'SH'
#!/usr/bin/env bash
while (($#)); do
  if [[ "$1" == "-o" ]]; then cp "${SATURN_TEST_FIXTURE}" "$2"; exit 0; fi
  shift
done
exit 1
SH
cat > "${TEST_TMP}/bin/systemctl" <<'SH'
#!/usr/bin/env bash
printf '%s\n' 'Sat 2026-09-12 23:00:00 EDT'
SH
cat > "${TEST_TMP}/bin/journalctl" <<'SH'
#!/usr/bin/env bash
if [[ " $* " == *" --since "* ]]; then exit 0; fi
printf '%s\n' \
  'FPGA BIT file data code = 09122026' \
  ' Product: Saturn; Version = 3' \
  ' FPGA Firmware loaded: Saturn, full function; FW Version = 28, major version = 0' \
  'All clocks present'
SH
cat > "${TEST_TMP}/repo/scripts/detect-front-panel.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' off
SH
cat > "${TEST_TMP}/repo/scripts/detect-lcd-profile.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH
chmod +x "${TEST_TMP}/bin/curl" "${TEST_TMP}/bin/systemctl" "${TEST_TMP}/bin/journalctl" \
  "${TEST_TMP}/repo/scripts/detect-front-panel.sh" "${TEST_TMP}/repo/scripts/detect-lcd-profile.sh"

SATURN_TEST_FIXTURE="${TEST_TMP}/perf.json" \
SATURN_ACTIVE_REPO_ROOT="${TEST_TMP}/repo" \
PATH="${TEST_TMP}/bin:${PATH}" \
bash "${VERSION_SCRIPT}" > "${TEST_TMP}/version-output.txt"

grep -Fq 'STALE retained startup banner: firmware_version retained=28 live=29' "${TEST_TMP}/version-output.txt"
grep -Fq 'Historical only; this banner is not current FPGA identity evidence' "${TEST_TMP}/version-output.txt"
grep -Fq '[stale]  FPGA Firmware loaded:' "${TEST_TMP}/version-output.txt"

bash -n "${VERSION_SCRIPT}"
echo "g2-version-info stale-banner tests passed"
