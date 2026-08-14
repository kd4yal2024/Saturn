# Saturn Bridge Direct XDMA

## Goal

Saturn Bridge will support two appliance-wide radio backends:

- `p2`: the stable Protocol 2 path through `p2app.service`
- `xdma`: direct FPGA register and DMA access modelled on piHPSDR

Protocol 2 remains the default until the direct backend passes the complete
receive, audio, transmit-safety, recovery, and soak-test gates.

Backend ownership is global to the appliance. It is not selected independently
for each browser connection because P2_app and Saturn Bridge must never drive
the same FPGA registers or DMA channels concurrently.

## Production Qualification Status (2026-08-14)

P2_app has not been removed. It remains the installed/default backend for
Thetis and other Protocol 2 clients. Direct XDMA is an explicit, transactional
alternative for Saturn Remote and other TCI clients; only one backend owns the
FPGA at a time.

| Area | Current evidence | Gate |
| --- | --- | --- |
| Direct RX | Live 384 kHz IQ/audio, tuning, reconnect recovery, and receive-safe cleanup passed on PCB2 firmware 1.27 at 383,321 IQ pairs/second. | Passed for the current hardware envelope. |
| Direct TX audio | After distinct-frame DMA batching, a 15-minute dummy-load run completed 22 keyed Voodoo 3.8k voice sessions with clean audio before and after a browser reconnect, 22 mux resets, and zero FIFO faults. | Passed for repeated-key voice and reconnect at the current 3 W envelope; longer unattended soak remains required. |
| Repeated-key automation | The final 384 kHz split-proxy run completed five independent RF-inhibited cycles under SCHED_FIFO priority 20, with 3,310 total TX frames, a 3,728-word high-water, zero TX FIFO faults, and receive-safe cleanup. | Passed; validation persisted. |
| Backend ownership | Transactional `P2 -> XDMA -> P2` switching and rollback have passed. The installer now preserves the selected backend instead of silently returning an XDMA appliance to P2. | Repeat after every installer or broker change. |
| Wider production use | P2 fallback is retained and direct XDMA stays opt-in. | Requires repeated-key, bounded-RF, reconnect, and soak gates below. |

The FIFO event above occurred before RF keying with RF inhibited. It does not
indicate microphone loss, an RF frequency error, or an over-the-air audio
failure. The runtime failed closed, disarmed the unkeyed TX session, returned
to RX, and restored the prior services. The correction deliberately waits for
DMA writes to become visible before making another prefill decision and admits
each live frame only with three complete frames of FIFO ceiling headroom.

The next hardware run confirmed that pacing removed the ceiling fault: 460 DUC
frames were staged with a 3,710-word high-water and no threshold or overflow.
The acceptance client then remained silent while waiting up to eight seconds
for the next readiness transition, exceeding the direct backend's 1.5-second
operator-control watchdog. RF-inhibited and bounded-RF client waits now send a
valid `saturn_ping` every 250 ms while armed. The watchdog and its timeout are
unchanged.

Two follow-up runs separated the remaining test conditions. One started while
an open Saturn Remote browser already held the operator lease; the acceptance
session was a viewer and therefore could not issue TX commands. The client now
requires an explicit operator-role response and fails immediately with an
instruction to close other Remote/TCI clients. An isolated run retained its
lease and heartbeat but underflowed after 185 frames. The 3,710-word prefill is
only about 24 ms at 192 kHz, while observed ordinary-scheduler control bursts
reached 25--45 ms. This confirms the earlier Phase 4 result: FIFO depth cannot
substitute for deterministic refill scheduling.

The first deterministic correction pinned the direct-XDMA TX producer to an
allowed CPU and required `SCHED_FIFO` priority 20 before startup readiness.
Failure to obtain or verify that policy aborted the direct backend before a
client could arm TX; P2's established UDP TX thread remained under the normal
scheduler.

The first two optimized 192 kHz five-cycle runs confirmed that correction on the
appliance. Both completed every arm/disarm cycle without a DUC fault and
returned receive-safe. Their final harness result still failed because the
original steady-IQ calculation divided all IQ progress by the entire runtime,
including the intentional half-duplex TX intervals, yielding approximately
169k pairs/second. The rate gate now snapshots a fresh start point only after
the final TX release has remained receive-safe.

The direct DDC was then advanced to its firmware-supported 384 kHz rate code.
The final split-proxy appliance run measured 383,321 IQ pairs/second over an
18.860-second receive-only interval, within the required 376,320--391,680
range. All five RF-inhibited TX cycles passed with one mux reset per arm, zero
FIFO faults, and a final receive-safe state. The passing record is stored at
`/var/lib/saturn-state/xdma-telemetry.json` with the client result, post-TX rate
start, final stopped snapshot, and service-restoration outcome.

A subsequent live 384 kHz browser run confirmed RX/audio, band changes, and
three keyed voice sessions, but exposed two independent recoverable TX
boundaries. The first Opus attempt accumulated decode errors before keying;
another session reached a 230-word DUC low-water observation while the FPGA's
underflow, overflow, and threshold flags were all clear. Split sessions now
downgrade both lanes atomically to PCM after an Opus decode failure and ignore
already queued Opus chunks during that handoff. Production DUC pacing now
treats a low but nonempty observation as an immediate refill opportunity; only
the FPGA's latched runtime fault flags force receive.

Two fresh 384 kHz inhibited runs then passed at 383,348 and 383,258 IQ
pairs/second with all five cycles fault-free. The installed bridge completed
five live Opus voice sessions, but the third session latched one real DUC
underflow while the other four remained clean. Kernel transfer diagnostics
isolated the remaining boundary: H2C completion interrupts arrived in tens of
microseconds, but an awakened priority-20 TX producer could wait 6--10 ms while
the equal-priority shared completion kthread continued servicing C2H work.
Production TX therefore runs at FIFO priority 21, one level above the
priority-20 completion thread, and the service grants `LimitRTPRIO=21`. This
preserves the power, SWR, watchdog, ceiling-headroom, and receive-safe cleanup
gates and requires one more inhibited acceptance plus live repeated-key
confirmation.

The priority-21 live follow-up still caught one second-key underflow. Kernel
timing showed that the H2C interrupt arrived promptly, but a synchronous
1,440-byte transfer could still take 6--11 ms to resume in userspace. The DUC
consumes one such 240-pair frame every 1.25 ms, so scheduling priority alone
cannot make a one-syscall-per-frame producer robust against that tail.
Production direct XDMA now batches up to eight consecutive, distinct DUC IQ
frames into one H2C transfer. The TX thread advances its cadence by the number
of frames in the batch. The backend writes the largest safe partial batch as
soon as measured FIFO space permits, then completes any remainder while always
retaining three-frame ceiling headroom; it never drains the FIFO merely to make
an entire requested batch fit. Packet-level display and diagnostic accounting
remain unchanged. RF-inhibited staging uses the same batch path so the
five-cycle acceptance exercises the correction before any RF test. Protocol 2
retains its established one-packet UDP output.

The corrected RF-inhibited split-proxy run passed all five cycles at FIFO
priority 21. It sustained 383,284 IQ pairs/second, staged 829--841 DUC frames
per cycle, recorded a 1,112-word low-water and 3,605-word high-water, and
reported zero FIFO faults. Final receive-safe cleanup and restoration of the
previous service state also passed, and the validation record was refreshed.

The installed production bridge then passed a live repeated-key voice check at
7.210 MHz. Four consecutive Voodoo 3.8k transmissions sounded clear, all four
sessions keyed and released, and the final runtime snapshot reported four
sessions started/completed, 12,256 distinct DUC frames in 6,976 H2C writes,
four mux resets, and zero FIFO faults. The 140--3,623-word lifetime FIFO range
shows that the producer exercised the low-water recovery path without latching
an underflow. The four startup-underflow indications are the expected sticky
empty-FIFO events cleared during the four deliberate reset/prefill cycles.
Two subsequent locked dummy-load probes passed at 7.200 MHz with a 3 W drive
setting and a bounded 2.5-second transmission. They measured approximately
0.487 W forward, zero reverse power, SWR 1.00, and 383,570--383,575 IQ
pairs/second. Both runs completed with zero FIFO and framing faults, verified
receive-safe cleanup, and restored the prior services.

A later 15-minute live dummy-load run completed 22 keyed Voodoo 3.8k voice
sessions with clean audio before and after a deliberate Saturn Remote browser
reload. All 22 sessions completed with one DUC mux reset per session, 55,563
distinct DUC frames in 30,476 H2C writes, a 653--3,623-word FIFO range, and
zero FIFO, header, or resynchronization faults. One rapid post-reload MOX
attempt armed without receiving microphone audio; the mic-stale watchdog
forced receive state and the session never keyed. The following deliberate
arm acquired microphone audio and transmitted normally. A longer unattended
mixed-path soak remains the next live gate.

Run the current production-path acceptance from the repository root:

```bash
CARGO_BUILD_JOBS=1 cargo build --release \
  --manifest-path update_manager/saturn-bridge/Cargo.toml

sudo update_manager/scripts/saturn-xdma-operational-rx-smoke.sh \
  --proxy-client-probe \
  --tx-cycles 5 \
  --duration-seconds 45
```

Close or disconnect every Saturn Remote and other TCI client before starting
the acceptance. The test must acquire the operator lease; it now rejects a
viewer assignment instead of waiting for TX state that the viewer cannot
request. Client acceptance selects the optimized release bridge by default
because a debug build cannot satisfy the 1.25 ms DUC refill cadence reliably.

A passing result must complete all five independent arm/disarm cycles. Every
cycle must report a new mux reset, advancing DUC DMA writes and frames, zero TX
FIFO faults, and a final unkeyed receive-safe state. Only a passing run writes
`/var/lib/saturn-state/xdma-telemetry.json`; an absent or stale file must not be
presented as current qualification evidence.

The direct-backend readiness snapshot in
`/run/saturn-bridge/xdma-ready.json` includes lifetime counters and the last
completed TX session. The session record identifies its frequency and filter,
whether it keyed, DMA writes and frames, FIFO low/high water, FIFO faults,
startup underflows, mux resets, peak forward/reverse power, SWR, and duration.
This makes an alternating-key regression attributable to one arm instead of
only to process-wide totals.

To return immediately to the Protocol 2 backend:

```bash
sudo /usr/local/lib/saturn-go/scripts/saturn-radio-backend-switch-root.sh switch p2
```

Verify `p2app.service` is active before opening Thetis or another Protocol 2
client. Do not start P2_app manually while the XDMA backend owns the FPGA; use
the transaction broker so ownership, systemd state, readiness, and persisted
selection move together.

### Remaining production gates

1. Pass the five-cycle RF-inhibited split-proxy test repeatedly with no FIFO,
   framing, DMA, session, or cleanup fault.
2. Pass `saturn-xdma-backend-switch-smoke.sh`, including restoration of the
   exact prior service state and persisted selection.
3. Repeat the bounded 3 W dummy-load RF test only after the inhibited gate is
   green; verify frequency/sideband with the locked tone and retain the power,
   reverse-power, SWR, and per-session record.
4. Exercise browser reconnect plus multiple voice key-ups and confirm every arm
   creates a new WDSP TX channel and DUC mux reset.
5. Complete long-duration RX and mixed RX/TX soak testing before raising the
   power envelope or considering XDMA as the default.
6. Re-run installer, CI, and rollback tests while P2 is selected and while XDMA
   is selected. Neither installation nor upgrade may discard the operator's
   valid persisted backend choice.

## Operator Switching Contract

Saturn Remote will ultimately expose the backend selector to its operator.
Changing it must be a transactional control-plane operation:

1. Force RX and disable RF output.
2. Disconnect and release the active backend.
3. Stop `p2app.service` when moving to XDMA, or release XDMA before starting P2.
4. Claim and validate the requested backend.
5. Re-bootstrap any connected client against the new backend.
6. Start its RX path and verify data-plane health.
7. Persist the selection only after readiness succeeds.
8. Automatically restore the previous backend if any step fails.

Service state alone is not backend readiness. With a client connected, the P2
backend must complete discovery and show advancing high-priority and DDC packet
counters. The direct XDMA backend will require advancing DMA/FIFO counters.
With no client connected, the selected backend may remain available and idle;
the first client connection must complete the same validation before the UI
reports it active.

The UI must show the active backend separately from the requested backend and
must not represent Phase 1 probing as an operational radio connection.

## Phase 1: Identity and Safe Lifecycle

Phase 1 adds a one-shot probe to the installed `saturn-bridge` binary:

```bash
sudo systemctl stop p2app.service
sudo -u pi /opt/saturn-go/bin/saturn-bridge --xdma-probe
sudo systemctl start p2app.service
```

The probe:

- refuses to run while `p2app.service` is active
- opens `/dev/xdma0_user` through the `saturn-radio` group policy
- verifies Saturn product ID, golden/primary image ID, clock status, and FPGA
  firmware major version
- preserves unrelated register bits while forcing MOX, TX enable, PA relay,
  CW keyer, TX watchdog override, and DUC streaming into their safe state
- repeats the safe-state operation during cleanup

`SATURN_BRIDGE_XDMA_USER_DEVICE` may override the register device path for
fixture testing. It is not an operational backend-selection setting.

Phase 1 does not open C2H/H2C DMA streams and cannot receive or transmit.

## Phase 2: RX-only DDC IQ

Phase 2 adds a one-shot, RX-only capture:

```bash
sudo systemctl stop p2app.service
sudo -u pi /opt/saturn-go/bin/saturn-bridge --xdma-rx-probe
sudo systemctl start p2app.service
```

The capture:

- refuses to run while `p2app.service` is active
- keeps MOX, TX enable, PA relay, CW keyer, and DUC streaming disabled
- configures hardware DDC6 for ADC1 at 384 kHz
- reads page-aligned DMA blocks from `/dev/xdma0_c2h_0`
- validates every 64-bit rate header and packed 24-bit I/Q frame
- reports a synthesized frame sequence, observed sample rate, DMA throughput,
  FIFO high-water/error counters, header resynchronization, RMS, and peak
- disables DDC streaming, clears the rate word, and resets the FIFO on every
  normal or error exit

The following test-only environment settings are supported:

- `SATURN_BRIDGE_XDMA_RX_FREQUENCY_HZ` (default `14200000`)
- `SATURN_BRIDGE_XDMA_RX_DURATION_MS` (default `2000`, range `250..10000`)
- `SATURN_BRIDGE_XDMA_RX_DEVICE` (default `/dev/xdma0_c2h_0`)

Phase 2 does not send the captured samples to clients. P2 remains the only
operational backend while the direct data path is validated in isolation.

## Phase 3: Codec microphone and speaker DMA

Phase 3 adds a one-shot codec-audio probe:

```bash
sudo systemctl stop p2app.service
sudo -u pi /opt/saturn-go/bin/saturn-bridge --xdma-audio-probe
sudo systemctl start p2app.service
```

The probe:

- refuses to run while `p2app.service` is active
- captures 48 kHz signed 16-bit microphone samples from
  `/dev/xdma0_c2h_1` at AXI offset `0x40000`
- honors the FPGA's current local/network byte-order setting
- writes only zero-valued 48 kHz stereo speaker frames to
  `/dev/xdma0_h2c_1` at AXI offset `0x40000`
- asserts the hardware speaker mute before opening the audio path and leaves
  it asserted during cleanup
- verifies that the speaker FIFO accepts the DMA write and begins draining
- reports microphone level/rate and per-stream DMA/FIFO telemetry
- resets both codec FIFOs and preserves the Phase 1 receive-safe RF state on
  every normal or error exit

The following test-only environment settings are supported:

- `SATURN_BRIDGE_XDMA_AUDIO_DURATION_MS` (default `2000`, range `250..10000`)
- `SATURN_BRIDGE_XDMA_MIC_DEVICE` (default `/dev/xdma0_c2h_1`)
- `SATURN_BRIDGE_XDMA_SPEAKER_DEVICE` (default `/dev/xdma0_h2c_1`)

Phase 3 does not route codec audio to clients and does not enable DUC or RF
transmit. P2 remains the only operational client backend.

## Phase 4: RF-disabled DUC IQ performance

Phase 4 adds a one-shot DUC throughput and tail-latency probe:

```bash
sudo systemctl stop p2app.service
sudo -u pi /opt/saturn-go/bin/saturn-bridge --xdma-duc-probe
sudo systemctl start p2app.service
```

The probe:

- refuses to run while `p2app.service` is active
- writes zero-valued 24-bit I/Q pairs to `/dev/xdma0_h2c_0` by default, with
  an optional deterministic changing-IQ soak pattern
- forces MOX, TX enable, PA relay, CW keyer, and TX watchdog override off
- holds the TX amplitude scale at zero and verifies the RF-safe register state
  before every DMA refill
- configures the safety state once and then uses read-only verification in the
  refill hot path, avoiding redundant register writes while still aborting
  before DMA if any RF control changes
- locks the 15,840-byte aligned DMA buffer in RAM; optional CPU affinity is
  available for A/B measurements but remains off by default because a
  non-isolated CPU can worsen scheduler stalls
- enables the zero-amplitude, RF-disabled DUC mux immediately before seeding
  twenty 1.25 ms frames in writes of at most eleven frames (H2C0 discards
  writes while the mux is disabled)
- refills at twelve frames toward a twenty-frame target; this keeps the normal
  transfer near eight frames, halves the refill rate, and retains approximately
  15 ms at the normal low-water boundary
- uses fixed-memory latency histograms, allowing multi-hour runs without
  accumulating one allocation per refill
- reports observation counts, FIFO low-water/fault counts, batch-size changes,
  DMA write latency, p99 and p99.99 write/refill service timing, exact maximum
  refill service, maximum loop gap, minimum FIFO time margin, and observed IQ
  rate; soak progress is printed every 60 seconds
- marks p99.99 as sample-sufficient after 10,000 refill observations; shorter
  runs still apply the conservative nearest-rank gate but do not claim a
  statistically representative tail
- rejects a rate outside five percent of 192 kHz or any runtime FIFO fault
- stops immediately on the first runtime FIFO fault or critical low-water
  observation, while retaining and printing the partial-run telemetry
- rejects p99.99 low-water-to-write-complete service above 5 ms
- rejects a maximum refill service that reaches the minimum observed FIFO
  margin, or p99.99 service above 60 percent of that margin
- disables the DUC mux and output gate, resets the FIFO, retains zero
  amplitude, and restores the receive-safe state on every exit
- prints the complete bounded telemetry record before returning a failed
  acceptance gate, preserving evidence from tail-latency failures

The following test-only environment settings are supported:

- `SATURN_BRIDGE_XDMA_DUC_DURATION_MS` (default `3000`, range
  `500..86400000`, up to 24 hours)
- `SATURN_BRIDGE_XDMA_DUC_DEVICE` (default `/dev/xdma0_h2c_0`)
- `SATURN_BRIDGE_XDMA_DUC_PATTERN` (`zero` by default, or `changing`)
- `SATURN_BRIDGE_XDMA_DUC_CPU` (`none` by default, a CPU number, or `auto` for
  the highest available CPU)
- `SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY` (`0` by default; `1..80` requests
  `SCHED_FIFO` and requires root or `CAP_SYS_NICE`)
- `SATURN_BRIDGE_XDMA_DUC_MAX_P9999_REFILL_SERVICE_US` (default `5000`)
- `SATURN_BRIDGE_XDMA_DUC_MAX_P9999_MARGIN_PERCENT` (default `60`)

The changing pattern exercises the H2C data path while TX amplitude remains
zero and the RF controls remain inhibited. Phase 4 cannot generate RF.
Guarded RF-keying tests remain separate Phase 5 work.

A 30-minute changing-IQ soak can be run with:

```bash
sudo systemctl stop p2app.service
sudo -u pi env \
  SATURN_BRIDGE_XDMA_DUC_DURATION_MS=1800000 \
  SATURN_BRIDGE_XDMA_DUC_PATTERN=changing \
  /opt/saturn-go/bin/saturn-bridge --xdma-duc-probe
sudo systemctl start p2app.service
```

For an A/B scheduler comparison, run the same soak once normally and once
under a real-time FIFO policy (root is required to select the policy):

```bash
sudo env \
  SATURN_BRIDGE_XDMA_DUC_DURATION_MS=1800000 \
  SATURN_BRIDGE_XDMA_DUC_PATTERN=changing \
  SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY=20 \
  /opt/saturn-go/bin/saturn-bridge --xdma-duc-probe
```

The result records the selected CPU, scheduler policy and priority, and locked
DMA-buffer state alongside the latency measurements.

On the development CM4, an unpinned one-minute `SCHED_OTHER` changing-IQ run
crossed 10,000 refill observations but exposed a 78.87 ms H2C write stall, two
FIFO underflows, and one critical-low event. This is longer than the entire
hardware FIFO can cover, so the result blocks Phase 5. The next acceptance run
must use `SCHED_FIFO`; increasing FIFO watermarks cannot mask that scheduler
tail.

The corresponding 30-minute `SCHED_FIFO` priority 20 baseline passed with
360,000 refill observations, exact 192 kHz consumption, 4.600 ms p99.99 and
5.919 ms maximum refill service, 8.806 ms minimum FIFO margin, zero critical
low-water events, and zero runtime FIFO faults.

The final Phase 4 gate applies bounded host pressure. Run the isolated
five-minute profiles first:

```bash
sudo update_manager/scripts/saturn-xdma-phase4-stress.sh --profile cpu --duration-seconds 300
sudo update_manager/scripts/saturn-xdma-phase4-stress.sh --profile memory --duration-seconds 300
sudo update_manager/scripts/saturn-xdma-phase4-stress.sh --profile network --duration-seconds 300
sudo update_manager/scripts/saturn-xdma-phase4-stress.sh --profile storage --duration-seconds 300
```

Each invocation runs the same RF-inhibited changing-IQ probe with just one
bounded stress source. The harness selects an available CPU for the SCHED_FIFO
probe by default; set `SATURN_XDMA_STRESS_PROBE_CPU=none` only for an explicit
un-pinned comparison. A failed run stops on its first FIFO fault or critical
low-water observation, prints the probe telemetry and stressor log tails, and
restores `p2app.service`. The storage profile performs read-only random I/O
against the repository's block device; all temporary writes and logs remain in
`/dev/shm`.

The harness also refuses to start unless the loaded driver reports
`completion_kthread_priority=20`. This prevents a high-pressure test from
repeating the known shared-workqueue underflow mode. Set
`SATURN_XDMA_STRESS_REQUIRED_COMPLETION_PRIORITY=none` only for a deliberate
driver A/B experiment with RF still inhibited.

Only after every isolated profile passes should the combined 30-minute gate be
repeated:

```bash
sudo update_manager/scripts/saturn-xdma-phase4-stress.sh --profile combined
```

The first combined development run correctly failed: p99.99 refill service
reached 64.360 ms, maximum refill service reached 146.023 ms, the FIFO reached
zero, and the probe recorded 1,781 critical-low events and 2,662 underflows.
The same host had already passed the unstressed 30-minute `SCHED_FIFO` baseline,
and showed no thermal throttling, kernel I/O errors, or XDMA driver errors after
the stress failure. The isolated profiles are therefore required to distinguish
CPU, memory, loopback-network, and shared-storage interrupt pressure before any
scheduler or IRQ-affinity tuning.

The isolated five-minute results separated two failure modes:

| Profile | Result | Key evidence |
| --- | --- | --- |
| CPU | Marginal gate failure, no FIFO danger | 5.140 ms p99.99, 5.391 ms maximum, zero critical/fault events |
| Memory | Critical low-water | stopped at 91.492 s with a 9.365 ms loop gap and 1.451 ms FIFO margin |
| Network | FIFO underflow | stopped at 10.646 s with a 16.505 ms synchronous XDMA write and two underflows |
| Storage | Marginal gate failure, no FIFO danger | 5.260 ms p99.99, 5.755 ms maximum, zero critical/fault events |

The generic synchronous XDMA character path pins pages, constructs and maps a
scatterlist, allocates transfer metadata, waits for interrupt completion, and
then unmaps and frees those resources for every refill. Interrupt completion is
normally deferred to the shared kernel workqueue. Phase 4 therefore includes an
opt-in driver A/B experiment:

- `completion_wq_highpri=1` routes interrupt completions through a dedicated
  high-priority unbound workqueue. It is load-time-only and defaults to `0`.
- `completion_kthread_priority=20` instead routes completions through one
  event-driven SCHED_FIFO thread at the requested priority. It is
  load-time-only, defaults to `0`, and supersedes `completion_wq_highpri` when
  both are set.
- `transfer_latency_warn_us=5000` logs synchronous transfers at or above 5 ms
  with separate page-pin/scatterlist, submit/completion-wait, and cleanup
  timings. The completion-wait warning is further divided into submit-to-IRQ,
  IRQ-to-worker, worker-to-wake, and wake-to-caller stages so hardware/IRQ
  latency can be distinguished from workqueue and scheduler latency. It
  defaults to `0` and can be adjusted at runtime.

The generic driver defaults remain disabled. Saturn installation persists the
validated appliance policy in `/etc/modprobe.d/saturn-xdma.conf`:

```text
options xdma completion_kthread_priority=20 transfer_latency_warn_us=5000
```

The options apply on the next module load and every reboot. An already loaded
module retains its current load-time completion mode until XDMA is safely
unloaded and loaded again or the appliance reboots.

These options do not change XDMA device names, transfer semantics, or the
Protocol 2/direct-XDMA selection model. A persistent pre-pinned DMA ring remains
a later option only if the stage timings show that eliminating synchronous
per-transfer setup or completion waits can materially improve the Phase 4
tail.

The staged A/B run isolated an 8.241 ms IRQ-to-workqueue delay while hardware
submit-to-IRQ took only 35 microseconds. The SCHED_FIFO completion thread
removed that bottleneck. The DUC probe then moved synchronous progress output
off its real-time thread, selected an available CPU, and widened the operating
band from 7→11 frames to 12→20 frames while preserving the 11-frame maximum
DMA batch. The wider eight-frame window averaged eight frames per write, halved
the refill syscall rate to approximately 100/second, and retained 15 ms at the
normal low-water boundary.

The resulting five-minute combined CPU, memory, network, and storage profile
passed with 30,001 writes, exact 192 kHz consumption, 2.800 ms p99.99 and
3.111 ms maximum refill service, 7.743 ms observed minimum FIFO margin, and no
critical-low or runtime FIFO-fault events despite a 12.212 ms maximum loop gap.

The final 30-minute combined soak also passed. It sustained 192,000.1 IQ
pairs/second across 180,000 DMA writes and 345,600,213 consumed pairs. The
p99.99 refill service was 0.600 ms, the maximum was 1.632 ms, and p99.99 used
6.9% of the observed 8.743 ms minimum FIFO margin. The run recorded no
critical-low events, FIFO overflow, threshold fault, runtime underflow, or
histogram overflow. Cleanup left amplitude at zero and every RF control
inhibited.

A post-reboot 60-second combined regression confirmed that the persisted
priority-20 completion policy was active. It sustained 192,000.6 IQ
pairs/second with 0.368 ms maximum refill service, 14.556 ms minimum FIFO
margin, and zero critical-low or runtime FIFO-fault events.

## Phase 5: Guarded transmit preflight

The first Phase 5 slice remains completely RF-inhibited:

```bash
sudo systemctl stop p2app.service
sudo update_manager/saturn-bridge/target/release/saturn-bridge \
  --xdma-tx-preflight
sudo systemctl start p2app.service
```

The preflight validates exclusive XDMA ownership and compatible Saturn
identity, forces the common receive-safe state, and verifies register readback.
The common emergency cleanup now clears the modulation source, output gate,
amplitude, watchdog override, DUC mux reset, IQ deinterleave, and DUC stream in
addition to disabling MOX, TX enable, PA relay, and CW.

Two deliberate failure checkpoints exercise automatic cleanup without keying
RF:

```bash
sudo env SATURN_BRIDGE_XDMA_TX_PREFLIGHT_INJECT_FAILURE=after-open \
  update_manager/saturn-bridge/target/release/saturn-bridge \
  --xdma-tx-preflight
sudo env SATURN_BRIDGE_XDMA_TX_PREFLIGHT_INJECT_FAILURE=after-verify \
  update_manager/saturn-bridge/target/release/saturn-bridge \
  --xdma-tx-preflight
```

Both injected commands are expected to fail after arming receive-safe cleanup.
No Phase 5 preflight path enables the DUC stream, output gate, amplitude, MOX,
TX enable, or PA relay. A later low-power RF-keying probe requires a separately
confirmed load or antenna, explicit frequency and power limits, forward/reverse
power trips, a bounded key-down timer, and authorization to generate RF.

### First guarded RF probe

The first RF-generating Phase 5 probe is intentionally locked to the field
validation hardware and operating envelope:

- primary Saturn image, PCB2, firmware 1.27
- ANT1 with a confirmed 40 m antenna or suitable load
- 7.200 MHz only
- 3 W absolute forward-power ceiling
- 2.5 W ramp target and drive byte no higher than 11
- 0.75 W reverse-power and 3.0:1 SWR trips
- 250 ms default key-down, with a hard 500 ms configuration maximum
- XDMA completion thread and probe thread both at validated FIFO priority 20

The probe preloads the proven Phase 4 DUC FIFO while amplitude, MOX, TX enable,
and the PA relay remain inhibited. It then switches from the Phase 4 always-on
consumer to the normal MOX-gated data path, ramps drive one step every 12 ms,
and checks the FIFO and forward/reverse power on every loop. Drive and amplitude
are zeroed before MOX, TX enable, and the relay are returned to receive state.
SIGINT and SIGTERM request the same cleanup. Live carrier samples use the exact
P2_app DMA representation: Q then I, signed 24-bit network byte order, with the
FPGA byte-swap control explicitly enabled and verified.

Do not run this command until ANT1 is physically confirmed:

```bash
(
  trap 'sudo systemctl start p2app.service' EXIT
  sudo systemctl stop p2app.service

  sudo env \
    SATURN_BRIDGE_XDMA_TX_CONFIRM=ANTENNA_CONNECTED_40M_7200000HZ_3W_ANT1 \
    SATURN_BRIDGE_XDMA_TX_FREQUENCY_HZ=7200000 \
    SATURN_BRIDGE_XDMA_TX_MAX_WATTS=3 \
    SATURN_BRIDGE_XDMA_TX_ANTENNA=1 \
    update_manager/saturn-bridge/target/release/saturn-bridge \
      --xdma-tx-probe
)
```

`SATURN_BRIDGE_XDMA_TX_DURATION_MS` may be set only within 100 through 500 ms.
`SATURN_BRIDGE_XDMA_TX_POWER_METER_SCALE` defaults to `1.0` and is constrained
to `0.5` through `1.5`. The frequency, ceiling, antenna, confirmation token,
hardware revision, firmware revision, and completion policy cannot be relaxed
by environment variables in this first RF probe.

The first antenna-connected field run passed on 2026-07-29. At 7.200 MHz for
250 ms, the guarded ramp reached its drive-byte-11 ceiling and measured
1.403 W peak forward power, zero measured reverse power, 1.00 peak SWR, and a
2,342-word DUC FIFO low-water mark. The probe ran on CPU 3 with both the caller
and XDMA completion thread at FIFO priority 20, then verified receive-safe
cleanup. This establishes the direct-XDMA RF path without relaxing the 3 W
ceiling; higher-power calibration remains separate work.

## Structured probe telemetry

Every Phase 1 through Phase 5 one-shot probe now writes its latest outcome to:

```text
/var/lib/saturn-state/xdma-telemetry.json
```

Successful runs include phase-specific identity, DMA, FIFO, scheduler, signal,
power, and cleanup metrics. Failed runs retain the error and cleanup state;
Phase 4 validation failures retain the full bounded performance record rather
than only the final gate error. The file is replaced atomically with mode
`0644`.

Snapshot persistence is deliberately best-effort. A state-directory or write
failure emits a diagnostic warning but cannot change the probe result, skip
DMA shutdown, or interfere with receive-safe RF cleanup.

`SATURN_BRIDGE_XDMA_TELEMETRY_PATH` may override the destination for fixture
tests. It is not an operational backend-selection setting.

Saturn Go reads this record through `/bridge_diag` and displays it in Radio
Telemetry & Diagnostics under the **XDMA Telemetry** tab. The same payload also
reports backend ownership, device-node readiness, driver completion policy,
PCIe link state, and XDMA interrupt counts. Direct XDMA remains explicitly
inactive—not failed—while `p2app.service` owns the FPGA.

## Phase 6 foundation: transactional backend ownership

The first Phase 6 slice installs a root-owned transaction broker:

```text
/usr/local/lib/saturn-go/scripts/saturn-radio-backend-switch-root.sh
```

Its `status` command is read-only. `switch p2` stops Saturn Bridge at the
current receive-safe release boundary, applies an explicit P2 systemd
environment drop-in, starts P2 before the bridge, verifies both services and
the bridge runtime selection, and only then atomically persists
`/var/lib/saturn-radio-backend/selection.json`. This state is kept in a
root-owned directory that the Saturn Go service group can read but cannot
modify.

The transaction snapshots the prior service activity, bridge drop-in, and
persistent selection. A start, readiness, or mutual-exclusion failure restores
all three. The XDMA form first stops P2 and refuses to start the bridge if P2
retains ownership. It then requires a fresh RX heartbeat with advancing DMA
read and IQ-pair counters before persisting XDMA. Tests exercise successful
transactions in both directions, a missing data-plane heartbeat, a failed
bridge start, and a P2 service that refuses to stop.

The XDMA-specific bridge drop-in also declares
`Conflicts=p2app.service`. If an operator later starts P2 manually, systemd
stops the direct bridge first; its SIGTERM handler completes receive-safe DDC
cleanup before P2 can claim the FPGA.

The base bridge unit depends only on network readiness. It does not `Wants=`
P2, because such a dependency cannot be removed reliably by a systemd drop-in
and would silently reacquire P2 during an XDMA switch. Transaction ordering,
service-state verification, and the persisted backend environment replace that
implicit ownership.

The installed `/etc/default/saturn-radio-backend` now contains:

```text
XDMA_OPERATIONAL_ENABLED="1"
```

P2 remains the persisted/default owner after installation. Operators can select
XDMA explicitly through the Radio Telemetry page or the root broker. The broker
persists only a ready selection and restores the exact prior state on failure.

The next Phase 6 slice now provides an RX-only operational bridge runtime when
`SATURN_BRIDGE_RADIO_BACKEND=xdma` is set in an isolated test:

- DDC6/ADC1 runs at the validated fixed 384 kHz rate.
- decoded IQ is published through the existing TCI client stream and WDSP
  produces the existing 48 kHz receive-audio stream.
- VFO/center tuning and RX DSP controls are applied without constructing a TX
  DMA path.
- TX requests are refused and the model remains in the RX phase.
- SIGINT/SIGTERM perform explicit DDC shutdown, FIFO reset, rate clearing, and
  receive-safe RF cleanup.
- `/run/saturn-bridge/xdma-ready.json` is atomically refreshed with DMA, IQ,
  framing, FIFO, and RF-safety counters.

The production gate remains closed until this exact runtime passes the
appliance client/retune/disconnect tests, transactional switching in both
directions, and failure-injection rollback. Direct antenna relay control is
also intentionally not claimed by this first runtime slice. Protocol 2 remains
the production default.

Build the standalone development binary and run the bounded receive-only
appliance smoke test with:

```bash
CARGO_BUILD_JOBS=1 cargo build \
  --manifest-path update_manager/saturn-bridge/Cargo.toml

sudo update_manager/scripts/saturn-xdma-operational-rx-smoke.sh
```

The harness refuses a binary older than its Rust/build inputs, stops both
possible radio owners, and requires an observed `ready` heartbeat with
advancing DMA and IQ counters. For client acceptance it calculates the
steady-state rate between a post-TX receive-safe heartbeat and the final stopped
heartbeat and requires it to remain within two percent of 384 kHz. After the bounded SIGTERM, it also requires the
final `stopped` heartbeat and explicit receive-safe cleanup log before restoring
only the services that were active before the test. It leaves the observed
ready JSON, final stopped JSON, and runtime log under `/tmp` for inspection.
It does not open the production backend gate.

The initial 15-second PCB2/firmware-1.27 appliance run passed. The debug build
spent part of the bounded wall time initializing, but its ready-to-stopped
interval delivered 1,836,648 IQ pairs in 9.567 seconds (approximately 191,975
pairs/second), with no header resynchronization, header error, or FIFO fault.
The runtime then disabled DDC, verified receive-safe cleanup, and restored both
previously active services.

Client acceptance is enabled with:

```bash
CARGO_BUILD_JOBS=1 cargo build --release \
  --manifest-path update_manager/saturn-bridge/Cargo.toml

sudo update_manager/scripts/saturn-xdma-operational-rx-smoke.sh \
  --client-probe \
  --duration-seconds 45
```

The localhost-only client opens the actual TCI WebSocket, validates 384 kHz IQ
and 48 kHz audio binary frames, submits an eight-command RX DSP preference
burst, requires media and DMA progress afterward, retunes both VFO and DDC to
7.200 MHz, and exercises the DUC with RF explicitly inhibited. The default is
five complete TX cycles; every cycle must perform a fresh DUC mux reset,
advance H2C writes and TX frames without a FIFO fault, and return to a stopped,
receive-safe stream state while MOX, TX enable, and the PA relay remain off.
Use `--tx-cycles COUNT` to select 1 through 20 cycles. After successful cleanup
and service restoration, the harness atomically records the result in
`/var/lib/saturn-state/xdma-telemetry.json` for the Radio Telemetry page.

The first two client runs exposed a genuine debug-build starvation path: the
runtime synchronized WDSP and published the complete radio snapshot after each
stream-control command, allowing the DDC FIFO to reach its 16,384-word
threshold. The operational loop now drains DMA before bounded control work,
handles at most eight commands per slice, synchronizes WDSP only for actual DSP
changes, and publishes targeted tuning/TX acknowledgements instead of flooding
the complete state.

The subsequent 45-second PCB2/firmware-1.27 client run passed:

- 191,992 steady-state IQ pairs/second over 39.609 seconds
- 37 delivered IQ frames and 62 delivered audio frames
- continued IQ, audio, and DMA progress after the DSP preference burst
- 30.048 ms worst reported DSP control batch
- FIFO high-water of 8,016 words, with zero FIFO or framing faults
- successful 7.200 MHz retune
- historical Phase 6 TX refusal before the Phase 7 production TX implementation
- verified DDC shutdown, receive-safe cleanup, and restoration of both
  previously active services

The same acceptance can be routed through Saturn Go's production TLS split
proxy instead of connecting directly to the bridge:

```bash
sudo update_manager/scripts/saturn-xdma-operational-rx-smoke.sh \
  --proxy-client-probe \
  --duration-seconds 45
```

The client modes default to `target/release/saturn-bridge`; the receive-only
mode above continues to default to the development binary.

This opens paired `/saturn/control` and `/saturn/media` WebSockets on the
localhost port-8443 listener, supplies the configured Basic credential without
printing it, sends the same `session_open` command as the browser, and proves
pairing by requiring text exclusively on the control lane while IQ/audio arrive
exclusively on the media lane. The localhost self-signed certificate bypass is
scoped only to this bounded appliance test.

After the split-proxy test passes, exercise the actual ownership transaction:

```bash
sudo update_manager/scripts/saturn-xdma-backend-switch-smoke.sh
```

That harness requires the appliance to begin with stable P2 ownership. It
rejects a stale source-tree binary and stages the tested binary in a temporary
root-owned directory under `/opt/saturn-go`. That location remains executable
inside the production unit's `ProtectHome=yes` namespace while avoiding the
appliance's `noexec` `/run` mount. It installs a temporary runtime override,
then invokes the root transaction broker under its explicit test gate,
validates `P2 -> XDMA`, runs the split-proxy client acceptance, validates
`XDMA -> P2`, and restores the exact prior backend drop-in, persisted selection,
readiness file, and service state. On failure it prints bounded
unit/readiness/journal diagnostics before restoration. RF remains inhibited
throughout this transaction test.

## Phase 7 production RX/TX backend

The direct runtime now reuses the same TCI operator lease, WDSP TX engine,
microphone path, TX display, EQ, CFC, phase rotator, noise gate, and watchdog
state machine as the P2 transport. Its backend-specific output packs WDSP's
192 kHz IQ as Q-then-I signed 24-bit big-endian samples and writes H2C0 with
the proven Phase 4 FIFO geometry.

The initial production envelope is intentionally narrow:

- primary Saturn PCB2 firmware 1.27 only
- ANT1 with the same band-filter mapping as P2app
- 3 W maximum direct-XDMA target
- PureSignal and RF-generating two-tone disabled
- DUC FIFO prefill before MOX
- immediate receive cleanup on DMA/FIFO/register error, SWR or reverse-power
  trip, operator disconnect, stale microphone, stale control, or process exit

P2app is retained and remains the default owner for Thetis and other Protocol 2
clients. Direct XDMA is an explicit alternative for Saturn Remote and other TCI
clients. The Radio Telemetry page calls the root-owned transaction broker; only
one backend can own the FPGA, and readiness failure restores the previous
service, systemd drop-in, and persisted selection.

RF-inhibited acceptance sets `SATURN_REMOTE_TX_RF_ENABLED=0`, arms a two-tone
diagnostic, and requires advancing DUC DMA/FIFO counters while MOX, TX enable,
and the PA relay remain off. Actual RF validation is a separate,
operator-approved dummy-load test. Its locked harness mode is:

```bash
sudo env \
  SATURN_XDMA_PRODUCTION_TX_CONFIRM=DUMMY_LOAD_CONNECTED_ANT1_7200000HZ_3W \
  SATURN_XDMA_RX_SMOKE_BRIDGE_BINARY="$PWD/update_manager/saturn-bridge/target/release/saturn-bridge" \
  update_manager/scripts/saturn-xdma-operational-rx-smoke.sh \
    --client-probe --rf-tx-probe --duration-seconds 45
```

The RF mode is fixed at 7.200 MHz, ANT1, a 3 W drive target, and a 2.5-second
key window. It temporarily stops Saturn Go so an open browser cannot compete
for the operator lease, uses only the direct localhost TCI socket, and restores
the exact prior P2, bridge, and Saturn Go service activity.

The live PCB2/firmware-1.27 production-path acceptance on 2026-08-11 passed:

- 191,757 steady-state IQ pairs/second over 37.763 seconds
- 7,254,424 IQ pairs at receive-safe shutdown, with no RX overflow/underflow or
  framing faults
- nonzero IQ and audio over paired Saturn Go TLS control/media WebSockets
- 846 RF-inhibited DUC frames in 829 H2C writes, with no TX FIFO fault
- successful transactional `P2 -> XDMA -> P2` acceptance with the original P2
  selection and all three services restored

The RX FIFO threshold bit is a sticky high-watermark warning, matching P2app's
handling; it is counted and drained. Actual overflow or underflow remains a
fail-fast runtime fault. The DUC's single expected empty-stream startup
underflow is likewise cleared after verified prefill; all later DUC FIFO
conditions remain fatal.

Field key-up testing on 2026-08-13 exposed a timing assumption in that DUC
prefill: 3,039 words remained after the nominal fixed write, below the
3,240-word keying floor. Production prefill now reads live FIFO occupancy and
closes the measured deficit in bounded H2C batches before appending the first
live IQ frame. It still fails closed on overflow, threshold, post-startup
underflow, an exhausted retry bound, or insufficient final occupancy.

The direct backend also flushes four zero-input WDSP blocks at each arm and
requires eight consecutive IQ packets backed by a recently processed mic block
before MOX. These direct-only qualifications reject retained filter transients
without changing P2app's established TX behavior. Key readback now verifies the
DUC phase word and logs carrier, mode, filter edges, Q/I packing, and FIFO
occupancy. Any reported opposite-sideband offset must be reproduced with the
locked 1 kHz tone before changing IQ packing; the production path remains the
P2-compatible Q-then-I network-order contract.

Follow-up field testing on 2026-08-14 isolated the remaining alternating-key
static. The first clean transmission received a Voodoo profile update while
keyed and then failed closed on a DUC underflow. A later arm reported WDSP
output peak `0.5318` with zero input, proving that native filter/ALC state had
survived the prior MOX cycle. Direct XDMA now closes and recreates the native
WDSP TX channel on every arm, defers TX DSP model changes received while keyed
until the next arm, and targets 3,420 prefill words before the first live frame
to retain both underflow margin and FIFO ceiling headroom. P2 retains its
existing channel lifecycle and live model-update behavior.

A subsequent direct-backend A/B test produced a clear first key-up and static
on the second despite identical carrier/filter readback, clean browser mic
sequencing, and healthy DUC FIFO occupancy. The remaining startup difference
from P2_app was the FPGA's 64-to-48-bit DUC multiplexer: P2 disables and pulses
that mux reset before resetting the FIFO, while the direct path had reset only
the FIFO. Direct XDMA now performs the complete P2-proven mux/FIFO reset
sequence on every arm and refuses to key if readback reports mux reset or IQ
deinterleave still asserted.

The post-fix field retest on 2026-08-14 completed four consecutive Voodoo 3.8k
voice transmissions at 7.210 MHz with clear received audio and no recurrence
of static. All four arms recreated and settled WDSP, keyed with identical LSB
carrier/filter/phase/Q-I readback, and returned safely to RX. Browser mic input
and the Bridge mic queue had no gaps or underruns; the DUC FIFO remained between
1,694 and 3,737 words with zero runtime FIFO faults, while RX reported zero
header errors or resynchronizations. The four recorded DUC startup underflows
were the expected sticky empty-FIFO indications cleared during the four
deliberate reset/prefill cycles.

The operator-approved production RF run on 2026-08-11 passed on a 50-ohm dummy
load. A 1 kHz TCI microphone tone keyed for 2.5 seconds at the 3 W drive target:

- 0.698 W peak observed forward power, 0.000 W reverse power, and 1.00 SWR
- 1,910 DUC frames in 1,893 H2C writes
- 2,231-word DUC low-water and 3,554-word high-water marks
- zero RX hard faults, TX FIFO faults, or power/SWR trips
- explicit TX release, receive-safe DDC/DUC cleanup, 191,877 IQ pairs/second,
  and restoration of P2, Saturn Bridge, and Saturn Go

Two earlier bounded attempts proved the fail-closed paths before this pass. An
artificial post-key catch-up burst reached the DUC high threshold and unkeyed;
the scheduler now resets its packet deadline at the key transition. A debug
build then underflowed because it could not deliver microphone/DSP frames in
real time and also unkeyed; the production RF gate therefore uses the optimized
release binary. Neither attempt sustained RF or bypassed a safety condition.

## Production Data Paths

| Function | XDMA node |
| --- | --- |
| FPGA registers | `/dev/xdma0_user` |
| DDC receive IQ | `/dev/xdma0_c2h_0` |
| Microphone input | `/dev/xdma0_c2h_1` |
| DUC transmit IQ | `/dev/xdma0_h2c_0` |
| Speaker output | `/dev/xdma0_h2c_1` |

## Migration Gates

1. Phase 1: identity, compatibility, ownership, and safe shutdown
2. RX-only DDC IQ with FIFO and framing telemetry
3. Microphone and speaker DMA
4. DUC IQ with RF forcibly disabled
5. Guarded RF transmit and failure-injection tests
6. Transactional client-selectable switching and rollback
7. Production direct RX/TX behind explicit operator selection, with P2 default
8. Long-duration and dummy-load acceptance before wider power or XDMA default
