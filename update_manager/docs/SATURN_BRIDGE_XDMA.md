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
- configures hardware DDC6 for ADC1 at 192 kHz
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
  eleven 1.25 ms frames (H2C0 discards writes while the mux is disabled)
- refills at seven frames toward an eleven-frame target; this keeps the normal
  transfer near four frames while reserving enough FIFO for observed 5–7 ms
  H2C kernel-write tails that cannot be predicted before a refill starts
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
bounded stress source. A failed run stops on its first FIFO fault or critical
low-water observation, prints the probe telemetry and stressor log tails, and
restores `p2app.service`. The storage profile performs read-only random I/O
against the repository's block device; all temporary writes and logs remain in
`/dev/shm`.

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
- `transfer_latency_warn_us=5000` logs synchronous transfers at or above 5 ms
  with separate page-pin/scatterlist, submit/completion-wait, and cleanup
  timings. It defaults to `0` and can be adjusted at runtime.

These options do not change XDMA device names, transfer semantics, or the
Protocol 2/direct-XDMA selection model. A persistent pre-pinned DMA ring remains
the preferred later optimization if completion prioritization alone does not
meet the Phase 4 gates.

## Planned Data Paths

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
7. Long-duration soak testing before considering XDMA as the default
