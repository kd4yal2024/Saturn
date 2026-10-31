# Managed feedback and native worker scheduling repair

The first G2 SINGLE trial on source `b717fbfa23f2` exhausted the Direct XDMA
feedback pool about 110 ms after keyed-control readback. The journal then shows
about 1.377 seconds before owner fault handling. Host cleanup completed with
`QueueOverrun` retained. These timestamps do not measure physical RF cessation.

Offline reproduction confirmed the four-box pool held only two queued Direct
blocks after allocating scratch and assembly storage. Read-only G2 inspection
also found the owner and native TX workers sharing FIFO 21 on CPU 3. Production
configured the owner before constructing WDSP; native pthreads inherited that
policy and affinity. Ordinary fixture tests skipped realtime setup. Scheduling
contention is a strong explanation to test, not a proved stack diagnosis of the
original stall.

## Changes

- Capture the allowed worker CPU mask before pinning the owner. Where multiple
  CPUs are available, reserve the realtime owner's last CPU for that owner.
- Construct and recreate managed native channels under explicit `SCHED_OTHER`
  and the retained worker mask. Restore and verify the creator's original
  scheduler, priority and affinity afterward. Restoration errors fail startup
  or cleanup; they cannot report ReadyOff success. No worker lookup by name,
  second native owner or change to the pinned WDSP archive is needed.
- Use a finite 16-slot feedback channel and 20 preallocated boxes: 16 queued,
  two producer-held, one consumer-held and one replacement before submission.
  This reaches explicit channel overflow rather than exhausting inventory
  before a full channel can reject a submission. It adds no allocation to the
  producer and does not drop samples to hide overload.
- Sixteen Direct blocks represent 85.33 ms of queued payload at 192 kpair/s;
  this is not a wall-clock guarantee. P2's 119-pair packets use the same slot
  budget sooner. Controller/source freshness, generation checks and the
  one-native-exchange-per-turn rule are unchanged.
- Retain maximum owner-turn time, native-pump time and observed feedback age
  as numeric counters and include them in existing lifecycle-transition logs.
  There is no new per-packet disk export or analyzer on the TX thread.

## Regression coverage

Stream tests exercise six native Direct blocks delivered in 32,768-byte DMA
chunks, original timestamps/ordinals, full-queue overflow with a consumer-held
box, P2 overflow and complete storage recovery. They retain older discontinuity
and generation tests.

Scheduling tests check success, callback error, failed affinity selection and
unwind restoration. A real-WDSP test verifies the actual Wflush1, PURESIGNAL and
Wchan1 policy and affinity after construction and two cleanup recreations.
These unprivileged tests use a SCHED_BATCH creator, not FIFO latency claims.

Combined actual-owner tests for both backends cover healthy queued bursts,
release, channel overflow and a real 200 ms consumer pause against an unchanged
150 ms feedback deadline. The stale pause must fault and produce no additional
output packets. Matching real-WDSP runs exercise the same cases with native
two-tone and zero-input keepalive. Loopback/file fixtures are not RF tests.

## Qualification boundary

No FPGA, kernel-driver, RXC1 polling, feedback scaling, drive or RF authorization
settings change is part of this repair. The G2 trial and its original fault
evidence are preserved. The repaired ARM binary must still demonstrate its
actual worker policies, receive operation and orderly cleanup on the CM4 before
another attended RF trial. No passing x86 replay establishes a CM4 realtime
latency bound, physical RF-off timing, live calibration or IMD improvement.
