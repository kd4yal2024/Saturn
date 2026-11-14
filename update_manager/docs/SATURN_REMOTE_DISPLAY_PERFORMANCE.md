# Saturn Remote display performance

The first performance slice adds a bounded display FFT worker, a transfer pool,
and an actual-page replay profiler. WebGPU, audio buffering and Bridge scheduling
are unchanged. The ordinary display remains the default while target-browser
latency is evaluated. A passing numerical test is not a performance qualification.

## Select the display path

Append these parameters to the existing Remote URL, retaining its other options:

| Parameter | Behavior |
| --- | --- |
| no `display_fft` parameter, or `display_fft=main` | Existing main-thread FFT; daily default |
| `display_fft=worker` | Experimental bounded display worker; explicit fallback if unavailable |
| `display_transport=iq` | Existing raw-IQ display transport override |
| `display_transport=spectrum` | Existing server spectrum override, if the Bridge advertises it |
| `display_transport=auto` | Existing selection; server rows for WAN, IQ for LAN |

The transport override changes only display selection. Changing the **RX Transport**
LAN/WAN setting also changes audio/rate policy and is unsuitable for isolating this
A/B. Do not change those settings during a display-only comparison.

Location of FFT computation does not determine frequency resolution. At 384 kS/s,
4096 points give 93.75 Hz bin spacing on either host. Eight-bit rows quantize level.
The server and browser row parser now support up to 16384 points, matching the
browser terrain FFT ceiling. Older hosts still echo their lower effective size.
Consequently, compare the **reported effective FFT size** and bin spacing, not just
requested settings. Raw complex-float32 payload at 384 kS/s is 24.576 Mbit/s;
4096 eight-bit bins at 30 rows/s are 0.98304 Mbit/s before framing; 16384 bins
at 30 rows/s are 3.93216 Mbit/s, or 1.31072 Mbit/s at 10 rows/s.

## Scheduling and ownership

`DisplayFftProducer` has one in-flight job, one replaceable pending request, and
one latest completed result. New requests replace the pending request, rather
than extending a FIFO. The source window and browser-ingress timestamps are
snapshotted together. Rate limiting is anchored to ingress, so late dispatch does
not permanently move production behind live input. RAF draws the latest completed
result; it neither triggers an FFT nor applies a second cadence throttle.

Two slots circulate main → worker → main. Each slot has a 131072-byte input store
and a 65536-byte output store: four stores, 393216 bytes total. Both stores are
transferred back in every valid reply, including an FFT error reply. Sender views
are detached while the other owner holds them. Only producer-owned copies transfer.
The transport packet and canonical retained IQ remain attached on the main thread.
This removes the combined-window/output backing allocations for worker jobs; it
is **not zero-copy**. One retained ingress copy, one pool input copy, renderer and
history copies, view objects, and worker FFT scratch storage still exist.

Results are borrowed until the next completion/reset. Render averaging and the
numerical history retain their own data before a slot can be recycled. A tuning,
connection, source, pause/visibility, rate, FFT-target or measurement boundary
invalidates obsolete results. Tuning also drops old IQ windows while retaining the
existing recentered visual history. TX snapshots are never spliced together.
Timeout, construction, transfer or protocol failure terminates the worker and
reports an explicit main-thread fallback; audio and TX authority do not depend on
successful display analysis. The worker is inlined into the existing single IIFE;
there is no additional public worker asset URL.

## Capture and measurement boundary

RX Measure explicitly uses the original synchronous, full-precision raw-IQ FFT
path while capturing. Its accumulation interval, orientation, settings checks,
source lease and file contract remain intact. It does not consume coalesced worker
results or eight-bit server rows. Diagnostics label this `main-raw-measurement`.
Bridge-side RF feedback/IMD captures, WDSP, PTT, firmware, driver and RXC1 polling
are unchanged. No display reduction is a substitute for raw measurement data.

## Reproduce checks and the baseline

From `update_manager/remote-web`, with the existing dependencies installed:

```sh
npm run typecheck
npm run check:seam
npm test
npm run build
npm run validate:display-worker
npm run validate:settings
npm run profile:display -- --seconds=12 --engines=main,worker --quality=performance --output=/tmp/saturn-display-ab
```

The profiler serves the actual template and compiled bundle on localhost. An
independent fixture worker supplies 30 IQ frames/s (12800 complex pairs at
384 kS/s) and 50 stereo PCM packets/s (960 samples at 48 kHz) through the actual
page ingress. Its eight IQ and sixteen PCM transport buffers have an explicit
return path and drop counters. No radio is contacted. The AudioWorklet actually
runs; audio buffering is not reduced. Do not run build/test/browser jobs alongside
a timing run. Repeat in reversed engine order and on the operating Windows browser
before promoting a default.

Workloads are 2D, 3D, and 3D with tuning, resizing and the real Settings pop-out.
Terrain quality is fixed for comparability; `auto` is available for a separately
labelled adaptive experiment. `--workloads=2d,3d` selects a subset; `--seconds`
supports 6–60 seconds. The report includes template/bundle hashes, browser and
renderer identities, workload settings, events, heap delta, long tasks, heartbeat
delay, full-window CPU/age percentiles, source delivery delays, audio progress and
underruns. Fixture timing arrays span tuning resets; application counters retain
their documented bounded recent history.

Allocation rates count main-realm Float32Array backing stores only. They exclude
JS objects, native allocations, other typed arrays and worker allocations. Pool
allocation/copy bytes and cumulative worker scratch allocations are separately
reported. Heap delta is not allocation rate. FFT duration is measured on the worker;
round trip, input copy, queue age, latest-held age and draw age use the main clock.
Both newest and oldest included packet ingress ages are retained. These are
browser-ingress-to-draw-submission timings, not FPGA acquisition age or GPU
presentation latency. Renderer/meter call durations are CPU submission timings.

## Initial observations and rollout decision

The November 6 local Chrome replay used **SwiftShader software rendering**.
Repeated exploratory runs reduced the instrumented backing allocation rate by
roughly 20–35%, but worker handoff under the 3D workload increased display-age tails;
results varied with software-renderer load. Some runs also recorded audio underruns.
The earlier adaptive-quality runs are exploratory, not a controlled comparison of
fixed geometry. Keep their reports, including regressions. Do not generalize them
to the operating Windows GPU or claim an across-the-board latency improvement.

A separate read-only, approximately ten-second G2 receive observation on release
`a1b9f39b` showed about 51% of one CPU core, stable RSS around 106 MiB and continuing
384 kS/s acquisition. It measured neither DMA/queue tails nor G2 rendering and does
not establish that Bridge scheduling is or is not a bottleneck. No service or
radio setting was changed.

The next promotion criterion is lower or acceptable p95/p99 display age and main
thread stalls under equal workload, with bounded queues, continued audio and no
measurement changes on the operating browser. Profile rendering/layout and the
existing acquisition owner alongside that comparison. Change scheduling only if
its own measurements justify it. Evaluate WebGPU after this CPU/ownership baseline;
retain FreeDV and audio-buffer tuning as separate work.

## Server cadence after missed deadlines

The server spectrum worker advances a missed deadline to the next future
cadence boundary. It skips obsolete periods rather than emitting a catch-up
FFT followed immediately by another FFT. A deterministic regression covers
on-time work, a boundary and multiple missed intervals. Display transport,
FFT resolution and canonical capture samples are unchanged.

## First allocation slice

Display orientation and smoothing accept optional caller-owned output arrays.
The ordinary helpers still return independent arrays unless an output is
explicitly supplied. The template borrows orientation scratch only for the
synchronous draw: numerical history and temporal averaging copy before the next
use. RX Measure continues to use the owned, full-precision helper. Spectrum
vertices and waterfall smoothing/history have separate renderer-owned buffers;
size changes replace those buffers. Inputs and overlapping views cannot be used
as smoothing scratch. Geometry, FFT sizes, windowing and display defaults stay
the same.

The RX Opus worker reuses its PCM, recipient-list and encoder output scratch.
It creates one immutable encoded payload per frame, shared by recipient queues.
Client wire framing and sequence numbers remain independent; queue limits,
pre-join exclusion and TX-priority/epoch checks retain their meaning. This is
not an end-to-end zero-copy path. Codec settings, 20 ms packets and browser audio
queue targets are unchanged. The optional SAB producer sends `stream-start`
once after the first data publication for each worklet; reconnecting with a new
node sends a fresh notification. The default postMessage path is unchanged.

For a current host CPU baseline, while the desired workload is already running:

```sh
python3 update_manager/scripts/saturn-thread-profile.py \
  --pid "$(systemctl show saturn-bridge -p MainPID --value)" \
  --seconds 12 --output /tmp/saturn-thread-baseline.json
```

The tool reads Linux task accounting and the public performance record. It does
not send radio commands, change scheduling or attach a sampler. TID start times
prevent reuse from mixing counters; process exit/restart invalidates the run.
CPU percentages use one core as 100%. Short-lived threads between observations
can be missed. Public histogram percentiles remain cumulative process-lifetime
values; this tool does not turn them into window-specific latency percentiles
or collect call stacks. It records observed workload transitions so mixed runs
can be identified. Label standby separately from active audio/IQ, heavy DSP,
3D and PS workloads; a quiet channel worker does not establish its loaded cost.

Browser before/after runs must use equal FFT/zoom, quality, smoothing, workload
and renderer, without concurrent test/build jobs. Report the allocation scope,
audio progress/underruns, source delivery and latency tails, including regressions.
Software-renderer results do not qualify the operating Windows GPU or justify
promoting worker/transport defaults.

## Matched display-path comparison

Server rows remain opt-in on LAN. Automatic WAN retains its existing 4096-bin
cap; the explicit `display_transport=spectrum` override enables the higher cap.
The larger size is negotiated through the
existing command/echo, with the same 64-byte header, Hann window and 0.625 dB
quantization. Old clients continue to request their own smaller limit. New
clients accept an older host's smaller echo and report the actual row FFT size;
requesting 16384 does not prove that size was received. The latest-row slot and
two-row acknowledgement credit remain bounded at the larger payload size.

The profiler can now run all three paths against the real Rust TCI frontend,
dispatch, outbound queues, frame builder, latest-IQ ring and spectrum worker.
Its explicitly ignored test fixture binds only an ephemeral loopback port,
publishes deterministic continuous complex tones and stereo PCM, and stops
within 120 seconds. It constructs no hardware/backend owner or WDSP channel.
The profiler closes its socket and requires zero remaining connections. Use
a **fresh** output directory for each run.

Build the test executable with native stubs, select the `Executable unittests
src/main.rs` path printed by Cargo, then run from `remote-web`:

```sh
# In saturn-bridge; this executable is a synthetic fixture, not a radio release.
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --release --bin saturn-bridge --no-run

# In remote-web, substitute that exact test executable path.
npm run build
npm run profile:display -- --seconds=12 --engines=main,worker,spectrum \
  --workloads=2d,3d,interactive --quality=performance --zoom=2 \
  --bridge-test-binary=/absolute/path/to/release/deps/saturn_bridge-HASH \
  --output=/tmp/saturn-display-paths-forward
```

Repeat in reversed engine order (`spectrum,worker,main`) in another fresh
directory, with no concurrent builds or browser tests. Optional
`--fft-size=16384` fixes the FFT for every selected workload; without it the
actual page computes its target from rate, zoom and texture limits. A zoom of
2 permits 16384-point terrain at 384 kS/s with an 8192-wide history texture.
Keep the same zoom, mesh quality and viewport in each arm.

For this loopback comparison only, the injected harness uses a common fixed
interval of 33 ms for Traditional and 100 ms for terrain on **every** path.
Without that intervention, raw/worker terrain at about 30 rows/s and server
terrain at 10 rows/s are unequal workloads. These fixture hooks do not change
the shipped page's cadence. The report checks effective FFT, target, interval,
zoom, mesh/backing-store geometry and audio rate/channels across the paths and
fails a mismatched comparison, retaining the report. A one-shot bin probe checks
both signs of the synthetic complex spectrum; it is not an RF measurement.

New report fields include measured binary bytes by IQ/row/audio stream, process
CPU as a percentage of one core, row replacements/credit, raw-IQ queue bounds,
and FFT histogram counts/tails. CPU is sampled from process task accounting
over the timed workload. The fixture's final transport/FFT counters cover its
whole lifetime, **including warmup**; histogram p95/p99 are bucket upper bounds.
The executable hash records which optimized or debug test build actually ran.
This measures Linux loopback publication/analysis, excluding hardware acquisition,
WDSP, TLS and the Go proxy. It cannot establish G2 CPU savings. Browser age begins
at ingress on each chosen path, so it omits the server FFT/network time that
preceded a row's arrival; it is not an end-to-end comparison of acquisition age.

RX Measure still leases original full-precision IQ and uses its synchronous FFT.
The new cap and profiler do not change capture samples, audio buffering, FFT
normalization, scheduling, TX authority, PureSignal, firmware or driver behavior.
Promotion still requires the operating browser and host at matched quality;
neither a lower wire rate nor software-renderer replay alone selects a default.

The interval match is a **requested target**, not a guarantee of identical
achieved draw rates. Inspect the actual full-window draw counts and durations:
the worker's ingress-anchored cadence can submit more frequently after delayed
dispatch. The November 13 loopback runs observed up to about 11% more terrain
draws on the worker. Do not attribute every per-second allocation or CPU
difference to FFT location. Keep measured rates, regressions and underruns in
any comparison; no default is promoted by this slice.
