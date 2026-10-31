#!/usr/bin/env bash
# OFFLINE ONLY. No SSH, radio devices, deployed process or RF operation.
set -euo pipefail
: "${SATURN_WDSP_DIR:?explicit reviewed local x86 WDSP build required}"
: "${RUSTFLAGS:?explicit offline test observer link required}"
: "${PS_TIMING_UI_EVIDENCE_DIR:?new absolute evidence directory required}"
[[ $PS_TIMING_UI_EVIDENCE_DIR == /* && ${SATURN_BRIDGE_STUB_NATIVE:-0} != 1 ]]
TIMING_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$TIMING_SCRIPT_DIR/.."
mkdir "$PS_TIMING_UI_EVIDENCE_DIR"
rg --files src tests scripts build.rs Cargo.toml Cargo.lock \
  ../remote-web/src ../remote-web/tests ../remote-web/scripts \
  ../remote-web/package.json ../remote-web/package-lock.json \
  ../templates/saturn-remote-next.html | LC_ALL=C sort | xargs sha256sum \
  > "$PS_TIMING_UI_EVIDENCE_DIR/source.sha256"
SATURN_BRIDGE_SOURCE_PIN="$(sha256sum "$PS_TIMING_UI_EVIDENCE_DIR/source.sha256" | cut -d ' ' -f 1)"
export SATURN_BRIDGE_SOURCE_PIN
printf '%s\n' "$SATURN_BRIDGE_SOURCE_PIN" > "$PS_TIMING_UI_EVIDENCE_DIR/source-content-pin.txt"
sha256sum "$SATURN_WDSP_DIR/libwdsp.a" "$SATURN_WDSP_DIR/wdsp.h" \
  > "$PS_TIMING_UI_EVIDENCE_DIR/native-inputs.sha256"
run_check() {
  local label="$1"
  shift
  if "$@" > "$PS_TIMING_UI_EVIDENCE_DIR/$label.log" 2>&1; then
    printf 'PASS %s\n' "$label"
  else
    local status=$?
    printf 'FAIL %s exit=%s; preserved log=%s\n' "$label" "$status" "$PS_TIMING_UI_EVIDENCE_DIR/$label.log" >&2
    return "$status"
  fi
}
# Real WDSP uses process-global state: serialize native tests, and do not run
# another browser/native replay concurrently with this evidence run.
run_check lifecycle cargo test --offline --features managed-puresignal --lib -- --test-threads=1
run_check native-binary cargo test --offline --features managed-puresignal --bin saturn-bridge -- --test-threads=1
run_check web-typecheck npm --prefix ../remote-web run typecheck
run_check web-build npm --prefix ../remote-web run build
run_check web-tests npm --prefix ../remote-web test
run_check timing-pcm npm --prefix ../remote-web exec -- vite-node \
  ../remote-web/scripts/generate-ps-timing.mts "$PS_TIMING_UI_EVIDENCE_DIR/pcm"
run_check native-timing env SATURN_PS_TIMING_PCM="$PS_TIMING_UI_EVIDENCE_DIR/pcm/timing.pcm-f32le" \
  SATURN_PS_TIMING_OUTPUT_DIR="$PS_TIMING_UI_EVIDENCE_DIR/native-output" \
  cargo test --offline --features managed-puresignal --bin saturn-bridge full_native_timing_ \
  -- --ignored --nocapture --test-threads=1
run_check python-timing env SATURN_PS_TIMING_NATIVE_IQ="$PS_TIMING_UI_EVIDENCE_DIR/native-output/wdsp-output-iq.f32le" \
  python3 -m unittest discover -s tests -p test_ps_timing_review.py
run_check browser-timing cargo test --offline --features managed-puresignal --bin saturn-bridge \
  actual_run_inner_browser_timing_both_backends -- --ignored --nocapture --test-threads=1
run_check browser-tab node ../remote-web/scripts/validate-puresignal-tab.mjs
run_check monitor-fixture env SATURN_RF_MONITOR_FIXTURE_DIR="$PS_TIMING_UI_EVIDENCE_DIR/rf-fixture" \
  cargo test --offline --features managed-puresignal --bin saturn-bridge \
  preallocated_observer_retains_raw_pair_identity_oldest_time_and_unknown_overload -- --test-threads=1
run_check browser-monitor node ../remote-web/scripts/validate-rf-monitor.mjs \
  "$PS_TIMING_UI_EVIDENCE_DIR/rf-fixture/synthetic-rf-spectrum.txt"
sha256sum --check --quiet "$PS_TIMING_UI_EVIDENCE_DIR/source.sha256"
printf 'PASS offline Timing Measurement UI; no G2 access or physical RF qualification\n'
