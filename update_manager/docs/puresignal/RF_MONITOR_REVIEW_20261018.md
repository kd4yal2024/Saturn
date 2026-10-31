# RF-feedback visual monitor / relative IMD — review candidate

## Disposition

Implemented and offline-tested; **not deployed or RF-qualified**. Preserve the
installed `feb91629911b…` live-display trial that Jerry has observed producing
both tones, 5.5 W and 1.15:1 SWR on his LP700. That operator report is not a
measurement produced by this new analyzer. No G2 operation, PTT, adaptation,
firmware, driver, RXC1 polling or Git push belongs to this implementation slice.

## What is visible

The PS tab retains the existing two-tone button and adds a separate flat
RF-feedback spectrum, frequency/dBc grid, F1/F2 and IMD3/5/7 markers, both-side
numeric readout, read-only live pop-out, and held/exportable PS-off-requested
baseline overlay. The existing main panadapter remains **digital WDSP TX**.
These sources are not relabeled as one another. The denser 3D mountain/ridge
renderer is not implemented here; the flat measurement view takes priority.

Baseline overlays require matching backend, host source pin, commanded center,
power, attenuation, tone offsets, FFT and window. This is not proof of matched
external power, coupler response, load, temperature or native correction state.
The export is a spectrum snapshot, not a raw IQ capture. LP700 readings remain
independent operator observations, not numbers invented by the panel.

## Source paths and boundaries

```text
Existing sole acquisition owner
  P2 managed: DDC0 paired datagram -> existing P2Adapter
  Direct shared: sole C2H reader -> existing DirectXdmaAdapter
  Direct Legacy opt-in observer: SAME sole reader and layout transaction
      |
      +-> normal shared owner delivery (unchanged, where selected)
      +-> preallocated try-lock observer copy; explicit diagnostic drops
             -> noncritical FFT/archive worker (maximum 10 Hz)
             -> bounded latest-only TCI display-class TEXT on control lane
             -> PS monitor / read-only pop-out / held spectrum overlay
```

Sources: `src/tx_thread/feedback_stream.rs`,
`src/xdma_rx/managed_feedback.rs`, `src/rf_monitor.rs`,
`src/rf_monitor/legacy.rs`, `src/shared_backend.rs`, `src/xdma_backend.rs`,
`src/tci/outbound.rs`; browser `src/puresignal/rf-monitor.ts`.

`SATURN_PS_LIVE_MONITOR=1` is an explicit host opt-in. Without it, there is no
observer worker or Legacy feedback-layout change, and the panel reports the
host observer unavailable. Legacy P2 is not newly wired; P2 shared acquisition
can feed the passive observer through its existing paired path.

Direct Legacy observes only an **already-keyed existing two-tone operation**.
Its new stream has no SharedIngress, native DSP, output permit or TX authority.
It cannot key, authorize calibration, fit or correct. Its layout/generation tags
cannot become output permissions. It reuses the pinned PCB2 1.31.002 /
0x53460004 layout transaction, acknowledgment, pause/drain, ADC1 RX BYPASS
selection and parser restoration; it creates no second C2H reader.

The route keeps its temporary layout through actual keying, including a
configuration fault, and restores after the TX owner's snapshot reports
unkeyed. Receive filter and attenuation edits are preserved for restoration.
On process shutdown, the TX worker is stopped/joined first, then acquisition
restoration is serviced with a finite deadline before normal RX shutdown.
Failed restoration is an error, not a successful cleanup receipt. These are
host-control guarantees, not measured physical RF-off timing.

## Measurement contract

* Original normalized signed-24 RF and TX-reference pairs, 192 kcomplex/s;
  never audio, IQ flattened into audio, AGC output, displayed averages or peak
  holds. Source/backend/generation/config/TX epoch are retained. Host source
  pin is supplied; firmware identity is explicitly not supplied by this
  observer and must accompany the bench record separately.
* One contiguous 16,384-pair window: 85.33 ms nominal sample duration,
  11.71875 Hz bin spacing. Four-term Blackman-Harris, coherent gain and ENBW
  explicitly applied; effective noise bandwidth approximately 23.49 Hz.
  Bin spacing is not advertised as resolution bandwidth.
* Fundamentals are searched at either signed orientation of the commanded
  two-tone offsets. The RF and reference orientations must agree; missing or
  unsuitable reference fundamentals suppress IMD numbers. Signed IQ offset
  is not yet qualified as absolute RF sideband orientation on the bench.
* Products are the conventional 3rd/5th/7th-order pairs. Power is integrated
  over eleven bins per expected line, corrected for window ENBW, with a local
  median-based noise estimate. The weaker fundamental is the stated dBc
  reference, not PEP. Absolute watts/dBm are not calculated.
* Below-floor products are estimated limits, not zero distortion or precise
  readings. Missing fundamentals, clipping and ambiguous orientation suppress
  product measurements. Relative estimates do not qualify an observation
  receiver, prove analyzer dynamic range or demonstrate RF IMD improvement.
* Overload coverage requires correctly sourced, valid interval brackets.
  Existing undecorated streams supply none, so their coverage stays **unknown**.
  Sample near/full-scale detection is additional evidence, not proof that
  analog stages upstream were unclipped. `rfImdQualified` remains false.

The FFT is the existing radix-2 engine with a caller-selected analysis window.
Its existing Hann display constructor remains unchanged. No new dependency,
signal-processing engine or watchdog gate is introduced.

Product frequencies and per-tone conventions were checked against primary
[Keysight IMD documentation](https://helpfiles.keysight.com/csg/pxivna/Applications/Swept_IMD_and_IM_Spectrum_Concepts.htm).
Our deliberately conservative **weaker-tone** reference is explicitly stated
rather than confused with the documentation's respective-side/average options.

## Freshness and output pressure

Original Bridge ingress observations are converted to a process-monotonic
clock without assigning arrival time at dequeue. A nonce-based ping exchange
bounds client/host clock offset using the whole round trip; delayed telemetry
cannot acquire freshness merely by arriving. A new keying interval requires
a fresh clock barrier and a capture observed after it. Disconnect, consumer
stall, old-session data and expired captures cannot certify a current display.
This bounds Bridge-ingress age, **not exact FPGA acquisition/RF-output age**.

Producer copies use preallocated buffers and try_lock. Gaps, generation and
configuration changes clear partial windows. The worker skips old captures;
the existing depth-one display slot replaces slow output. RF spectra remain
below 24 kB and have lower priority than release/safety and normal control.
They are text on the control lane, never binary on it or text on split media.
The writer drops telemetry queued more than 250 ms; the browser applies a
750 ms conservative observation-age deadline and marks held/stale data.

## Raw evidence

`SATURN_RF_MONITOR_CAPTURE_DIRECTORY=/new/absolute/directory` optionally enables
a new private directory, owned by the Bridge process. It refuses an existing
directory. The FFT worker writes at most 32 windows per process, at most two
per generation separated by at least one second. No disk formatting or FFT
occurs in the TX or acquisition hot path. Exhaustion/errors never authorize RF.

Each `.pairs` record is 24 bytes:
`refI:f32le, refQ:f32le, rfI:f32le, rfQ:f32le, originalBridgeArrivalNs:u64le`.
Signed-24 / 8388608 values are exactly representable in these floats. The
associated JSON states layout/scaling, source/native hashes, raw SHA-256,
original times, relative spectrum and unavailable qualifications. Window and
sample counts must be checked before treating an archive as measurement
evidence; a valid filename or empty directory is not a successful capture.

## Offline evidence and remaining bench work

Tests exercise known two-tone products through -65 dBc in synthetic data on
both orientations, clipping/silence/short captures, the existing P2 pair decoder,
the actual Direct operational layout/demux/restoration against ordinary files,
configuration edits and re-arm, queue contention, finite raw archives and the
actual WebSocket writer/lane/priority behavior. Native owner regression tests
still run against pinned real WDSP. The browser check runs the actual template
and IIFE in Chrome at desktop/mobile widths, with **synthetic Rust spectra and
rebased mock timing**, all external networking blocked. Screenshots are UI
evidence, not RF observations.

Next deployment must preserve the currently installed working trial and its
configuration, use a matched binary/UI and new runtime overrides, and leave
firmware/driver/polling and PS fitting/correction unchanged. First verify RX,
single acquisition ownership and orderly rollback with no PTT. Then Jerry may
run the existing operator-controlled two-tone at the approved bench settings
and compare LP700 power/SWR against his working baseline. Check that actual
reference/RF tones appear, attenuation is suitable, release restores RX, and
captured raw pairs agree with the view. Do not increase power just because an
IMD product is below the floor. RF-path linearity, frequency orientation,
measurement uncertainty, load/coupler effects and G2 timing remain unqualified.

## Managed single-fit generated PCM follow-up — 2026-10-18

This later **offline** slice adds the missing generated source to the existing
shared TX owner. It does not replace the running `2ac502cc20ef…` timing trial,
enable its fitter, or convert its MeasurementOnly permission into correction
permission. No G2 installation, RF action, firmware, driver or RXC1 change is
part of this slice.

The host explicitly opts in with `SATURN_PS_GENERATED_TRIALS=1` and an explicit
`SATURN_PS_GENERATED_FREQUENCY_HZ`. MeasurementOnly rejects that combination.
The default is off. All existing topology/native identity, RF-permission,
feedback admission and lifecycle limits remain applicable; the new option
supplies none of them. The announced duration is the lesser of 45,000 ms and
the existing host TX-duration ceiling. The action accepts only TCI tagged PCM,
USB/LSB, 1–5 commanded W, the fixed frequency, enabled non-automatic single-fit
configuration, a sealed operator credential and a fresh ingress timestamp.

Protocol:

```text
saturn_ps_two_tone_caps:0,tagged-two-tone-v1,<maximum-ms>,<frequency-hz>,5;
saturn_ps_two_tone:0,<client-request-id>,start,<duration-ms>;
saturn_ps_two_tone:0,<client-request-id>,queued;
saturn_ps_two_tone:0,<client-request-id>,accepted;
saturn_ps_two_tone:0,<client-request-id>,rejected,<reason>;
saturn_ps_two_tone:0,<client-request-id>,complete,<fault-or-none>;
```

`Calibrate (prepare single)` remains a settings transaction and cannot key.
The PS-tab two-tone action is separate, consumes fresh attended confirmation
and TX Ready, and uses a new fixed 700/1900 Hz PCM source, not the microphone
or Legacy native generator. Native microphone gain is temporarily set to 0 dB
with readback, without disabling single-fit mode. Cleanup recreates the native
engine and restores the ordinary profile. The owner, independently of browser
timers or incoming packets, expires the trial from its original request time.
Release/disconnect/settings retain the existing cancellation and output fence.
The ticket waits for a newer completed backend/native cleanup before reporting
completion. Both terminal results and the newer clean owner publication are
required by the browser before another action. Old replies/callbacks cannot
authorize another generation. A locally cancelled, never-admitted request can
retain its rejection reason without inventing an authorization fault; genuine
authority/native/stale faults remain faults.

Run the reproducible offline check with explicit reviewed x86 `SATURN_WDSP_DIR`,
observer `RUSTFLAGS`, and a new absolute `PS_SINGLE_EVIDENCE_DIR`:
`bash tests/run-puresignal-single-two-tone.sh`. It pins current source, runs the
existing full timing/browser baseline, then the generated-action tests serially.
The tests cover the actual WebSocket dispatch, scheduled owner and both existing
output adapters, cancellation/stale requests, finite expiry, configuration stop,
missing-feedback watchdog, and matched-WDSP unity/manual-fit control readback
and profile restoration. Output tests use loopback/files, not hardware; their
reference settings are fixtures, not a production feedback profile. They do
not establish a successful fit or RF improvement. The existing actual-runtime
Chrome timing baseline is rerun; a Chrome single-fit hardware path is not claimed.

### Reference scaling review and the next concrete bench observation

The preserved steady timing windows 53 and 97 have returned-reference envelope
peaks 0.66801836 and 0.66786777, and RF peaks 0.35252645 and 0.35231844.
These are normalized signed-24 / 8388608 samples, not RF watts. WDSP's pinned
`SetPSHWPeak` sets the inverse scale used on the **returned reference**, not
on the RF envelope. Neither those two reference maxima nor the older 0.6121
constant independently verifies the present path's full-scale reference gain.
The existing shared signal log has component peaks, but that is not an exact
retained, paired WDSP-output/completed-H2C scaling record. The optional Legacy
digital-capture worker is not presently the shared owner's WDSP/H2C capture.

Therefore no live HWPeak, adaptation threshold or fitting profile is generated.
The repeatable integer delay candidate is zero samples; the roughly 71/73-sample
3-dB correlation widths and unavailable total uncertainty do not prove exact
zero delay. Hardware overload coverage remains unknown, separately from absence
of digital full-scale samples.

Next, retain a short matched digital WDSP/H2C window through the existing writer
and returned reference/RF window at unchanged 7.200 MHz / 5 commanded W, with
fitting disabled. Compare complex envelope scaling, exact packing, actual input
level and continuity; preserve original times and stop/cleanup. This is a targeted
scaling observation, not another identical timing repeat or a power increase.
Then review the explicit HWPeak/admission/delay trial settings before installing
the separately opted-in single-fit profile. Jerry's LP700 report of about 5 W,
1.5:1 during timing is external operator evidence, not an internal calibration.
