#!/usr/bin/env bash
# OFFLINE only. Actual UI, native WDSP, loopback/file owners; no RF/G2.
set -euo pipefail
PCM_UI_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PCM_UI_WDSP="${1:?usage: bash tests/run-puresignal-managed-pcm-ui.sh VERIFIED_X86_WDSP_BUILD_DIR}"
: "${PS_REMOTE_TEMPLATE:?actual Saturn Remote template required}"
: "${PS_REMOTE_BUNDLE:?built actual Saturn Remote bundle required}"
: "${PS_BROWSER_PYTHON:?explicit isolated Python interpreter required}"
: "${PS_BROWSER_RUNNER:?experiments/run_managed_pcm_ui.py required}"
: "${PS_BROWSER_SPEECH:?local speech WAV required}"
: "${PS_BROWSER_RESULTS:?fresh results prefix required}"
bash "$PCM_UI_TEST_DIR/run-puresignal-owner-native.sh" "$PCM_UI_WDSP" \
  browser_managed_pcm_ui_both_backends --ignored
