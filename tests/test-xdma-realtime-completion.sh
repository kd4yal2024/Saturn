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
grep -Fq 'module_param(completion_kthread_priority, uint, 0444);' "$LIBXDMA" \
  || fail "SCHED_FIFO completion mode is not load-time-only"
grep -Fq 'sched_setattr_nocheck(xdev->completion_task, &attr);' "$LIBXDMA" \
  || fail "completion thread does not receive real-time scheduling"
grep -Fq 'atomic_or(engine->irq_bitmask,' "$LIBXDMA" \
  || fail "interrupt completion cannot signal the real-time thread"
grep -Fq 'wait_event_interruptible(' "$LIBXDMA" \
  || fail "real-time completion thread lacks an event-driven wait"
grep -Fq 'kthread_stop(xdev->completion_task);' "$LIBXDMA" \
  || fail "real-time completion thread is not stopped during teardown"
grep -Fq 'cancel_work_sync(&engine->work);' "$LIBXDMA" \
  || fail "engine teardown does not drain completion work"
grep -Fq 'struct workqueue_struct *completion_wq;' "$LIBXDMA_HEADER" \
  || fail "XDMA device does not own its completion workqueue"

grep -Fq 'unsigned int transfer_latency_warn_us;' "$LIBXDMA" \
  || fail "transfer latency diagnostics are not default-off"
grep -Fq 'module_param(transfer_latency_warn_us, uint, 0644);' "$LIBXDMA" \
  || fail "transfer latency threshold is not runtime-adjustable"
grep -Fq 'pin_sg=%llu us submit_wait=%llu us cleanup=%llu us' "$SGDMA" \
  || fail "slow-transfer warning omits stage-level timing"
grep -Fq 'submit_to_irq=%llu us irq_to_worker=%llu us worker_to_wake=%llu us wake_to_resume=%llu us' "$LIBXDMA" \
  || fail "completion diagnostics omit interrupt, worker, or wake timing"

printf 'XDMA real-time completion contract passed\n'
