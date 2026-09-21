# Targeted 3D rendering correction

This pass starts from `9366845`. It changes the 3D renderer, diagnostics and
operating overlays; the live transport, FFT, AGC, receiver gain, audio and TX
paths are unchanged. The supplied reference/current-capture files are not present
in this conversation or workspace. Comparisons below use identical deterministic
measurements rendered by the actual before/after application, not invented RF
activity presented as a hardware capture.

## Diagnosis before appearance changes

The verified data path is:

1. `handleIqFrame` reads the existing Float32 IQ payload and retains the existing
   RX/TX source handling. The display scheduler transforms each accepted IQ
   version with `FftProcessor`.
2. FFT levels are `20 log10(hypot(re, im) / N + 1e-8)` after a Hann window. Units
   are **relative dB**, not calibrated dBm. No new offset or window correction is
   introduced. `visibleBinsForDisplay` crops and reverses frequency exactly once.
3. `acceptSpectrumFrame` copies those signed values into `SpectrumHistory`
   before Traditional averaging/peak hold. Rows contain sampled per-bin maxima
   within the explicit cadence bucket. There is no RGB reconstruction or second
   dB normalization. Missing buckets are marked separately.
4. The numerical ring uploads unchanged Float32 levels to an R32F texture.
   Shaders peak-reduce bins only when needed for visual resolution, then use
   `n = clamp((db - floor) / (ceiling - floor), 0, 1)`.
5. Both views within 3D use the same `palette(pow(n, gamma))`. Depth does not tint
   or light the surface. Height is independently `n × heightControl × scale`.
   There is no added signal-height offset or nonzero minimum extrusion.

The diagnostics now report source units, min/median/p95/max, normalized median,
floor/ceiling clipping percentages, range, color gamma, height multiplier and
zero offset. Source and stored-bucket statistics are separate so peak aggregation
is visible. These are computed from CPU arrays only, on the existing one-second
diagnostic refresh while enabled. No runtime GPU readback was added.

For example, with a fixed −140/−40 range, a background at −90 dB has normalized
height 0.5 and palette coordinate approximately 0.555 with gamma 0.85. That
background legitimately appears cyan/green under those selected bounds. This is
not evidence of a dB decoding error. It requires checking the actual input/range,
not simply darkening a palette. The operator's live distribution is still unknown.
Use **Display Settings → Level / rendering diagnostics** before **Fit range to
current signals**. That fit remains a one-shot, explicitly requested adjustment;
there is no per-frame auto-normalization. Real broadband increases stay visible.

## Verified defects, appearance adjustments and unresolved observations

| Finding | Evidence and correction |
| --- | --- |
| Opaque front fence | Verified extra `TRIANGLE_STRIP` skirt closing every front bin to baseline. Removed; replaced by one `LINE_STRIP` at the measured leading edge. No intermediate skirts exist. |
| Float-sampler precision unspecified | Both scalar samplers inherited a low-precision default despite high-precision float arithmetic. Explicit `highp sampler2D levels` now protects signed Float32 samples. A defect in the declarations is verified; corruption on the operator's device is not reproduced. |
| Excessive geometric relief | Appearance adjustment: reduced clip-space multiplier from 1.25 to 0.75, preserving the saved height control and linear dB normalization. Zero bias remains zero. Range, color and measurements are unchanged. |
| Artificial terraces | Not reproduced with constant input: connected strips have uniform color at multiple ages and a planar constant-height surface. Strip restarts separate rows correctly. No temporal smoothing or signal suppression was added. |
| Horizontal waterfall dividers | Not reproduced in the constant-input renderer, before or after, across three full wraps. All waterfall pixels are uniform. The synthetic broadband pulse survives exactly once. Periodic lines in the original mixed validator are deliberately injected pulses/missing intervals; the operator's lines cannot be identified without their capture/levels. Genuine gaps remain marked. |
| Duplicate labels and truncation | Lower band badges removed in 3D only; boundary lines remain. Passband label reduced, divider subdued, status shortened to retained seconds, chronology/boundary details moved to a tooltip. |
| Grid | Explicit 3D-only opacity control, default zero. Traditional grid preferences stay intact. Controlled raw-canvas captures exclude every DOM overlay. |

R32F remains NEAREST sampled with `texelFetch`, not float-linear-filtered and not
used as a render attachment. The RGBA8 palette alone uses linear filtering.
This avoids relying on unrelated float filtering/renderbuffer capabilities.
The explicit sampler precision follows [MDN's float-texture precision guidance](https://developer.mozilla.org/en-US/docs/Web/API/WebGL_API/WebGL_best_practices#be_precise_with_glsl_precision_annotations).
GPU limit/error queries remain setup/allocation checks; no synchronous query or
readback was added to steady drawing. Changed rows still upload incrementally.

## Controlled comparison evidence

[Before/after gallery](../../FPGA/lab/results/terrain-correction/comparison.html)
uses actual renderer screenshots. Raw PNGs omit every overlay; full-page captures
show the application with display overlays disabled. The pulse also has an
overlays-enabled capture to distinguish DOM presentation from numerical rows.

| Fixture | Before | After | Verified result |
| --- | --- | --- | --- |
| Constant −110 dB | [PNG](../../FPGA/lab/results/terrain-correction/before/constant-raw.png) | [PNG](../../FPGA/lab/results/terrain-correction/after/constant-raw.png) | Uniform waterfall, no terraces; opaque front wall removed |
| Noise around −134 dB | [PNG](../../FPGA/lab/results/terrain-correction/before/noise-raw.png) | [PNG](../../FPGA/lab/results/terrain-correction/after/noise-raw.png) | Low dark background, median normalization 0.0600 |
| Same noise + −112/−58 dB carriers | [PNG](../../FPGA/lab/results/terrain-correction/before/carriers-raw.png) | [PNG](../../FPGA/lab/results/terrain-correction/after/carriers-raw.png) | Both single-bin carriers retained as narrow traces/ridges |
| One −60 dB broadband pulse | [PNG](../../FPGA/lab/results/terrain-correction/before/pulse-raw.png) | [PNG](../../FPGA/lab/results/terrain-correction/after/pulse-raw.png) | One contiguous event, no periodic copies |

Every fixture uses the same fixed −140/−40 range, gamma 1, height control 0.65,
zero smoothing, Balanced geometry, 4096 bins, 33 ms cadence and 1800 frames
(three buffer wraps). Only the explicitly documented geometry correction differs.
Each fixture checks 1312 GPU waterfall sample positions against numerical ring
values; all 512 retained rows and timestamps match the recorded fixture frames.
The constant waterfall's maximum channel spread is **zero** before and after.
Its front-wall test pixel changes from `[24,48,80]` to background `[1,3,12]`.
Constant upper samples at four different ages have the same palette color.

The last 512 recorded frames are also fed to Traditional with the same Reference
palette, floor/ceiling, contrast 100 and zero smoothing. Per-bin Traditional RGB
ring values match the recorded levels and ages at selected rows/columns; 3D
retains their exact Float32 values and timestamps. The production presentations
can otherwise differ intentionally because Traditional averaging, peak hold,
contrast and range remain independent. No claim of identical colors is made
when those settings differ.

Frequency checks cover left, center and right at front and receding rows, zoom
1/4/16, desktop/phone sizes and DPR 1/1.25/1.5/2. Surface picking, waterfall
mapping and ruler mapping agree with **0 Hz maximum observed error** in these
flat-surface checks (allowed tolerance: one geometry column). This is in addition
to the real PointerEvent tuning/drag/release-over-TX checks in the standard gate.
The backing canvas tracks CSS dimensions at the selected tier's DPR/pixel cap;
effective scale is now visible in diagnostics. Emulated DPR is not a real
cross-monitor or Safari test.

Pixel checks allow 3/255 RGB LUT/raster rounding; known-amplitude leading-edge
heights allow two physical pixels for line rasterization. Screenshot inspection
allows two pixels of antialiasing and font variation, not frequency/time shifts.
Test-only GPU readbacks are confined to browser validators.

## Validation and limits

Run from `update_manager/remote-web`:

```sh
npm test
npm run typecheck
npm run check:seam
npm run build
npm run validate:waterfall
CHROMIUM=google-chrome npm run validate:remote-next-layout
# Standalone controlled test (also included by validate:waterfall):
node scripts/validate-terrain.mjs --controlled=after --output=/tmp/terrain-correction-after
```

The full unit suite has 515 passing tests. The 185-export template seam,
typecheck, production build, original waterfall checks, integrated 3D tests and
controlled comparisons pass. The standard gate retains 100 switches, numerical
history/pause preservation, single-row uploads, context-loss/Canvas2D fallback,
raw cursor checks and command isolation. Responsive layout results and command
logs are archived beside the screenshots.

No live radio, audio-continuity, hardware-GPU, Safari/iOS or transmission test was
performed. The earlier 30-minute soak was not repeated for this targeted pass;
this pass exercises three wraps per fixture plus the existing switch/lifecycle
gate. The operator's original screenshots and actual source distribution remain
unavailable; matching that exact presentation still needs their comparison.
No deployment or service restart is included.
