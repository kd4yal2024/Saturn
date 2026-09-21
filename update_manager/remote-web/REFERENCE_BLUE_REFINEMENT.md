# Reference Blue / Rainbow refinement

This targeted pass starts from `038fe7a`. The connected mesh, thin leading trace,
height scale, fixed range, history, transport and operating gestures are unchanged.
The front fence stays removed. No lighting, blur or temporal averaging was added.

## What changed

- **Reference Blue / Rainbow** (`reference`) now maps midnight blue through royal
  blue, electric blue, cyan, green, yellow, orange, red and white. Low-level blue
  detail is brighter without changing geometric height or normalized levels.
- **Reference Dark** (`reference-dark`) retains the exact preceding palette.
  It is validated and persisted through the existing preference path. Existing
  Reference selections use the refined Reference preset; choose Reference Dark
  to recover its former appearance. Classic/Ember/Enhanced and other palettes
  are unchanged.
- In 3D, the RX passband is a **4% opacity rectangular fill**, with thin vertical
  boundaries and no rounded capsule, horizontal border or inset shadow. The
  compact label can extend beyond the narrow filter width; its tooltip and the
  filter's tooltip include the complete mode and filter values. Drag handles,
  hit areas, actual frequency coordinates and TX overlay behavior remain intact.

The shader was already unlit. There is no fog, shadow, depth-based darkening or
landscape lighting to disable. Known constant levels match the same palette
color in the upper surface and waterfall, at several depths. The muted appearance
was reproducible from the old palette alone, not from a lighting defect.

Controls remain independent: palette/gamma affect color, height affects geometry,
and manual floor/ceiling select the level range. No per-frame stretching is used.
A quiet capture does not acquire synthetic warm peaks. The one-shot range-fit
button remains optional and was **not used** for the comparisons.

## Same-data screenshots

[Before/after gallery](../../FPGA/lab/results/terrain-reference-blue/comparison.html)
contains actual renderer PNGs, not mockups. Each pair has identical 4096-bin
samples, timestamps, 33 ms cadence, 1800 frames (three wraps), viewport, Balanced
geometry, −140/−40 relative-dB range, gamma 1, height 0.65 and smoothing off.

- Noise only: [before](../../FPGA/lab/results/terrain-reference-blue/before/noise-raw.png) / [Reference Blue](../../FPGA/lab/results/terrain-reference-blue/after/noise-raw.png)
- Noise with weak/strong carriers: [before](../../FPGA/lab/results/terrain-reference-blue/before/carriers-raw.png) / [Reference Blue](../../FPGA/lab/results/terrain-reference-blue/after/carriers-raw.png)
- Constant background + stationary carrier: [before](../../FPGA/lab/results/terrain-reference-blue/before/carrier-flat-raw.png) / [Reference Blue](../../FPGA/lab/results/terrain-reference-blue/after/carrier-flat-raw.png)
- Six-frame broadband increase: [before](../../FPGA/lab/results/terrain-reference-blue/before/broadband-raw.png) / [Reference Blue](../../FPGA/lab/results/terrain-reference-blue/after/broadband-raw.png)
- [Desktop with operating overlays](../../FPGA/lab/results/terrain-reference-blue/desktop.png) / [phone viewport](../../FPGA/lab/results/terrain-reference-blue/phone.png)

These are explicitly synthetic measurements. The original reference image and
operator's latest screenshot were not available as files/attachments, so their
exact visual comparison remains unavailable. The existing mixed fixture's repeated
horizontal pulses and gaps are deliberately supplied input, not grid decoration.

## Shelf, bands and detail findings

| Test | Observed result |
| --- | --- |
| Constant level, every bin/frame | Flat surface; no additional shelf or depth tint. Waterfall channel spread is exactly zero through multiple ring wraps. |
| Stationary narrow carrier on constant background | Carrier and background values are constant through every retained row; a continuous narrow ridge/trace remains. |
| Six-frame broadband increase | Elevated values occupy exactly ages 119–124 (six rows, 198 ms). The corresponding rear raised region is preserved. |
| Single-row broadband pulse | One contiguous visible waterfall event, no periodic copies. |
| New palette versus dark preset | CPU history and projected geometry unchanged. All six dark-preset raw PNGs are byte-for-byte identical to the preceding implementation's PNGs. |
| Known levels above/below seam | Same unlit base palette colors; no age-dependent darkening. GPU comparisons retain 3/255 channel tolerance. |

The source distributions and normalized median are unchanged: the noise fixture's
median normalization remains approximately 0.0600. The revised palette reveals
its texture with greater blue separation. The synthetic sinusoidal noise creates
visible diagonal/fine texture; that structure is in the fixture samples. It was
not added by the renderer. No global blur or source interpolation was introduced.

Scalar storage remains full-width R32F with high-precision nearest `texelFetch`.
Frequency-column and compressed-time reduction preserve peaks. GPU waterfall
pixels are compared with those exact numerical reductions through the physical
ring seam. There is no detected row-addressing or missing-upload defect to hide.
Backing resolution and quality caps are unchanged and remain visible in
Diagnostics. Resize/zoom checks include fractional DPR 1.25/1.5, DPR 1/2, and zoom
1/4/16; observed mapping error was 0 Hz for left/center/right flat-surface samples.
A lower quality tier still reduces geometry/backing resolution explicitly; this
pass does not claim extra RF resolution.

The operator's particular rear shelf, dash texture and horizontal bands cannot
be classified without their source frames or capture. No real wideband changes,
interference or missing-data markers have been suppressed. This is a palette and
overlay refinement, not a claimed fix for an unobserved live history defect.

## Validation and scope

Passed from `update_manager/remote-web`:

- `npm test`: **516 tests**, 65 files, including existing TX safety checks.
- `npm run typecheck`, `npm run check:seam` (185 exports), `npm run build`.
- `npm run validate:waterfall`: original waterfall checks, 100-switch/state,
  tuning/release safety, context/fallback checks, and all six controlled fixtures.
- `CHROMIUM=google-chrome npm run validate:remote-next-layout`: **24 scenarios**.
- Frozen-bundle/template before-run of `validate-terrain.mjs --controlled=before`.

New browser assertions check the computed rectangular 4% fill, full-value
tooltips, matching Traditional/3D filter coordinates, and LSB −3050 to −50 Hz
placement versus USB +50 to +3050 Hz. Switching palettes emits no radio commands
and preserves measured levels. Browser reports and command logs are archived
with the screenshots in the ignored lab-results directory.

These are Chromium/SwiftShader tests, including phone viewport emulation. Live
radio/audio continuity, hardware GPU performance and Safari/iOS were not tested.
No deployment, service restart or transmission was performed. The earlier
30-minute soak was not repeated for this palette/overlay change.
