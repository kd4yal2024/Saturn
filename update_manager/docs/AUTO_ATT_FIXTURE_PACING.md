# Auto ATT file/loopback fixture completion clock

The November 16 real-WDSP Auto ATT matrix failed on both `76c6dc1f` and
`0a76e2ff`: Direct XDMA `AutoCorrection` reached feedback-ingress `QueueOverrun`
at client stage 3. Those original failures remain in
`Saturn_XDMA_Notified_Wait_20261116`; they are not passing reader-wait results.

## Confirmed fixture defect and repair

`shared_backend_tests::Io::before_receive` supplied a 128-frame Direct DMA
completion on every acquisition-loop turn. A turn may immediately follow a
successful receive; it is not a sample clock. The fixture therefore supplied
feedback faster than the configured 192,000 pairs/s and could overwhelm the
unchanged bounded ingress while native Auto fitting was running.

Both modeled Direct layouts produce 48,000 frames/s:

| Layout | Samples per frame | Samples per 128-frame completion |
| --- | --- | --- |
| Normal DDC6 at 384 kS/s | 8 complex RX samples | 1,024 complex RX samples |
| Paired feedback at 192 kpair/s | 4 reference/RF pairs | 512 pairs |

A completion is due every 128 / 48,000 seconds (approximately 2.667 ms).
The fixture now uses integer frame accounting against an absolute `Instant`
epoch. Late polling retains owed completions and their original due timestamps;
it does not reschedule them from the current time, discard them, or refresh
their observed age. Sequence advances only when publishing a completion.

An epoch restarts on an **applied** acquisition revision after the existing
pause/reset/readback/resume transaction. A requested revision alone does not
restart it. The test reader acknowledges pause but does not service layout,
native fitting, lifecycle, or restoration. The actual `run_inner` owners still
do that work, including on turns with no new completion.

The waveform helper generates exactly the frames delivered, rather than
generating 256 and discarding half. Feedback amplitude comes from the applied
layout scope. The independent P2 UDP source keeps its existing pacing. The
explicit diagnostic burst and diagnostic measurement cadence remain separate.

All executable changes are inside test-only modules. Production ingress
capacity, storage pools, timestamp validation, sample guards, watchdogs, native
processing and hardware reader scheduling are unchanged. This repair requires
no appliance deployment or service restart.

## Validation

Evidence: `Downloads/Saturn_Auto_ATT_Fixture_20261116` on the development host.
The runner and logs retain real native WDSP results separately from stub tests.
Run real-native cases serially, each in a fresh process without overlapping
builds/profiles, because WDSP has process-global state.

Deterministic tests cover the exact first deadline, late-completion debt and
timestamps, revision reset, one-second sample counts at multiple polling
cadences, and waveform partition continuity. The actual layout test checks that
the fixture exposes the old revision before pause, none during pause, and the
new revision only after application. Negative P2/Direct queue tests still
require `QueueOverrun` on exhaustion and recovery of all pooled storage.

A deterministic comparison sends identical samples through the real Direct
parser and feedback queue while withholding the consumer for 60 ms. The old
one-completion-per-1-ms-turn schedule overflows at 34 ms. The corrected sample
clock queues 11 complete blocks, with contiguous intervals and original
per-pair arrival timestamps; the queue can hold 16. This demonstrates the
excess-rate failure without relying on host scheduling or native-fit timing.

The complete 14-case Single/Auto/cancellation matrix passes twice on the repaired
fixture. It covers both P2 and Direct XDMA: Single, Auto, release during startup,
missing feedback, reconfiguration, disconnect and feedback range failure.
Successful Single/Auto cases require two corrected generations. Receive/release/
rearm/shutdown, WAN spectrum, acquisition-error cleanup and rear-PTT checks also
pass on both backends, retaining restoration, cleanup and owner-join assertions.
The minimum- and maximum-attenuation correction/rearm cases, native keepalive
liveness/missing-source checks and receive-settings preservation also pass on
both backends. Stub binary suites pass 747 managed and 548 default tests; these
are listed separately and do not substitute for native results.

The historical real-native failure is intermittent: both a fresh unchanged
`0a76e2ff` run and a control retaining all other fixture fixes but removing the
completion clock passed in this investigation. Their logs and the control diff
are retained. The pacing defect and its queue effect are demonstrated; these
runs do not isolate the entire historical scheduling stall or prove its only
cause. Keep that distinction when interpreting the repaired matrix results.

These are offline software results. They do not establish physical RF output,
amplifier behavior, calibrated IMD, or reliability outside this fixture profile.
