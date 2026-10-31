#!/usr/bin/env bash
# OFFLINE ONLY: no SSH, service management, devices, firmware or RF.
set -euo pipefail
: "${PS_SINGLE_EVIDENCE_DIR:?new absolute directory required}"
[[ $PS_SINGLE_EVIDENCE_DIR == /* ]]
SINGLE_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SINGLE_SCRIPT_DIR/.."
mkdir "$PS_SINGLE_EVIDENCE_DIR"
# Includes source pinning, the complete serial native/non-native suites,
# browser baseline through actual run_inner, UI and timing-review regressions.
PS_TIMING_UI_EVIDENCE_DIR="$PS_SINGLE_EVIDENCE_DIR/baseline" \
  bash tests/run-puresignal-timing-ui.sh
SATURN_BRIDGE_SOURCE_PIN="$(< "$PS_SINGLE_EVIDENCE_DIR/baseline/source-content-pin.txt")"
export SATURN_BRIDGE_SOURCE_PIN
if cargo test --offline --features managed-puresignal --bin saturn-bridge generated_ \
    -- --include-ignored --test-threads=1 > "$PS_SINGLE_EVIDENCE_DIR/generated-trials.log" 2>&1; then
  printf 'PASS generated trials, real native gain/control readback, websocket dispatch and packed-tone regression\n'
else
  SINGLE_STATUS=$?
  printf 'FAIL generated trials exit=%s; see generated-trials.log\n' "$SINGLE_STATUS" >&2
  exit "$SINGLE_STATUS"
fi
sha256sum --check --quiet "$PS_SINGLE_EVIDENCE_DIR/baseline/source.sha256"
printf 'PASS offline single-fit two-tone path; not a live calibration or RF qualification\n'
