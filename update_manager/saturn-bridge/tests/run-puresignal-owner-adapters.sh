#!/usr/bin/env bash
# Offline tests only. Explicit loopback UDP and regular-file register/DMA fixtures.
# Native FFI is stubbed here; real WDSP is tested by the separate native runner.
set -euo pipefail
OWNER_BRIDGE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v cargo >/dev/null
cd "$OWNER_BRIDGE"
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --offline --features offline-puresignal-owner-adapters
