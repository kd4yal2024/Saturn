#!/usr/bin/env bash
# Actual run_inner orchestration, with loopback/file endpoints, never hardware.
set -euo pipefail
: "${SATURN_WDSP_DIR:?explicit pinned local real WDSP archive required}"
: "${SATURN_RUNTIME_EVIDENCE_DIR:?explicit new evidence directory required}"
: "${RUSTFLAGS:?explicit local replay observer link flags required}"
if [[ ${SATURN_BRIDGE_STUB_NATIVE:-0} == 1 ]]; then
  printf 'Real matched native is required\n' >&2
  exit 1
fi
RUNTIME_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$RUNTIME_SCRIPT_DIR/.."
mkdir "$SATURN_RUNTIME_EVIDENCE_DIR"
for RUNTIME_TEST in \
  actual_run_inner_receive_release_rearm_active_shutdown_both_backends \
  actual_run_inner_preserves_receive_settings_both_backends \
  actual_run_inner_acquisition_failure_flushes_receive_settings_both_backends \
  actual_run_inner_acquisition_error_restores_and_joins_both_backends \
  actual_run_inner_satp_startup_error_stops_and_joins_both_backends \
  actual_run_inner_attachment_error_stops_and_joins \
  actual_production_spawn_native_lease_failure_is_joined \
  actual_run_inner_failed_restoration_cannot_report_clean_or_detach_owner
do
  # Each process owns its own native channels. Failed restoration intentionally
  # retains uncertain native storage; never reuse it in another fixture.
  if timeout 90s cargo test --offline --features managed-puresignal "$RUNTIME_TEST" \
      -- --ignored --nocapture --test-threads=1 > "$SATURN_RUNTIME_EVIDENCE_DIR/$RUNTIME_TEST.txt" 2>&1; then
    printf 'PASS %s\n' "$RUNTIME_TEST"
  else
    RUNTIME_STATUS=$?
    printf 'FAIL %s exit_status=%s\n' "$RUNTIME_TEST" "$RUNTIME_STATUS" >&2
    exit "$RUNTIME_STATUS"
  fi
done
