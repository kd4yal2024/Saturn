#!/usr/bin/env bash
# Offline only: real Chrome, loopback P2 and regular-file Direct XDMA fixtures.
# No radio discovery, device access, deployment, RF or managed Opus experiment.
set -euo pipefail
CANCEL_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CANCEL_WDSP="${1:?usage: bash tests/run-puresignal-cancellation-browser.sh VERIFIED_WDSP_BUILD_DIR}"
: "${PS_BROWSER_PYTHON:?explicit Python interpreter required}"
: "${PS_BROWSER_RUNNER:?path to remote-web/experiments/run_ps_browser.py required}"
: "${PS_BROWSER_SPEECH:?local recorded speech WAV required}"
: "${PS_BROWSER_RESULTS:?new output prefix required; retain previous evidence}"
bash "$CANCEL_TEST_DIR/run-puresignal-owner-native.sh" "$CANCEL_WDSP" \
  browser_cancellation_pcm_lifecycle_both_backends --ignored
