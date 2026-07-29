#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="$ROOT/update_manager/rust-server/src/main.rs"
PAGE="$ROOT/update_manager/templates/p23test.html"

grep -Fq 'fn xdma_bridge_telemetry(' "$SERVER"
grep -Fq '"xdma": xdma,' "$SERVER"
grep -Fq '"direct_operational": false' "$SERVER"
grep -Fq 'completion_kthread_priority' "$SERVER"
grep -Fq 'transfer_latency_warn_us' "$SERVER"
grep -Fq 'XDMA_TELEMETRY_SNAPSHOT_FILE' "$SERVER"

grep -Fq 'id="tab-xdma"' "$PAGE"
grep -Fq 'id="panel-xdma"' "$PAGE"
grep -Fq 'id="xdma-diag-dashboard"' "$PAGE"
grep -Fq 'function renderXdmaDashboard(xdma)' "$PAGE"
grep -Fq 'An inactive direct path is expected while' "$PAGE"
grep -Fq "showLabPanel('xdma')" "$PAGE"

echo "XDMA telemetry API and UI checks passed"
