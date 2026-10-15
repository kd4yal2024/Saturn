#!/usr/bin/env bash
# OFFLINE ONLY. Actual unmodified Saturn Go TLS/proxy + loopback/file owners.
# Never launch an installed Bridge, contact G2, alter startup or open hardware.
set -euo pipefail
REAL_PROXY_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REAL_PROXY_WDSP="${1:?usage: bash tests/run-puresignal-real-proxy.sh VERIFIED_X86_WDSP_BUILD_DIR}"
: "${PS_SATURN_GO_BIN:?explicit locally built Saturn Go executable required}"
test -x "$PS_SATURN_GO_BIN"
export PS_UI_PREQUEUE_REJECTION=1
bash "$REAL_PROXY_TEST_DIR/run-puresignal-managed-pcm-ui.sh" "$REAL_PROXY_WDSP"
