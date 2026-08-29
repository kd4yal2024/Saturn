#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
LIBXDMA="$REPO_ROOT/linuxdriver/xdma/libxdma.c"
LIBXDMA_HEADER="$REPO_ROOT/linuxdriver/xdma/libxdma.h"
SGDMA="$REPO_ROOT/linuxdriver/xdma/cdev_sgdma.c"
BYPASS="$REPO_ROOT/linuxdriver/xdma/cdev_bypass.c"
THREADS="$REPO_ROOT/linuxdriver/xdma/xdma_thread.c"
MODULE="$REPO_ROOT/linuxdriver/xdma/xdma_mod.c"

fail(){
  printf 'XDMA driver hardening contract failed: %s\n' "$*" >&2
  exit 1
}

grep -Fq 'if (!list_empty(&engine->transfer_list)) {' "$THREADS" \
  || fail "poll completion does not recheck the transfer list under lock"
grep -Fq 'expected_desc_count = transfer->desc_cmpl_th;' "$THREADS" \
  || fail "poll completion does not snapshot its threshold safely"
grep -Fq 'xdma_kthread_stop(&cs_threads[thread_cnt]);' "$THREADS" \
  || fail "partial polling-thread startup does not stop created threads"

grep -Fq 'struct mutex perf_lock;' "$LIBXDMA_HEADER" \
  || fail "performance ioctl lifecycle has no mutex"
grep -Fq 'mutex_lock(&engine->perf_lock);' "$SGDMA" \
  || fail "performance ioctls are not serialized"
grep -Fq 'if (!requested.transfer_size)' "$SGDMA" \
  || fail "performance ioctl accepts a zero transfer size"
grep -Fq 'struct mutex open_lock;' "$LIBXDMA_HEADER" \
  || fail "streaming C2H ownership has no mutex"
grep -Fq 'mutex_lock(&engine->open_lock);' "$SGDMA" \
  || fail "streaming C2H open and close are not serialized"

async_fail_closed_count="$(grep -Fc 'return -EOPNOTSUPP;' "$LIBXDMA")"
(( async_fail_closed_count >= 2 )) \
  || fail "legacy asynchronous transfer entry points are not fail-closed"
grep -Fq 'sgt->sgl, sgt->orig_nents,' "$LIBXDMA" \
  || fail "DMA unmap does not retain the original scatterlist count"

grep -Fq 'goto fail_resource;' "$LIBXDMA" \
  || fail "engine register initialization does not release allocated resources"
grep -Fq 'engine_free_resource(engine);' "$LIBXDMA" \
  || fail "engine failure cleanup does not free coherent allocations"
grep -Fq '(u8 __iomem *)bypass_addr +' "$BYPASS" \
  || fail "bypass BAR offset is not applied as a byte offset"
grep -Fq 'failed to restore interrupts for %s: %d' "$LIBXDMA" \
  || fail "PCI recovery does not fail closed when IRQ restoration fails"
grep -Fq 'if (rv)' "$MODULE" \
  || fail "module initialization does not check PCI registration"
grep -Fq 'xdma_cdev_cleanup();' "$MODULE" \
  || fail "module initialization failure does not release cdev resources"

grep -Fq 'list_del_init(&transfer->entry);' "$LIBXDMA" \
  || fail "failed engine start leaves the transfer queued"
grep -Fq 'INIT_LIST_HEAD(&transfer->entry);' "$LIBXDMA" \
  || fail "performance transfer cleanup can operate on an uninitialized list node"
grep -Fq 'if (free_desc && engine->desc) {' "$LIBXDMA" \
  || fail "performance error cleanup can discard a preallocated descriptor pointer"
grep -Fq 'channel < 0 || channel >= xdev->h2c_channel_max' "$LIBXDMA" \
  || fail "kernel transfer API accepts a negative H2C channel"

printf 'XDMA driver hardening contract passed\n'
