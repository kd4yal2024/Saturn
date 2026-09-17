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
