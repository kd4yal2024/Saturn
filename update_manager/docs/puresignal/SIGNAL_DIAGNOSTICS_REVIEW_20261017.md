# Two-tone / PureSignal diagnostic additions — first offline slice

2026-10-17. No G2 access, installation, keying, fitting, firmware, driver or
polling change in this assignment. WDSP and the existing TX owner are retained.
This is an offline analyzer, bounded capture API and imported-evidence Monitor,
**not completed live capture wiring or proof of RF output**.

## Crate evaluation and existing capabilities

The six proposed crates are not Bridge dependencies today. Cargo.toml and
Cargo.lock are unchanged by this slice. Documentation below was checked on
2026-10-17; latest docs are not a dependency pin.

| Tool | Existing implementation / gap | Decision for this slice |
| --- | --- | --- |
| [`spectrum-analyzer` 2.0.0](https://docs.rs/spectrum-analyzer/latest/spectrum_analyzer/) | Its real-sample API is suitable for PCM; Hann window and scaling must still be explicit. Saturn already has a complex FFT. | Do not add a second FFT dependency. Analyze real mono PCM by explicitly setting Q=0 in the existing FFT, never by flattening I/Q. |
| [`rustfft` 6.4.1](https://docs.rs/rustfft/latest/rustfft/) | Useful complex FFT planner, including AArch64 NEON; output normalization belongs to the caller. Existing display has a tested radix-2 complex FFT. | Extract and reuse that exact kernel without changing display arithmetic. Consider RustFFT only if non-power-of-two windows or measured performance justify replacing it. |
| [`tracing` 0.1.44](https://docs.rs/tracing/latest/tracing/) | Existing stderr logs and owner events lack one automatically correlated signal/control timeline. Subscriber work can run synchronously; the crate alone does not move formatting away from TX. | Add typed offline event records now. Defer dependency/subscriber wiring until a bounded off-thread collector is connected to the actual owner. No live timeline is claimed here. |
| [`hdrhistogram` 7.6.0](https://docs.rs/hdrhistogram/latest/hdrhistogram/) | Runtime telemetry already has fixed power-of-two latency histograms; the isolated DUC probe has finer bounded write/refill histograms, maxima and overflow accounting. Release timing still needs measurement. | Reuse those first. They are upper-bound percentiles, not precise HDR quantiles. If finer tails are needed, use fixed bounds, disabled auto-resize and explicit overflow counts. |
| [`ringbuf` 0.5.3](https://docs.rs/ringbuf/latest/ringbuf/) | Existing bounded queues and RX atomic ring. Diagnostic producers must never wait for analysis. | Reuse standard synchronization with producer `try_lock`, preallocated slots and explicit drops. This is **not lock-free/wait-free** and has no measured G2 execution-time bound. Consider SPSC ringbuf if measured contention warrants it. |
| [`hound` 3.5.1](https://docs.rs/hound/latest/hound/) | Listening fixtures need WAV, but exact IQ must remain separate. | No dependency needed for a small PCM16 listening-copy exporter. Keep original f64 bits and scaling; reject IQ, nonfinite input and clipped listening exports. Hound remains reasonable for richer WAV needs. |

Source inventory:

- [Bridge dependencies](../../saturn-bridge/Cargo.toml).
- [Existing FFT, now shared unchanged](../../saturn-bridge/src/diagnostic_fft.rs),
  reused by [RX display](../../saturn-bridge/src/display_spectrum.rs).
- [Runtime latency histogram](../../saturn-bridge/src/xdma_telemetry.rs) and
  [isolated DUC write/refill timing](../../saturn-bridge/src/xdma_duc.rs).
- [Existing paired recorder, overload coverage and delay estimator](../../saturn-bridge/src/puresignal/measurement.rs).
- [Local headphone monitor](../../saturn-bridge/src/tx_monitor.rs) is not the PS
  measurement Monitor. It demodulates/resamples for listening and intentionally
  excludes two-tone; it must not become a substitute for raw signal evidence.

## Implemented offline path

| Capture point | Samples / evidence | Implementation |
| --- | --- | --- |
| Pre-WDSP PCM | Real mono, both 700/1900 Hz components, RMS/peak/DC, at-full-scale count, exact bits | [Capture API/analyzer](../../saturn-bridge/src/signal_diagnostics.rs) |
| WDSP output | Logical complex I,Q; signed frequencies and opposite-sideband estimates | [Real-native replay](../../saturn-bridge/src/xdma_tx_radio/tx_spectrum_review.rs) |
| Immediately before backend output | Same native IQ and actual packed bytes | Same replay; original payload saved separately |
| Completed output | Actual P2 loopback receive after socket send; actual DirectTxState file-backed write and file readback | Same replay; **socket/file completion, not physical RF or G2 DMA timing** |
| Returned reference/RF | Existing paired capture JSON, exact IEEE754 bits, original fence/arrivals/scaling and unknown overload retained | [Feedback conversion](../../saturn-bridge/scripts/feedback-diagnostics.py), then same analyzer |
| Monitor | Imported point-labeled levels/status, unavailable reasons and read-only pop-out | [UI module](../../remote-web/src/puresignal/signal-monitor.ts), actual PS tab template |

The native replay contains positive USB/LSB controls and a negative rejected
completion record. That negative record tests interpretation; it is **not an
injected kernel/DMA failure test**. Existing owner fault tests remain separate.

No audio is mixed into the TX path by these diagnostics. A Native PostGen
two-tone can legitimately have silent **pre-WDSP PCM** because the generator
is later in the WDSP chain; interpretation must include source selection.

### Capture and analysis contract

`CaptureTap` has 1–32 preallocated slots, at most 16,384 f64 components per slot,
and a finite 1–8,192 record budget. Producers copy only within that bound, never
allocate/format/analyze/export, and drop on contention or exhausted slots.
Queue drops and budget exhaustion are separate counters. Worker analysis
releases the queue lock first. There is no automatic retry/backpressure into TX.

`SATDIAG1` stores exact f64 sample bits, point/backend/event, 128-bit session and
configuration IDs, generation, TX epoch, original observation bounds, recorded
time, timestamp domain, rate, units/full-scale, commanded attenuation, continuity,
overload, keyed snapshot and requested/completed byte counts. Unknown is not
clear, zero, successful completion or fresh. Source/native identity, full
overload brackets and original admission reasons remain in the **original
capture JSON sidecar**, retained byte-for-byte; the compact record does not
replace that evidence. These are not authenticated capture files.

The feedback converter deliberately leaves compact overload/continuity evidence
unknown rather than trusting a JSON producer's clear flag. Original evidence is
preserved for the existing checker. It never divides normalized feedback by
8,388,608 twice, merges generations or assigns new arrival times. Empty captures
and summary-only inputs are rejected. Missing per-pair arrival coverage remains
unknown. Nonfinite sample bits remain available even when analysis is rejected.

Analysis uses a symmetric Hann window and the shared f32 display FFT, with raw
f64 samples untouched. Spectrum amplitudes are `abs(FFT)/sum(window)`, fftshifted,
two-sided dBFS (not PSD, dBm, watts or RF dBc). Exact-frequency Hann DTFT estimates
use the same coherent-gain divisor; real PCM alone uses the one-sided x2 factor.
Short unresolved tone estimates are flagged and hidden from the Monitor's tone
level readouts. Other levels remain available. FFT length rounds **down** to a
power of two; `rawFrames` versus `fftFrames` records the analyzed prefix. Never
concatenate discontinuous blocks to pretend to obtain a longer coherent window.

At/full-scale component counts are digital observations, **not proof of ADC or
FPGA clipping**. Unknown overload coverage remains unknown. The analyzer does
not calculate or qualify RF IMD. The existing delay estimator's convention is
positive lag = RF arrives later than reference; ambiguity/periodic signals and
uncertainty remain explicit and are not converted into WDSP delay settings.

The Monitor opens imported evidence only, not a live stream, and always labels
observation/delivery age unverified. It neither negotiates nor changes TX/PS
settings. The pop-out is a read-only snapshot, not a live synchronized panel.
This is a first level/status view, not the completed dockable spectrum UI.

## What the four requested questions can establish now

1. **Both tones before/after WDSP?** Yes in the new real-WDSP offline replay,
   with intended USB/LSB orientation. This is not a live G2 observation.
2. **Nonzero correctly scaled IQ at completed H2C writes?** Verified through
   actual packing and completed **ordinary-file** output/readback; P2 socket
   output is checked separately. No deployed H2C capture has been taken.
3. **Keyed state persists throughout the interval?** **Unknown.** Host snapshots
   and synthetic events cannot establish continuous keyed state or physical RF.
4. **Returned reference/RF content?** Existing synthetic paired native replay
   captures are accepted bit-exactly and analyzed separately. Real G2 feedback
   windows must still be collected with original acquisition provenance.

## Reproduction (offline only)

From `update_manager/saturn-bridge`:

```sh
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --offline --lib
SATURN_BRIDGE_STUB_NATIVE=1 cargo build --offline --example tx-diagnostics
python3 -m unittest discover -s tests -p test_signal_diagnostics.py -v

# A fresh output directory is required. Input is the existing RAW capture, not
# saturn-two-tone-summary-v1. Keep original-capture.json and sha256.json.
python3 scripts/feedback-diagnostics.py RAW_CAPTURE.json NEW_OUTPUT_DIRECTORY
target/debug/examples/tx-diagnostics 700 1900 \
  NEW_OUTPUT_DIRECTORY/capture-0000-referenceF64Bits.satdiag \
  NEW_OUTPUT_DIRECTORY/capture-0000-rfF64Bits.satdiag > diagnostics.json

# Listening copy for pre-WDSP PCM ONLY. Never pass complex IQ here.
python3 scripts/diagnostic-pcm-wav.py PRE_WDSP.satdiag NEW_LISTENING_COPY.wav
```

Real-native test (requires the already reviewed x86 WDSP archive/header/observer,
never the stub and never an ARM binary):

```sh
SATURN_WDSP_DIR=REVIEWED_NATIVE_DIR \
RUSTFLAGS='-C link-arg=REVIEWED_OFFLINE_OBSERVER.o' \
SATURN_SIGNAL_DIAG_EVIDENCE_DIR=NEW_ABSOLUTE_DIRECTORY \
cargo test --offline --features managed-puresignal --bin saturn-bridge \
  tx_spectrum_review -- --ignored --nocapture --test-threads=1
```

From `update_manager/remote-web`:

```sh
npm test
npm run typecheck
npm run build
node scripts/validate-signal-monitor.mjs OFFLINE_EVIDENCE_DIRECTORY/diagnostics.json
```

The Chrome harness uses the actual template and built bundle, with HTTP and
WebSocket access blocked. It checks imported levels, pop-out, empty-evidence
rejection and zero socket attempts. It does not connect to a G2 or key anything.

## Remaining bounded production work — not replaced by these tools

The current live two-tone blocker is the explicit rejection in
[Direct-XDMA command dispatch](../../saturn-bridge/src/xdma_backend.rs), before
the corrected generator. Fix/test that actual command path separately; adding
an analyzer cannot make a rejected request emit RF.

The capture API currently runs in offline fixtures. Production wiring remains:

- Attach opt-in taps to the existing owner at actual pre-WDSP/output boundaries;
  preserve its session/generation/config/TX epoch and sample-age provenance.
- Capture exact H2C bytes before submission and record completion **after** the
  real write returns, including short/error writes. Capture key/readback/control
  changes, selected source, requested drive versus actual code, stop cause and
  cleanup in the same generation timeline. P2 completion remains weaker evidence.
- Attach paired taps only to the existing acquisition owner, with its original
  samples/fence/arrival bounds. Add no C2H reader or synthetic clear bracket.
- Keep analysis/export off TX, count drops/export errors, bound retention/disk
  use and protect real microphone recordings. Do not enable raw capture by default.
- Connect low-rate results to the PS Monitor with the previously reviewed
  freshness/delivery contract; do not portray this imported view as live.

Those changes need actual owner-loop tests including failures, release and
disconnect before a candidate is installed. No new RF qualification, G2 test,
dependency, FPGA, driver or polling change is implied by this offline slice.
