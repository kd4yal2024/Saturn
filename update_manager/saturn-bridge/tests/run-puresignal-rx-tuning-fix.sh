#!/usr/bin/env bash
# OFFLINE ONLY: matched WDSP, actual runtime, loopback/file backend fixtures.
# No SSH, installed Bridge, radio device, firmware or driver operation.
set -euo pipefail
: "${SATURN_WDSP_DIR:?explicit reviewed x86 WDSP build required}"
: "${RUSTFLAGS:?explicit local test observer link required}"
: "${SATURN_BRIDGE_SOURCE_PIN:?explicit source-content pin required}"
: "${PS_TUNING_EVIDENCE_DIR:?new offline evidence directory required}"
if [[ ${SATURN_BRIDGE_STUB_NATIVE:-0} == 1 ]]; then
  printf 'Real WDSP required for this regression runner\n' >&2
  exit 1
fi
TUNING_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$TUNING_SCRIPT_DIR/.."
mkdir "$PS_TUNING_EVIDENCE_DIR"
sha256sum "$SATURN_WDSP_DIR/libwdsp.a" "$SATURN_WDSP_DIR/wdsp.h" \
  ../templates/saturn-remote-next.html ../remote-web/dist/saturn-remote-next.js \
  ../remote-web/src/transport/managed-pcm.ts ../remote-web/src/tci/apply.ts \
  ../remote-web/scripts/validate-puresignal-tab.mjs \
  > "$PS_TUNING_EVIDENCE_DIR/inputs.sha256"
rg --files src tests build.rs Cargo.toml Cargo.lock | LC_ALL=C sort | xargs sha256sum \
  >> "$PS_TUNING_EVIDENCE_DIR/inputs.sha256"
run_check() {
  local label="$1"
  shift
  if "$@" > "$PS_TUNING_EVIDENCE_DIR/$label.log" 2>&1; then
    printf 'PASS %s\n' "$label"
  else
    local status=$?
    printf 'FAIL %s exit=%s; log=%s\n' "$label" "$status" "$PS_TUNING_EVIDENCE_DIR/$label.log" >&2
    return "$status"
  fi
}
run_check lifecycle cargo test --offline --features managed-puresignal --lib -- --test-threads=1
run_check native-binary cargo test --offline --features managed-puresignal --bin saturn-bridge -- --test-threads=1
run_check web-typecheck npm --prefix ../remote-web run typecheck
run_check web-tests npm --prefix ../remote-web test -- --run
run_check browser-tuning cargo test --offline --features managed-puresignal --bin saturn-bridge \
  actual_run_inner_browser_measurement_both_backends -- --ignored --nocapture --test-threads=1
SATURN_RUNTIME_EVIDENCE_DIR="$PS_TUNING_EVIDENCE_DIR/runtime" \
  bash tests/run-puresignal-shared-runtime.sh
sha256sum --check --quiet "$PS_TUNING_EVIDENCE_DIR/inputs.sha256"
printf 'PASS offline RX tuning regressions; physical RF remains unqualified\n'
