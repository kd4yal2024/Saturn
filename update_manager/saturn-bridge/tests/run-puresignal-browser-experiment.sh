#!/usr/bin/env bash
# Explicit offline experiment only. No installed services, devices or G2 access.
set -euo pipefail
BROWSER_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${PS_BROWSER_PYTHON:?supply Python with isolated experiment dependencies}"
: "${PS_BROWSER_SPEECH:?supply local recorded speech WAV}"
: "${PS_BROWSER_RESULTS:?supply new result directory prefix}"
export PS_BROWSER_RUNNER="$BROWSER_TEST_DIR/../../remote-web/experiments/run_ps_browser.py"
"$PS_BROWSER_PYTHON" "$PS_BROWSER_RUNNER" --speech "$PS_BROWSER_SPEECH" \
  --output "$PS_BROWSER_RESULTS-comparison"
export PS_BROWSER_CORPUS="$PS_BROWSER_RESULTS-comparison"
bash "$BROWSER_TEST_DIR/run-puresignal-owner-native.sh" "${1:?supply verified offline WDSP build directory}" \
  browser_experiment_actual_bridge_decoder_corpus --ignored
bash "$BROWSER_TEST_DIR/run-puresignal-owner-native.sh" "$1" \
  browser_experiment_actual_ingress_native_both_backends --ignored
