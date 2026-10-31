#!/usr/bin/env bash
# Offline only. No radio devices, SSH, installed services or RF commands.
set -euo pipefail
: "${PS_CAPTURE_EVIDENCE_DIR:?new absolute evidence directory required}"
[[ $PS_CAPTURE_EVIDENCE_DIR == /* ]]
CAPTURE_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$CAPTURE_SCRIPT_DIR/.."
mkdir "$PS_CAPTURE_EVIDENCE_DIR"
PS_SINGLE_EVIDENCE_DIR="$PS_CAPTURE_EVIDENCE_DIR/baseline" bash tests/run-puresignal-single-two-tone.sh
SATURN_BRIDGE_SOURCE_PIN="$(< "$PS_CAPTURE_EVIDENCE_DIR/baseline/baseline/source-content-pin.txt")"
export SATURN_BRIDGE_SOURCE_PIN
mkdir "$PS_CAPTURE_EVIDENCE_DIR/fixtures"
SATURN_SHARED_CAPTURE_FIXTURE_DIR="$PS_CAPTURE_EVIDENCE_DIR/fixtures" \
  cargo test --offline --features managed-puresignal --bin saturn-bridge \
  measurement_only_shared_signal_capture_actual -- --ignored --nocapture --test-threads=1 \
  > "$PS_CAPTURE_EVIDENCE_DIR/capture-native.log" 2>&1
python3 -m unittest discover -s tests -p test_shared_signal_review.py \
  > "$PS_CAPTURE_EVIDENCE_DIR/capture-python.log" 2>&1
for capture_backend in DirectXdma P2; do
  python3 scripts/review-shared-signal.py "$PS_CAPTURE_EVIDENCE_DIR/fixtures/$capture_backend" \
    --backend "$capture_backend" > "$PS_CAPTURE_EVIDENCE_DIR/$capture_backend-review.json"
done
sha256sum --check --quiet "$PS_CAPTURE_EVIDENCE_DIR/baseline/baseline/source.sha256"
printf 'PASS shared capture: native/acquisition/output boundaries, raw hashes and negative evidence checks; no RF qualification\n'
