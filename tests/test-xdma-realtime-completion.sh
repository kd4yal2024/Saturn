#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
LIBXDMA="$REPO_ROOT/linuxdriver/xdma/libxdma.c"
LIBXDMA_HEADER="$REPO_ROOT/linuxdriver/xdma/libxdma.h"
SGDMA="$REPO_ROOT/linuxdriver/xdma/cdev_sgdma.c"

fail(){
  printf 'XDMA real-time completion contract failed: %s\n' "$*" >&2
  exit 1
}

grep -Fq 'static bool completion_wq_highpri;' "$LIBXDMA" \
  || fail "high-priority completion mode is not default-off"
grep -Fq 'module_param(completion_wq_highpri, bool, 0444);' "$LIBXDMA" \
  || fail "completion workqueue selector is not load-time-only"
grep -Fq 'WQ_HIGHPRI | WQ_UNBOUND | WQ_MEM_RECLAIM' "$LIBXDMA" \
  || fail "dedicated completion workqueue lacks required priority and isolation flags"
grep -Fq 'queue_work(engine->xdev->completion_wq, &engine->work);' "$LIBXDMA" \
  || fail "interrupt completion does not use the dedicated workqueue"
grep -Fq 'schedule_work(&engine->work);' "$LIBXDMA" \
  || fail "default shared-workqueue fallback was removed"
grep -Fq 'cancel_work_sync(&engine->work);' "$LIBXDMA" \
  || fail "engine teardown does not drain completion work"
grep -Fq 'struct workqueue_struct *completion_wq;' "$LIBXDMA_HEADER" \
  || fail "XDMA device does not own its completion workqueue"

grep -Fq 'static unsigned int transfer_latency_warn_us;' "$SGDMA" \
  || fail "transfer latency diagnostics are not default-off"
grep -Fq 'module_param(transfer_latency_warn_us, uint, 0644);' "$SGDMA" \
  || fail "transfer latency threshold is not runtime-adjustable"
grep -Fq 'pin_sg=%llu us submit_wait=%llu us cleanup=%llu us' "$SGDMA" \
  || fail "slow-transfer warning omits stage-level timing"

printf 'XDMA real-time completion contract passed\n'
