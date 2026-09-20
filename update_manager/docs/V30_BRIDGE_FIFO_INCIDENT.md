# V30 Direct-XDMA bridge FIFO incident

Status: repair candidate, not hardware-qualified

Observed: 2026-09-15 on `saturn-g2`

FPGA artifact under test: `saturn-primary-v30-a904f42b.bin`
FPGA source commit: `a904f42b1dcc3e23a08b4af604983f90a161453b`

## Symptom

Saturn Remote's split control WebSocket opened and immediately closed with
code 1005; the media lane then failed with code 1006. TLS and authentication
were successful. Saturn Go closed the browser-facing sockets because its
upstream `saturn-bridge` process exited.

The decisive bridge error was:

```text
operational XDMA RX FIFO fault: depth=16385 overflow=1 threshold=1 underflow=0
```

One reproduction processed an eight-command browser startup batch for 54,058
microseconds immediately before this fault. The operational bridge serviced
DDC C2H reads, WDSP, WebSocket publication, and client control in one loop.
The control batch therefore withheld C2H service for approximately the fill
time of the 16,384-word DDC FIFO at 384 ksps.

The G2 checkout was commit `24e8f5c8cebd517e7ddc726cb1506e148c3ff976`.
Its bridge admitted firmware 1.30 but still used the older unconditional
`overflow || underflow` fatal rule. It did not contain commit `f322c15`
(`Handle V29+ RX FIFO status correctly`). The installed bridge SHA-256 was
`ca2c859d91ca7827339c67722818095bbf48a7ad1958bdcaba416243c2b4dd1a`.

## Why V27 appeared to work

V27's top-level block design disabled `HAS_AFULL` on the four monitored AXIS
FIFOs and tied all four `FIFO_Monitor` overflow inputs to constant zero. Its
legacy status bit 31 could consequently never assert. The old bridge could
pause long enough to fill the DDC FIFO without terminating on bit 31, although
that behavior did not prove that samples were preserved.

The first V30 compatibility attempt excluded the new `almost_full` inputs from
legacy bit 31 but synthesized that bit from `count >= configured depth`. That
was not the exact V27 host contract. When control work starved C2H and the DDC
count reached 16,385, V30 asserted bit 31 and the deployed legacy bridge exited.

## Repair contract

The repair has two independent requirements:

1. V30 legacy status must exactly preserve V27's observable boundary. Legacy
   bit 31 remains zero; bits 29 and 30 retain their V27 read-to-clear behavior.
   Almost-full and configured-capacity observations remain available through
   the extended minimum, maximum, and transition telemetry.
2. The bridge must not depend on hiding a full FIFO. A dedicated priority-22
   C2H owner continuously drains `/dev/xdma0_c2h_0` into a preallocated,
   page-aligned, locked 256-buffer/8 MiB ring. WDSP, WebSocket, filesystem, and
   control work consume that ring independently. If the consumer exhausts the
   bounded reserve, the oldest unread host buffer is discarded and explicitly
   counted so the hardware FIFO remains serviced.

V29 retains its version-specific legacy status interpretation. V30 uses the
restored V27 legacy contract. Direct RF transmission remains inhibited for
unqualified V28/V29/V30 firmware.

## Required qualification

The repair is not qualified merely because Vivado timing and implementation
gates pass. Before release it requires an end-to-end G2 test with the Direct-
XDMA backend at 384 ksps that:

- opens both split WebSocket lanes and sends the normal browser startup burst;
- exercises repeated connect, disconnect, and reconnect cycles;
- verifies `saturn-bridge.service` never restarts;
- verifies FPGA FIFO extrema/events and host-ring drop/discontinuity counters;
- completes controlled RX and antenna RX soaks;
- keeps TX inhibited until a separate dummy-load TX qualification.

The original V30 artifact and logs remain historical evidence. A repaired
artifact must carry a new Git SHA and manifest and must not overwrite the
`a904f42b` build.

## Repaired-FPGA field follow-up

The repaired FPGA artifact built from commit
`104d5c569054909aef0de27a3458844d0fd4df17` passed implementation with
WNS `+0.119 ns`, WHS `+0.049 ns`, and all DRC, CDC, methodology, and telemetry
netlist gates clear. Its primary-slot image is
`saturn-primary-v30-104d5c56.bin`, SHA-256
`d80eebf7eb126e23e9922bf05c155a5a7bfa29f190a4eb0d850b0cd1d5f82974`.

After that FPGA was loaded, the browser completed one split-WebSocket session
and received 384 kHz IQ, confirming that TLS, authentication, proxy routing,
split-lane pairing, and V30 DDC output could all operate. A later reboot still
used the old installed bridge binary
`ca2c859d91ca7827339c67722818095bbf48a7ad1958bdcaba416243c2b4dd1a` and the
old service ceiling `LimitRTPRIO=21`. That bridge repeatedly exited with:

```text
operational XDMA RX FIFO remained over threshold after 16 bounded startup drains
```

Systemd restarted it five times and then marked `saturn-bridge.service`
failed. `saturn-go.service` remained active and its subsequent proxy attempts
failed with connection refused because no bridge listener remained. This is
the expected incomplete state when only the FPGA half of the two-part repair
is installed; it is not evidence that the repaired FPGA reintroduced the
original bit-31 failure. Hardware qualification begins only after the matching
priority-22 dedicated-reader bridge and service unit are deployed.

## Direct-RX byte-order field follow-up

Deploying the matching priority-22 bridge allowed split control and media
lanes to remain connected, but received audio and spectrum were unusable
full-scale noise with no discernible stations. This was a separate Bridge
ownership bug, not another V30 FIFO-monitor failure.

Read-only capture through the Bridge's TCI IQ output showed ten consecutive
frames at approximately `0.57` to `0.60` normalized RMS (`-4.9` to `-4.4`
dBFS), with peaks repeatedly reaching `0.999`. Reinterpreting the exact same
24-bit sample bytes in the opposite order produced approximately `0.014` to
`0.027` RMS (`-37.2` to `-31.4` dBFS). The stream had no header errors,
resynchronizations, FIFO faults, host-ring drops, or discontinuities.

P2 establishes the required representation during startup with
`SetByteSwapping(true)`, which sets RF GPIO bit 26 before DDC operation. The
Direct-XDMA RX path decoded every sample as signed 24-bit network byte order
but never set or verified that global FPGA bit. Earlier V27 tests could inherit
the correct bit from a preceding P2 owner; a cold boot or ownership sequence
that did not run P2 left the power-on local byte order active. The resulting
byte-reversed sample magnitudes looked like near-full-scale random noise even
though DMA framing remained structurally valid.

The Bridge repair now sets and reads back RF GPIO bit 26 before enabling either
the probe or operational DDC stream. Failure to establish the byte order is
fatal before samples are published. Correcting the sample representation also
removes the false near-full-scale input that drove WDSP's meter to the observed
`S9+51` level; ordinary per-radio S-meter calibration remains a separate trim.

## Direct-RX performance and provenance follow-up

After the byte-order repair restored usable stations, the first Web Manager
capture incorrectly reported `client=n/a`, `connections=n/a`, no application
telemetry, and "waiting for a Protocol 2 client" while a paired split control/
media session was actually connected. The direct backend emitted only its
five-second `xdma status` line; Web Manager still inferred bridge activity from
the one-second `saturn-bridge: diag` line emitted by the Protocol 2 backend.
This made the displayed workload and provenance false, not merely incomplete.

Read-only process attribution on the G2 also found approximately 55% of one
core in the bridge during the observed workload. The largest worker consumed
about 29%, the main thread about 9%, and the dedicated XDMA reader about 7%.
Source review showed that direct RX called `WdspRxEngine::push_iq()` for every
384 kHz sample block before `publish_audio_frame()` checked whether any client
had requested audio. The hardware drain and framing checks were necessary;
unconditional floating-point expansion and WDSP audio work were not.

The repaired runtime preserves the quality boundary explicitly:

- an audio consumer receives the unchanged full-rate decoded IQ -> WDSP ->
  audio path;
- an IQ-only consumer receives every decoded IQ sample, but WDSP audio work is
  bypassed;
- with no media consumer, the dedicated C2H owner continues draining the same
  bounded ring and the parser continues validating headers, sequence, frame
  layout, and loss counters without expanding every 24-bit sample to `f32`;
- one decoded block every 100 ms maintains a labeled raw-IQ meter estimate;
- after any intentional WDSP input gap, the channel is cleanly down-slewed,
  flushed, and restarted before audio processing resumes so stale AGC, NR, or
  filter history cannot leak into the new stream.

The bridge now writes `/run/saturn-bridge/perf.json` atomically every second.
It includes the service PID, exact Saturn build commit and dirty state, pinned
WDSP flavor and source commit, FPGA product/PCB/firmware/date/clock identity,
client and split-lane state, processing mode, DMA/IQ/audio rates, queue depths,
drops, framing/loss counters, and TX state. Web Manager accepts it only when
the schema/source/backend are correct, it is no more than five seconds old,
and its PID matches `saturn-bridge.service`; older deployments retain the
journal fallback.

Qualification must measure three workloads separately rather than treating a
lower idle CPU number as an audio-quality result:

1. active IQ plus audio: no regression in received stations, spectrum, audio,
   S-meter behavior, drops, discontinuities, or FIFO/framing counters;
2. IQ-only: full-rate spectrum with zero WDSP-audio processing and a material
   CPU reduction from the active-audio baseline;
3. no-client standby: continuously advancing DMA/IQ validation, zero hardware
   or host-ring loss, approximately 10 Hz meter decoding, and a material CPU
   reduction from both active modes.

No claimed performance gain is considered accepted until those appliance
measurements are captured with the new provenance fields.

The first active-audio capture from commit `40a208f` correctly identified the
binary and workload but also exposed two follow-up defects. P2-only counters
that did not exist in the direct backend were rendered as zero, while the
direct host-ring counters were visible only as boot/process-lifetime gauge
values. More importantly, the lifetime values showed a full 256-buffer host
ring, 95 reclaimed buffers (389120 bytes), four parser discontinuities, and two
header resynchronizations. A single capture cannot establish whether those
events were confined to startup or still increasing, but it is not a passing
quality result.

The follow-up maps the direct counters into explicit cumulative application
counters so Performance Lab can calculate interval deltas and fail on new loss
rather than on old lifetime history. It also samples and presents the existing
marker-gated V29 FIFO and V30 ADC banks from the exclusive direct-XDMA register
owner. To reduce diagnostic interference, ephemeral `/run` snapshots keep
atomic rename but omit crash-durability `fsync`, the duplicate immediate
startup writes are removed, and the compatibility journal line runs every five
seconds while the authoritative file remains at one-second cadence. A new
appliance run must demonstrate zero deltas for buffer drops, discontinuities,
pool starvation, and FIFO faults before performance qualification continues.

Code review then identified a matching producer/consumer failure mechanism.
When an audio client arrived after an intentional IQ-only or idle interval, the
direct backend restarted WDSP with `SetChannelState(channel, 0, 1)`. In this
single-threaded WDSP caller, `dmode=1` waits for exchange work that the same
blocked thread must supply; the pinned WDSP implementation times out before
taking its force-reset path. During that wait the dedicated XDMA reader keeps
producing buffers while the only ring consumer is stopped. This violates the
existing channel-state contract and can fill the 256-buffer ring, matching the
captured high-water mark and reclaimed-buffer loss.

Input-gap resume now uses `SetChannelState(channel, 0, 0)`, feeds a bounded set
of zero-input exchange blocks to complete the normal down-slew and native
buffer flush, then performs the state-1 up-slew. This is the same nonblocking
pattern already used for RX/TX suspension and rate changes. Process-lifetime
resume count, last/maximum elapsed microseconds, and flush failures are exported
so the appliance test can verify the corrected path directly.

## Web telemetry presentation follow-up

The Direct-XDMA backend already exported coherent V30 ADC episode telemetry and
V29 FIFO telemetry, but two prominent Radio Telemetry rows still consumed only
P2app's shared-memory schema. The ADC card therefore reported that it was
waiting for a Protocol 2 client, and the DUC queue row rendered `n/a`, even
while the bridge-owned telemetry was current and valid. These were frontend
schema-selection errors, not missing FPGA observations.

The ADC card and runtime row now select `fpga_adc_v30` whenever Direct-XDMA is
the active backend. The P2-only Enable/Disable controls are disabled and
explicitly identify V30 episode telemetry as always active. The Direct-XDMA DUC
row now combines the coherent V29 DUC occupancy snapshot with its boot-lifetime
minimum, maximum, and transition accumulators plus bridge TX stream/key state,
DMA writes, frames, FIFO low/high-water observations, faults, and startup
underflows. Host queue depth, age, and mode remain labeled uninstrumented rather
than being fabricated from unlike data.

The same review found that the Performance Lab's second poll could throw after
an optional-delta helper was declared inside one sibling block and referenced
from another. The fetch succeeded, but the page-level catch hid the JavaScript
`ReferenceError` behind a generic telemetry fault. The helper is now scoped for
both branches, and every inline template script is parsed and checked for
unresolved identifiers in CI. A separate undeclared timeout variable in the
keyed-transmit safety path was replaced with the configured, clamped transmit
duration and covered by a regression assertion.

## Bridge service-contract deployment follow-up

On 2026-09-16 a Saturn Go self-deploy installed the current bridge binary but
regenerated `saturn-bridge.service` through the older root-owned deployment
broker already present on the appliance. The source-tree broker and standalone
installer granted `LimitRTPRIO=22` and `LimitMEMLOCK=16M`; the installed broker
still generated priority 21 with systemd's 8 MiB locked-memory default. The new
Direct-XDMA bridge therefore exited before radio initialization with:

```text
could not lock aligned XDMA buffer in memory: Cannot allocate memory
```

This was not ordinary RAM exhaustion: the appliance had hundreds of MiB
available and no pages locked after each failed process exited. It was a
binary/systemd contract mismatch. The self-update trust boundary deliberately
does not replace its own root-owned broker from an unprivileged staged payload,
but it previously had no version handshake to prove that the installed broker
could deploy the source tree's bridge service contract.

The deployment broker now carries an explicit contract version. A bridge-
inclusive self-update fails before build or service mutation when the installed
and source broker versions differ; a Saturn Go/web-only update remains possible
with `SATURN_SATURNGO_BUILD_BRIDGE=0`. Both deployment paths also verify the
effective priority and locked-memory limits before accepting or starting the
new bridge. Updating the trusted broker remains an explicit privileged
installation step rather than executing staged root code.

## Performance baseline and first optimization

On 2026-09-17 the recovered bridge at commit `de5c6f0`, running active 384 kHz
IQ plus WDSP audio, consumed approximately 57.0% of one CM4 core over an
eight-second thread sample. WDSP worker `Wchan0` accounted for 30.6%, the main
bridge/control thread 10.9%, the dedicated XDMA reader 6.6%, and the remaining
WebSocket/TX workers 9.9%. The stream delivered approximately 384,445 IQ
pairs/second. Over a separate ten-second interval, host-buffer drops,
discontinuities, pool starvation, header errors/resynchronizations, FIFO
faults/thresholds, outbound drops, and WDSP resume failures all had zero delta.
Process-lifetime history still contained 50 reclaimed host buffers and two
discontinuities, so qualification remains delta-based until the next clean
candidate restart.

The first optimization deliberately changes no DSP setting or data-path
algorithm. Release builds target the appliance's Cortex-A72 for both Rust and
the pinned WDSP C archive, and Rust uses thin LTO with one codegen unit. Unsafe
floating-point reassociation is explicitly excluded. Telemetry exports the
selected CPU target so baseline and candidate binaries cannot be confused.
The initial acceptance target is active IQ plus audio below 45% of one core
with the existing 384 kHz IQ rate, 48 kHz stereo audio, filters, AGC, NR, FFT
configuration, and all integrity-counter deltas unchanged.

The deployed Cortex-A72 candidate at commit `de317d6` passed the integrity
gate but did not produce a material standalone CPU improvement. A 30-second
active IQ-plus-audio sample consumed 55.56% of one core: `Wchan0` 30.26%, the
main bridge thread 10.30%, the XDMA reader 6.07%, and all other workers 8.93%.
It sustained approximately 384,064 IQ pairs/second with zero interval deltas
for host-buffer drops, discontinuities, pool starvation, header errors or
resynchronizations, RX FIFO faults, outbound drops, and display/audio drops.
The compiler result is retained as a valid build improvement, but it is not
claimed as the requested performance gain.

The next isolated candidate removes allocation and queue-copy overhead around
the unchanged WDSP call. Interleaved `f32` IQ now converts directly into the
reused fixed-size `f64` WDSP input buffer instead of entering a `VecDeque` and
then being copied out. Stereo output fills one reusable frame buffer and is
published through a synchronous callback instead of allocating a new `Vec`
for every frame and pushing/popping every sample through a second deque. The
WDSP DSP size remains 64, output packet boundaries remain client-selected,
and all demodulator, filter, AGC, noise-reduction, FFT, sample-rate, and
floating-point behavior remains unchanged. This candidate must pass the same
active-workload quality counters and an appliance A/B measurement before it
can be accepted.

That allocation-free candidate sustained three consecutive 30-second active
IQ-plus-audio windows at 55.30%, 57.40%, and 55.76% of one core (56.15%
average), all at approximately 384 kHz with zero summed integrity deltas. Its
main-thread cost remained effectively unchanged, demonstrating that staging
allocation was not a material CPU contributor. The reusable buffers remain
useful for bounded allocation behavior, but no CPU gain is claimed.

The next candidate targets the dominant `Wchan0` cost without changing DSP
features. WDSP's RX exchange size increases from 64 to 256 samples, reducing
exchange and worker-wakeup frequency from 750 to 187.5 calls/second at the
48 kHz DSP rate. The 384 kHz hardware input and 48 kHz stereo output rates,
2048-float client audio packet boundary, FFT/filter/AGC/NR configuration, and
strict floating-point behavior are unchanged. The larger exchange adds about
4 ms of buffering relative to the 64-sample setting and is exported as
`wdsp_rx_dsp_size` so appliance results are attributable.

The deployed 256-sample candidate at commit `7417ad5` produced a repeatable,
modest gain. Three consecutive 30-second active IQ-plus-audio windows measured
53.30%, 55.00%, and 51.66% of one core (53.32% average), 4.0% below the matched
55.56% control and 6.4% below the original 56.98% baseline. `Wchan0` averaged
27.16% instead of the control's 30.26%. IQ remained approximately 384,500
pairs/second and audio approximately 96,250 stereo floats/second. All interval
deltas remained zero for host-buffer drops/bytes, discontinuities, pool
starvation, header errors/resynchronizations, RX FIFO faults/thresholds/almost-
full observations, outbound/audio/display drops, and WDSP resume-flush
failures. Automated quality and continuity therefore pass; subjective audio
latency and listening quality remain an operator acceptance item.

## Full-rate TCI IQ transport candidate

Source review after the 256-sample WDSP result found that Direct-XDMA decoded
approximately 384,000 complex pairs/second and called `publish_iq_frame` for
each roughly 4 KiB DMA block, but the shared TCI display limiter admitted only
about 30 of those calls per second. The browser therefore received periodic
small snapshots rather than a continuous IQ stream. The old `iq_frames_s`
field counted calls before that limiter and could report approximately 844
frames/second even though those frames did not reach the WebSocket.

The candidate packetizes every accepted Direct-XDMA RX sample in original
order into exactly 30 frames/second: 12,800 complex pairs, 25,600 `f32` values,
and 102,464 bytes including the 64-byte TCI header per frame. The packetizer
uses one fixed reusable buffer and bypasses the generic snapshot limiter only
after it has formed a complete frame. Tuning, loss of the IQ consumer, or TX
media-priority suppression clears an incomplete frame so a later frame cannot
mix RF centers or pre/post-session data. The bounded per-client display queue
remains the backpressure boundary; any replacement or send drop is explicit
failure evidence rather than silent rate limiting.

`iq_frames_s` now means completed TCI frames offered to at least one eligible
media client. The bridge additionally exports `iq_tci_frames_s`,
`iq_tci_pairs_s`, fixed packet geometry, pending pairs, suppressed frames and
pairs, plus display replacement, drop, and rate-limit rates. A passing active
RX interval requires approximately 30 frames/second and 384,000 pairs/second,
with suppressed/replaced/dropped/rate-limited values all zero. The expected
wire payload is approximately 3.07 MB/s (24.6 Mbit/s) before WebSocket/TCP
overhead, so this is a quality/transport candidate—not yet a CPU-performance
claim—and must be compared against commit `7417ad5` on the appliance.

Saturn Remote already accepts variable-size TCI IQ messages. Its render path
now retains one copy of each incoming frame and extracts the newest contiguous
FFT-sized window instead of allowing a large frame to make the FFT stride
across time. This preserves the existing 4096-sample spectrum analysis
boundary while the complete IQ stream remains available at the TCI boundary.
The Saturn Go split-WebSocket relay also moves the common `Bytes` payload
directly between Tungstenite and Axum, removing one full-frame allocation and
copy on each proxy hop.

### Full-rate TCI IQ soak evidence (2026-09-18)

Commit `ab51b13d519f7d6fb50d3aab16b64d34462ae8b3` ran continuously under an
active 384 kHz RX/IQ/audio workload with PID 1365058 from 2026-09-17 20:13:20
EDT. The captured 14,409,376,632 complex pairs represent 37,524.418 seconds
(10.423 hours) of full-rate input. A live sample reported 29.991 TCI frames/s,
383,886 transported pairs/s, the exact 12,800-pair / 102,464-byte geometry,
and zero suppressed, dropped, replaced, or rate-limited frames. Ethernet TX
was 26.28 Mbit/s, consistent with the expected 24.59 Mbit/s IQ payload plus
audio, control, WebSocket, TLS, and TCP overhead. The bridge remained on the
same PID with zero systemd restarts.

Process CPU time divided by the active-IQ duration gives a long-run estimate
of 51.03% of one Cortex-A72 core. This is 2.29 percentage points (4.29%) below
the prior 53.318% 256-sample candidate average while transporting the complete
IQ stream rather than periodic small snapshots. The dashboard's instantaneous
56.63% CPU value came from one interval after its browser baseline had reset
and is not the long-run average.

The strict continuous-loss gate is not yet a clean pass. The retained journal
window from 02:02:51 through 06:42:30 EDT contained 3,354 diagnostic samples
with zero outbound drops, display drops, display rate limiting, audio loss,
or command loss, but two intervals reported `display_replaced_s=1` (06:25:29
and 06:28:30). Each coincided with an incomplete local WebSocket handshake and
132-140 ms of accumulated send blocking; the evidence establishes correlation,
not causation. The process-lifetime counters also contained 136 host-ring drops
(557,056 bytes), four host discontinuities, and four header resynchronizations.
Those totals did not change during a subsequent read-only 20-second check, but
the earlier journal segment had already rotated, so their timing cannot be
classified as startup/session-boundary or steady-state loss. A repeat soak
must capture start/end counter deltas and retain the complete diagnostic log
before the overall zero-loss qualification can pass.

The follow-up transport revision separates continuous Direct-XDMA IQ from the
legacy latest-display-frame scheduler. Full-rate IQ now has an ordered bounded
FIFO of four 102,464-byte frames per eligible media client, covering roughly
133 ms at 30 frames/second with about 410 KiB maximum queued IQ memory per
client. Safety and control remain higher priority, RX audio remains ahead of
IQ, and snapshot/TX display traffic retains its depth-one replacement behavior.
An IQ frame that encounters a nonblocking socket write is requeued at the
front; concurrent arrivals remain ordered, and the newest frame is discarded
only if the four-frame bound is already full. That discard is an explicit
per-interval and cumulative qualification failure.

New process-lifetime telemetry records formed/published and suppressed IQ
frames/pairs, queue enqueued/written/dropped delivery totals, current depth,
high-water mark, and per-client capacity. These totals persist across browser
baseline resets and client reconnects for the lifetime of the bridge process.
The Performance Lab permanently raises a critical alert after any cumulative
full-rate queue loss, removing the prior dependence on catching a one-second
replacement pulse.

### Browser panadapter/waterfall response candidate (2026-09-19)

The operator reported slow display startup and a spectrum/waterfall that
appeared to trail RX audio. Source inspection on `v30-bridge-fifo-integration`
at `1ac4e2a` identified three browser costs independently of the ongoing
full-rate IQ transport qualification:

- `FftProcessor` applied a hard-coded 0.82 previous / 0.18 current temporal
  average before the user-selected spectrum average. Its 90% step settling
  takes 12 updates (400 ms at 30 Hz), even with the visible average set to one.
  Zero-filled initial dB bins also produced a false startup transition.
- Every waterfall row shifted and uploaded the entire 512-row RGBA texture.
  At 4096 bins this is an 8 MiB upload per line (240 MiB/s at 30 lines/s), plus
  the CPU history copy. These are calculated transfer volumes, not measured
  browser or G2 CPU results.
- Each received IQ frame called the full `updateUi()` synchronously.

The candidate removes the hidden FFT average and seeds the visible average
from the first real spectrum after startup, reset, or a bin-count change.
The window, FFT scaling, orientation, 4096-bin default, and IQ sample rate
remain unchanged. User averaging, peak hold, and waterfall cleanup remain
available. With averaging set to one, spectrum steps now appear on the next
processed frame rather than settling through a second hidden filter.

The WebGL waterfall stores history in a circular texture and uploads only
one row per update (16 KiB at 4096 bins, a 512-fold reduction in normal upload
volume). The shader addresses the circular history with clamped logical
edges and interpolation across the physical wrap. Full uploads remain for
clear/resize and horizontal tuning shifts. Canvas2D now consumes each pending
row once; redraws cannot duplicate a row or replay a cleared pending line.
Classic and Ember color ramps no longer jump at their segment boundaries.
This improves gradient continuity without claiming increased hardware color
depth.

Routine IQ-triggered status updates are coalesced to at most four per second;
source/rate changes request an immediate animation-frame refresh. IQ samples,
frame counts, FFT rendering, and control-event refreshes continue independently.

`window.SaturnRemotePerf.snapshot().displayPipeline` and the existing sample
export include rolling p95/p99/max FFT and CPU draw-submission times, newest-IQ
browser-arrival-to-draw time, first-IQ-to-draw time, FFT size, and renderer
backend. These are browser-local measurements; they exclude FPGA/network
sample age and GPU presentation time. They cannot establish the audio/display
offset by themselves. A matched on-radio browser capture remains necessary.

Local regression coverage executes the actual template averaging and IQ
handler, and checks FFT silence/tone levels and immediate transitions.
`npm run validate:waterfall` executes the shipped renderer in headless Chrome
with software WebGL2, reads pixels across a full ring wrap, checks scaled
interpolation and tuning shifts, verifies single-row uploads, and exercises
the Canvas2D fallback. It fails if WebGL2 is unavailable. The separate layout
validation covers all 22 phone/tablet/desktop scenarios.

Validation completed: 459 tests in 58 files passed; TypeScript type checking,
production bundle build, template/bundle seam and scope checks passed. All 22
layout scenarios and the six real-browser waterfall checks passed, including
pixel-level interpolation at the circular texture seam. These results verify
local behavior, not a live G2 latency improvement.

Deployment must publish both `saturn-remote-next.html` and the matching rebuilt
`saturn-remote-next.js` bundle/checksum through the web deployment path. This
candidate requires no FPGA rebuild or bridge binary change. The G2 has not
been modified as part of this browser change. Full-rate IQ queue-loss soak
qualification remains separate and is not resolved by these display changes.

Next acceptance: capture matching browser performance samples before/after
with the same RF frequency, zoom, average, cleanup, browser, and viewport;
check startup response, tuning, audio continuity, and transport counter deltas.
Use those timings to decide whether an FFT worker or selectable larger FFTs
are warranted. Bridge-generated spectrum transport remains a separate future
option rather than silently reducing the requested full-rate TCI IQ feed.

### 2026-09-19 — display profile and intermittent-stall diagnosis

The operator's deployed browser capture reports WebGL2, FFT size 2048,
FFT p99 0.24 ms, draw CPU p99 2.04 ms, newest-IQ arrival-to-draw p99
32.02 ms (maximum 173.62 ms), and first-IQ-to-draw 30.22 ms. These
measurements do not establish end-to-end latency or audio/display alignment.
They do not identify FFT computation as the cause of the isolated delay.

Read-only G2 inspection found `streamMode: "wan"` in
`/var/lib/saturn-state/remote_settings.json`; the bridge remained PID 1688709
with NRestarts=0. Persisted WAN mode selects the lower-resolution display
even on a LAN address. Browser-local settings can override persisted settings,
so the actual profile and all selection reasons must be captured in-browser.
For local operation, Setup → Network → RX Transport → LAN selects the
existing higher-resolution profile and higher-bandwidth audio option, unless
another profile trigger such as fresh high RTT applies. No live settings,
services, or assets were changed during this investigation.

The diagnostics drawer now reports actual/target FFT size, bin spacing,
profile selection reasons, and browser-local FFT/draw/arrival timing.
Copy Network Diagnostics includes that snapshot plus the last 16 draws with
at least 100 ms animation gap, processing time, or arrival-to-draw time.
Each event includes visibility, audio queue/underruns, and bridge queue context.
This instrumentation does not change IQ rate, FFT resolution policy, audio
processing, or rendering effects. Browser RAF/main-thread timing and audio
latency estimates are context, not a shared-clock synchronization measurement.

Next field check: use the same station/browser with the LAN setting, compare
actual FFT size and display response, and copy diagnostics if a stall occurs.
Do not declare the 173.62 ms outlier fixed without that capture.

Validation: 465 tests in 59 files passed, including six executable profile
and slow-frame regression tests. Type checking, template seam/scope checks,
production build, and all 22 browser layout scenarios passed. Changes are
web-only and require deployment before the new diagnostics appear on the G2.

### 2026-09-19 — live RX performance baseline

Read-only sampling, no service restart or settings changes. PID 1688709,
bridge build 1ac4e2a254759f474a962453c26e3cf3f50e0234, WDSP 2.00,
384 kHz RX at 3.910 MHz, one client/two split connections, IQ and audio on.
Ten-second pidstat average: process 54.25% of one CPU; Wchan0 26.07%,
main thread 10.59%, XDMA reader 6.79%, two client threads 2.80% and 5.69%.
Reader voluntary context switches approximately 4457/s, main 2900/s;
idle TX and PURESIGNAL threads each approximately 984/s. Source inspection
finds 250 us poll sleeps in both the reader and main RX loop.

Unprivileged `perf record -e cpu-clock:u -F 99 -p 1688709 -o - -- sleep 20`
piped directly to `perf report --stdio -i - --no-children` collected 816
samples with zero lost samples. Largest sampled user-space symbols:
xHBResampler 26.47%, xfircore 5.27%, xnob 5.27%, xanf 4.53%,
getKey 3.55%, LambdaD 3.19%, DdcStreamParser::feed_inner 2.82%.
Several libc offsets remained unresolved. This is a short flat user-space
profile, not call-chain attribution or kernel CPU measurement; percentages
are sample shares, not total machine CPU or guaranteed achievable savings.
Privileged perf was unavailable because sudo requires a password. No kernel
permission settings were changed. User-only perf stat context-switch zeros
are not valid evidence of no switches; use pidstat for that measurement.

Separate telemetry interval 1789819463036–1789819478049 ms: same PID,
12667 DMA reads and 5764896 IQ pairs, no increments in host drops,
discontinuities, header errors/resyncs, FIFO faults, IQ queue drops, or ADC1
episodes/high clocks. Current audio_dropped_s was zero. Lifetime IQ queue
drops were 17 and ADC1 episodes 362736; their timing/cause is not established
by this short capture. Do not describe the entire soak as lossless.

Next investigation: inspect the pinned half-band resampler implementation
and its rate/configuration, benchmark equivalent ARM optimization with
signal-quality tests, then examine event-driven host buffer wakeups and
bounded reader polling. Preserve full-rate TCI IQ, filter/alias rejection,
audio bandwidth, and TX safety. Do not simply disable DSP features or
increase DMA batches without latency and FIFO-headroom measurements.

### 2026-09-19 — isolated half-band indexing candidate

The pinned reshb.c hot loop computes circular indices using repeated runtime
integer modulo operations. Index bounds allow replacing these expressions
with one conditional addition for negative indices. Added the standalone
`saturn-bridge/scripts/benchmark-hbres.py` runner, C harness, and usage notes.
The runner reads the exact pinned Git object, checks its SHA-256, and builds
both implementations in a temporary directory; the installed WDSP archive
and production build path remain unchanged.

Initial x86_64 GCC 11.4 -O3 timing of the 384k->48k cascade measured
4.73–4.88x kernel speedup over four alternating-order trials. This is NOT
a Cortex-A72 or whole-bridge speedup. Output bits and internal ring state
matched across 80 rate/block/in-place cases, six signal patterns, and 32
consecutive blocks per pattern. Exhaustive index checks cover pinned tap
counts 3, 19, 23, 27, 35, 319. AddressSanitizer and UndefinedBehaviorSanitizer
also passed. No coefficient, rate, floating-point operation order, or
sample-count changes were made by the candidate transformation.

Required next gate: isolated native Cortex-A72 equivalence and benchmark,
scheduled outside a live quality soak because the benchmark itself consumes
CPU. Only then integrate a source-guarded production patch and requalify
bridge performance, audio/RF behavior, and continuity. No G2 writes or
service changes were performed during this experiment.

### 2026-09-19 — native Cortex-A72 benchmark gate passed

Following operator approval, copied only the isolated benchmark runner/harness
to `/tmp/saturn-hbres-arm.p47ncW` on G2. Ran at nice 19 using Debian GCC
14.2.0 with `-O3 -mcpu=cortex-a72` against the pinned native source Git object.
No bridge archive/binary, radio configuration, or services were changed.
Unlike the initial plan to defer until outside the soak, this isolated test
ran alongside RX at low priority with five-second telemetry checks; this
interval must not be treated as an undisturbed soak baseline.

All exhaustive index checks and all 80 output/ring-state equivalence cases
passed on ARM. Four alternating-order 384k->48k kernel timings:

| Trial | Reference ms | Candidate ms | Speedup |
|---|---:|---:|---:|
| 0 | 2116.303 | 1149.806 | 1.84x |
| 1 | 2116.566 | 1149.444 | 1.84x |
| 2 | 2118.783 | 1149.965 | 1.84x |
| 3 | 2116.101 | 1149.641 | 1.84x |

Approximately 46% less elapsed time in this isolated kernel, NOT 46% less
whole-bridge CPU. Benchmark exit status was zero. Across telemetry timestamps
1789820184663 through 1789820214688 ms, PID stayed 1688709; deltas were zero
for host drops, discontinuities, header resync/errors, FIFO faults, IQ queue
drops, and ADC1 episodes. Each sampled audio_dropped_s was zero. These are
server-side counters, not an independent listening or browser-render test.

The candidate is now eligible for guarded build integration. It is still
absent from the production build path and has not been deployed. Next step:
share one source-hash-guarded transformation between benchmark and WDSP build,
test rejection on source drift, then perform a controlled bridge deployment
and matched before/after CPU and audio-quality soak. Retain the original
archive/binary for rollback before deployment.

### 2026-09-19 — guarded production build integration

Moved the benchmark transformation into shared `scripts/wdsp_hbres_index.py`.
The WDSP build validates its input before removing an existing build archive,
then transforms only the freshly copied build-directory `reshb.c`. Original
source SHA-256 is `3cd128001a52c31c84151052e444fa956ea8d67b259e1fbb0424492bf988918c`;
optimized output must be `45462ec55fcf5c8b241351ecbb2d8857b9248a04fb641d589c2796f808b6f255`.
Only CRLF-to-LF checkout normalization is allowed. Unknown source, changed
expressions/output, and double application fail closed. Build logs identify
the patch as `hbres-index-wrap-v1`. No floating-point flags, coefficients,
sample rates, safety policy, or Rust signal-processing code were changed.

Validation completed:

- Six executable guard/CLI tests passed, including preservation of an existing
  archive when the build receives unexpected source. Added to CI.
- Shared-transformation numerical benchmark passed all 80 configurations and
  exhaustive ring-index checks on x86; output hash is the exact ARM-tested
  candidate from the preceding entry.
- All 262 Rust bridge tests passed with native DSP stubs (not an RF test).
- Low-memory build contract, Bash syntax, and ShellCheck passed.
- Built the complete modified WDSP archive on G2 in isolated temporary staging
  at nice 19. All installer-required symbols plus xHBResampler and optimized
  source hash verified. No installed binaries, settings, or services changed.

Deployment/rollback and matched-soak instructions are in
`saturn-bridge/scripts/benchmark-hbres.md`. RF TX remains disabled for this
qualification. Production activation has NOT happened; end-to-end CPU and
audio-quality acceptance remain field-test gates, not established results.

### 2026-09-19 — post-integration RX profiling baseline

Read-only runtime inspection now confirms production build
`0c0e2e5e9053dd221a578c4457b23b588df3fb00`, clean build, PID 2437349,
NRestarts=0. This supersedes the preceding entry's activation status;
deployment occurred outside this profiling operation. Workload: one client,
two split connections, IQ+audio active, DDC6/ADC1 at 384 kHz, 7.200 MHz,
WDSP 2.00, TX unkeyed. No settings, binaries, or services were changed.

A 20-second `pidstat -t -u -w` baseline averaged 49.05% of one CPU core
(about 12.3% of four-core capacity): Wchan0 21.10%, main thread 10.10%,
XDMA reader 6.70%, two other bridge threads 6.05% and 3.00%, PureSignal
1.25%, TX thread 0.75%. Reader voluntary context switches averaged
4463.55/s; TX and PureSignal each averaged 985.05/s while TX was inactive.
The reader polls FIFO status and sleeps 250 us when less than a minimum
4 KiB DMA read is available. This identifies a wakeup optimization candidate,
not proof that increasing batch size is latency-neutral.

Unprivileged `perf record -F 99 -e cpu-clock:u` for 20 seconds collected
675 samples with zero lost samples. Data remains on G2 at
`/tmp/saturn-rx-profile.IBmd8V/perf.data`. Shares of sampled USER CPU:
xHBResampler 18.81%, xfircore 6.96%, xanb.part.0 4.44%, xemnr 4.00%,
getKey 3.70%, DdcStreamParser::feed_inner 3.11%. An unresolved libc address
0xa4094 accounts for 12.44%; nearby libc addresses also appear. Dynamic
symbol lookup does not identify these internal routines reliably. Obtain
matching symbols/callers before labeling them as copies or locking overhead.
These percentages exclude kernel CPU and are not whole-core utilization.
Sudo was unavailable noninteractively; no permission changes were made.
Unprivileged perf-stat context-switch zeros are not valid substitutes for
the nonzero pidstat thread measurements.

Telemetry endpoints at 1789836621038 and 1789836673085 ms retained the same
PID/build and zero host drops/discontinuities, header errors, IQ delivery
drops, and RX FIFO faults. Sampled audio_dropped_s was zero. ADC1 episode
count stayed 362736; high-clock total stayed 1438946. FPGA DDC/DUC event
totals stayed 64/113. Historical FPGA counts are not this process's errors.
This short interval does not certify all-day soak or audible quality.

Next priorities: resolve the libc hotspot, then benchmark additional
bit-equivalent resampler/parser changes and reduce reader polling overhead
with explicit latency/FIFO acceptance gates. Full-state publication still
runs in the RX control path (recent batches around 7–12 ms), making it a
separate tail-latency target. No matched old-build capture exists here, so
49.05% must not be presented as a proven percentage improvement over earlier
WAN/LAN or different-feature measurements. Preserve full-rate IQ, DSP
features, and TX safety behavior throughout further optimization.

### 2026-09-19 — resolved libc hotspot; smaller WebSocket read scratch

Disassembly of the installed G2 libc places offset 0xa4094 at `dc zva`
inside a zero-fill loop. Offsets around 0xa3180 are vector load/store copy
loops, not the unrelated preceding exported `__xpg_strerror_r` symbol.
A second 20-second 99 Hz user-CPU profile with 4096-byte DWARF stacks
collected 681 samples, zero lost, at
`/tmp/saturn-rx-callers.PcRlWF/perf.data`. The zero-fill hotspot accounted
for 11.89% of sampled user CPU and its caller was TCI `handle_client`.

Pinned Tungstenite 0.29.0 source explains the mechanism: `FrameCodec::read_in`
resizes its input buffer with zero fill before calling Read, then truncates
back after WouldBlock. Our 2 ms nonblocking client loop uses its default
128 KiB read scratch even for empty sockets. This is repeated initialization,
not DSP processing or the outgoing full-rate IQ payload.

Candidate sets only `WebSocketConfig.read_buffer_size` to 8 KiB. That reduces
scratch bytes exposed/initialized per empty attempt by 16x. It does NOT imply
a 16x application speedup or a measured CPU saving yet. Incoming frame and
message limits stay 256 KiB; larger messages assemble across multiple reads.
Outgoing IQ packetization, audio/DSP, socket polling interval, RF safeguards,
and DMA polling remain unchanged. Larger inbound messages may need more
read syscalls, so TX microphone/control traffic needs qualification too.

Five new executable tests exercise the actual pinned codec: 1000 empty
WouldBlock reads with bounded scratch, byte-exact maximum-size binary
delivery, fragmented 200000-byte delivery across 997-byte reads/WouldBlock,
oversized frame rejection, and oversized fragmented-message rejection.
All 267 bridge tests passed with native DSP stubs; formatting and diff checks
passed. This does not test native DSP/RF quality. Production remains build
0c0e2e5, PID 2437349. At telemetry timestamp 1789838185473 ms, host drops,
discontinuities, header errors, IQ delivery drops, and RX FIFO faults remained
zero; audio_dropped_s was zero and ADC1 episode count remained 362736.

Next field gate: deploy through the existing installer with RF TX disabled,
then compare matching LAN IQ+audio workload CPU/thread statistics, libc
zero-fill sample share, control latency, and continuity counters. Preserve
the current installed binary for rollback first. DMA polling changes are
deferred to isolate this directly measured, smaller-scope optimization.

### 2026-09-19 — smaller WebSocket buffer deployed and measured

Operator installation succeeded. Readback confirms clean build
6f4bf9f6b1b2f158368cee8060237a478df82637, PID 2463156, NRestarts=0,
ExecMainStatus=0. Installed and build-tree binaries both hash to
ba30ff051e0c6fa8aa02cc85dd69e3a692633c9b90b2e03668345c6d447ab8be.
Startup log confirms RF-inhibited operation. Workload remained ADC1/DDC6,
384 kHz, 7.200 MHz, one split client with IQ and audio enabled.

First 20-second pidstat sample overlapping a 99 Hz user-CPU profile averaged
45.25% of one core. A separate 20-second pidstat sample without perf recording
averaged 44.30% versus the preceding 49.05% baseline: 4.75 percentage points,
approximately 9.7% less CPU in these short observations. Combined WebSocket
client threads fell from 9.05% to 5.05%; WDSP Wchan0 stayed similar at 20.90%
versus 21.10%, and DMA reader at 6.55% versus 6.70%. This is encouraging
mechanism-consistent field evidence, not a controlled repeated A/B benchmark.

Profile `/tmp/saturn-rx-small-buffer.KDGRzS/perf.data` contains 588 user-CPU
samples, zero lost. The libc zero-fill address 0xa4094 fell from 12.44% of
samples in the earlier profile to 0.34%, now attributed to Wchan0 rather
than the WebSocket client. Resampler remains the largest named hotspot.

Important qualification exception: IQ delivery drop total is ONE, not zero.
The journal first reports it at 13:25:33, shortly after client connection
at 13:25:25–26. Its cause has not been established. It was already present
before profiling and stayed at one through telemetry timestamp 1789838850580.
From timestamp 1789838783514 through that endpoint, host drops/discontinuities,
header errors, RX FIFO faults stayed zero, sampled audio_dropped_s was zero,
and ADC1 episode count stayed 362736. Do not call this a zero-loss soak pass
or dismiss the IQ drop as harmless startup behavior. Continue observing
counter deltas and investigate reconnection queue pressure separately.

### 2026-09-19 — reconnect-drop investigation; send retry defect confirmed

No live reconnect, restart, settings change, or optimization was performed.
The journal narrows the first IQ drop to 13:25:28–13:25:33: zero at the
first diagnostic and one at the second. Both original split clients remain
connected; no send error or disconnect appears in this window. Client 3's
13:25:29 incomplete handshake never registers an outbound queue and cannot
itself discard a queued IQ delivery. Settings/control work in this window
includes 17 logged batches, summed elapsed time 231.047 ms, maximum batch
72.470 ms. These elapsed times are not all contiguous or CPU-only time.
The IQ queue reached its four-message bound. FPGA/host drop counters stayed
zero. This points to outbound queue pressure, not evidence of FPGA loss.

Found a separate, definite send-retry correctness defect relevant to that
pressure: `send_outbound` calls Tungstenite `WebSocket::send`. In pinned
0.29.0, a transport WouldBlock can occur AFTER the frame has entered the
library's write buffer. `handle_client` nevertheless requeues FullRateIq
(and safety messages). Later resend can deliver an already-buffered frame
again. `requeue_front` can additionally evict a newly queued IQ frame when
another frame arrives while the original is out of the application queue.

Added a deterministic test using the actual pinned WebSocket codec and a
blocked writer with a full-size 102464-byte TCI IQ message. Flushing after
unblocking delivers one message; resending after unblocking delivers two
byte-identical messages. The existing scheduler test independently confirms
that pop/enqueue/requeue at capacity increments IQ drops by one and removes
the newer frame. Both tests pass and demonstrate the mechanisms; they do
NOT prove which branch caused this historical field drop. Current counters
combine enqueue eviction, requeue eviction, failed sends, and queue teardown.
General outbound_drops is interval-reset and cannot exclude an earlier drop.

At telemetry timestamp 1789839407078, PID remained 2463156, enqueued=20420,
written=20419, dropped=1, pending=0; host drops/discontinuities and RX FIFO
faults stayed zero. Library write acceptance is not browser receipt, so these
counts cannot detect duplicate wire messages or certify end-to-end IQ order.

Recommended next correctness patch (not implemented in this investigation):
track library-owned pending writes and flush them on WouldBlock without
reenqueueing the same application message; distinguish WriteBufferFull,
where the library explicitly returns an unaccepted message. Preserve safety
priority and bounded backpressure. Add a deterministic full send-loop test
for partial writes, repeated WouldBlock, concurrent arrivals, exact-once
ordered IQ, and safety/close behavior. Add drop-reason counters/timestamps
before another controlled reconnect qualification. Do not simply enlarge
the queue or suppress the loss counter. Production code remains unchanged.

Validation: all 268 Rust tests passed with native DSP stubs; formatting and
diff checks passed. The added test is diagnostic coverage, not a deployed fix.

### 2026-09-19 — buffered-send correctness fix and reason telemetry

Implemented a per-socket BufferedSender shared by the production client loop
and deterministic codec tests. An Io/WouldBlock from send transfers the
application item into a single pending/library-owned slot; only flush may
complete that item. It is never requeued or passed to send a second time.
No subsequent application frame is submitted until that slot clears. The
loop still reads inbound commands while output is blocked, so inbound dekey
is not deliberately suspended. Safety priority remains at the next frame
boundary; a partially transmitted frame cannot safely be interrupted.
Close is considered sent only after its buffered write flushes.

WriteBufferFull is handled separately: Tungstenite explicitly rejected that
message, so it can be requeued. Existing application queue bounds remain;
a genuine overflow is still counted, not hidden. Send/flush fatal errors and
connection teardown account for any pending item exactly once. The existing
written count now includes a stalled item only after successful flush. It
means handed to the transport, never an acknowledgment from the browser.

Added cumulative metrics to `/run/saturn-bridge/perf.json`:

- `iq_tci_drop_queue_overflow_total`
- `iq_tci_drop_requeue_overflow_total` (only unaccepted WriteBufferFull retries)
- `iq_tci_drop_send_error_total`
- `iq_tci_drop_flush_error_total`
- `iq_tci_drop_connection_closed_total` (pending or queued at teardown)
- `iq_tci_last_drop_epoch_ms` (zero before the first drop)
- `iq_tci_in_flight_deliveries` (separate from application queue depth)

Existing aggregate and interval counters remain. The web server already
copies bridge metrics into `gauges.bridge`, so diagnostic JSON carries these
fields without a web deployment. Drop journal messages identify reason,
count, and epoch time; logs are limited to one per second while counters
remain exact. These are host-side accounting categories, not proof of
delivery or non-delivery at the browser. Atomic fields can be sampled across
an update; evaluate settled snapshots/deltas rather than demanding exact
conservation during a concurrent enqueue/write.

Six new tests cover partial writes/repeated WouldBlock with four concurrent
new IQ arrivals, byte-exact ordered delivery without duplicates, inbound
control during an output stall, safety priority, pending close flush,
WriteBufferFull retry, fatal send/flush, queued/pending teardown, per-reason
totals, and stalled audio/control messages. All 274 bridge tests passed with
native DSP stubs; cargo check, formatting, and diff checks passed. Cargo
check retains the pre-existing unused ClientOutbound constructor and stub
function warnings. No DSP, RF, FIFO/DMA, IQ rate, or performance tuning was
changed. This patch is not deployed; the historical drop cannot be assigned
retrospectively to one of the new reason counters.

Field acceptance remains: preserve rollback binary, install with RF TX
disabled, deliberately reconnect under operator control, and compare fresh
reason counters, sequence/continuity behavior, and audio. Do not call a
local deterministic test a completed live reconnect or RF qualification.

## September 19 field qualification and unresolved stalls

Deployed bridge: clean `6b5886698a3eb8f425ce3831e3cea54325879ed7`,
Cortex-A72, WDSP 2.00 with the existing pinned optimization. No additional
optimization, FPGA change, or deployment was performed during this review.

All times below are EDT on September 19, 2026. Full snapshots are retained
locally under `FPGA/lab/results/`; this tracked summary preserves their
important results independently of ignored lab output.

| Interval/event | Observation |
| --- | --- |
| Dummy-load baseline 16:44:45, PID 2480366 | 14.2 MHz, ADC1/DDC6, 384 kHz, split IQ/audio; historical connection-closed IQ drop=1; host losses=0; ADC1 episodes=365143 |
| Unattended event 18:16:35 | 22 new QueueOverflow IQ drops; first log epoch_ms=1789856195139, last drop epoch_ms=1789856195844; queue HWM reached 4; no host losses, header errors or RX FIFO faults; client-reported audio gaps increased 0 to 1 by 18:16:39 |
| Antenna interval 18:56:51–19:22:24 | 7.205 MHz, same process/build; 45972 additional IQ frames written, zero new drop reasons/host losses/ADC episodes; zero restarts; audio-gap total remained 1 |
| P2 switch / bridge return | Old bridge stopped at 19:22:53; new bridge started at 19:39:51 as PID 2629435; P2 inactive afterward |
| Remote connection failure after return | Browser reported ERR_CERT_AUTHORITY_INVALID before either lane opened; bridge RX was ready with zero subscribers. Fresh tab restored connectivity. Served certificate was self-signed and lacked IP SAN 192.168.0.139. Exact browser exception-state behavior not established; this is separate from streaming drops |
| Tune/start event 19:45:10 | iq_start/audio_start and settings sequence, frequency 14.2 to 7.205 MHz; one QueueOverflow IQ drop; first subsequent host report had 15 dropped buffers / 61440 bytes, one discontinuity and one header resync |
| Restarted antenna baseline 19:49:41 | PID 2629435, includes those historical one IQ drop and 15 host drops; ADC1 episodes=365273 |
| Check 21:09:33 | Same PID/build; 143772 additional IQ frames written, zero NEW IQ drops, host losses, discontinuities, header errors/resyncs, FIFO faults or ADC episodes; audio gaps=0 and zero restarts |

The 80-minute clean segment is a steady-state RX success, not proof that the
earlier unattended or tune/start failures are fixed. The operator was absent
during the unattended event and reported no intentional client/network action.

### Investigation findings (no code fix asserted)

1. The full-rate IQ application queue holds four frames (about 133 ms at
   30 frames/s, excluding in-flight and downstream buffers). QueueOverflow
   removes oldest queued frames. The 22 drops identify where loss occurred,
   not why the consumer stalled. Requeue/send/flush-error counters stayed zero.
2. `send_blocked_ms` increments by synthetic 2 ms steps at selected paths;
   it is NOT measured wall-clock stall duration. A small value cannot exclude
   a long scheduling pause or serialization/lock delay.
3. `audio_gaps` aggregates client-reported `audio_seq_gap_count`; it is not
   a direct measurement of speaker silence or FPGA sample loss.
4. Operational RX drains at most 16 host buffers per loop, then executes up
   to eight commands and conditional DSP synchronization/publication on the
   same consumer loop. The independent DMA reader continues filling the ring.
   At approximately 844 four-KiB reads/s, 16 reads represent about 19 ms of
   incoming data and 256 buffer slots roughly 303 ms when initially empty.
   Actual headroom depends on current occupancy and transfer sizes.
5. The 19:45:10 command burst included a 109100 us DSP-sync batch and a
   40711 us batch plus multiple smaller batches. This establishes a consumer
   service-budget risk consistent with host-ring saturation. Batch elapsed
   time includes command handling, DSP sync and publication; it does not
   isolate which operation incurred the delay. `sync_model` already checks
   individual setting changes, so do not assume every sync rebuilds filters.
6. The 18:16 incident had no corresponding DSP/tuning burst in the inspected
   logs. A radio-state publication batch took 26887 us immediately before the
   drop, but no host-ring loss occurred. Do not assign both events one cause.
   Kernel journal query for 18:16:20–18:16:50 returned no entries; this does
   not rule out transient network, scheduler, proxy or browser problems.

Next: supervised reconnect/tune reproduction with synchronized browser and
bridge evidence. If existing data is insufficient, add narrowly scoped actual
send/flush age, client-loop delay, queue oldest-age and command/DSP/publication
timings. Preserve TX inhibit, FIFO/signal rates and rollback artifact. Do not
increase buffers merely to hide losses or call this fully qualified yet.

## Supervised reproduction and diagnostic-only follow-up

September 19, 2026 (EDT), same deployed 6b58866 build and PID 2629435:

- Operator confirmed two deliberate reconnects: disconnect 21:19:43,
  reconnect 21:19:48, IQ at 21:19:49; disconnect 21:20:12,
  reconnect 21:20:18, IQ at 21:20:19. No new IQ drop reasons, host-buffer
  losses, discontinuities, resyncs, header errors or FIFO faults. Two WDSP
  resumes, no resume-flush failures, and no service restart.
- Supervised band recalls at 21:22:05 and 21:22:24, followed by tuning
  through 21:22:50, also produced no new losses. The largest observed
  control batches were 74731 and 78400 us. End frequency was 7.200 MHz;
  audio-gap count remained zero and TX remained inhibited.
- Operator reported markedly faster panadapter/waterfall response. This is
  a useful responsiveness result, not a reproduction or resolution of the
  earlier intermittent unattended and tune/start stalls.

Evidence is in ignored lab results `supervised-reconnect-start-2026-09-20T01-19-17.040Z.json`,
`supervised-reconnect-result-2026-09-20T01-21-02.148Z.json`, and
`supervised-tune-result-2026-09-20T01-23-31.284Z.json`.

The local follow-up adds measurements only; it is not deployed. New direct
XDMA performance metrics are process-lifetime maxima shared across TCI
clients and retained across disconnects and interval drains:

| Metric | Meaning |
| --- | --- |
| `iq_queue_wait_max_us` | IQ enqueue to a send attempt; requeue retries retain the original enqueue time. Frames discarded before a send attempt are not sampled. |
| `iq_pending_max_us` | Age since an IQ send attempt, observed while pending and on completion/failure/requeue; includes the initial send call, not just WouldBlock time. |
| `writer_loop_gap_max_us` | Time between client writer-loop starts, including ordinary work, sleep and descheduling; applies to control and media clients. |
| `socket_call_max_us` | Elapsed synchronous outbound send or flush call, including serialization for sends; all outbound message classes, not inbound read calls. |

These use monotonic wall time, not CPU time, TCP acknowledgement time or
browser playback latency. Maxima are not an atomic cross-field snapshot.
Observations at least 50 ms produce timestamped phase logs, limited to one
log per second across clients/phases. Suppressed logs do not suppress the
maxima. A long pending operation can be observed repeatedly; logs are not
an exact event counter. Existing `send_blocked_ms` remains unchanged and
must still not be interpreted as measured stall duration.

Existing control-batch logs retain their 5 ms threshold and add
`handling_us`, `model_lock_us`, `dsp_sync_us`, and `publish_us`. Handling
includes command-handler locks and side effects; model-lock timing covers
the subsequent shared-model acquisition; publication combines tuning, TX
and radio-state publication. These are elapsed time, not DSP CPU profiling.

Validation: all 276 bridge tests pass with `SATURN_BRIDGE_STUB_NATIVE=1`,
including exact-once buffered-send/retry/failure tests, queue-wait timing,
pending-age observation, and preservation of timing maxima across drains.
Native-stub tests do not qualify actual WDSP performance or live RF/audio.
No queue sizes, signal rates, scheduling policy, DSP algorithms, firmware,
send ownership, or drop classifications are changed. Next step is an
operator-approved diagnostic build deployment followed by the same controlled
reconnect/tune test and a fresh unattended baseline; retain the current
working binary for rollback. Do not introduce another optimization first.
