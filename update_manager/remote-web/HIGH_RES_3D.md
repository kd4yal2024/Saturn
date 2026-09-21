# High-Res 3D operating display

High-Res 3D is an optional presentation of the existing `/remote-next` live FFT.
Use **View → High-Res 3D**, or **Traditional** to return. The initial default is
Traditional. No transport, audio, DSP, receiver, VFO, filter, gain, or transmit
command belongs to the view selector. The current server already redirects
`/remote` to `/remote-next`; this feature does not change that routing.

## Operating

- Left to right is RF frequency. Height is **relative dB** from the browser FFT,
  not dBm. The FFT computes `20 log10(hypot(re, im) / N + 1e-8)` after its Hann
  window. No window-gain correction or absolute power calibration is added.
- The newest spectrum is at the front. Older spectra recede into the upper
  surface. The newest waterfall row is at the top; older rows move down. These
  are **overlapping histories**, not consecutive portions of a time tape.
- Click/touch tuning, passband handles, wheel tuning, pinch zoom and receiver
  markers retain the existing operating controls. The upper terrain uses
  projected triangle picking; its top frequency ruler and the lower waterfall
  remain direct frequency surfaces. A drag holds its initially picked frequency
  plane so a tuning-triggered history boundary cannot interrupt the gesture.
  Camera adjustments use the settings only.
- **Display Settings** contains surface height, recent-history depth, camera
  elevation, manual floor/ceiling, gamma, spatial smoothing (including zero),
  palette, split, quality, diagnostics, and reset. Start the floor near the raw
  noise level and the ceiling near strong signals; the default −140 / −40 dB
  range is a starting point, not a measured hardware calibration. It also links to the existing
  Traditional appearance controls. The divider remains adjustable with pointer
  and keyboard; the split slider also works on phones.
- **Fit range to current signals** samples the current numerical spectrum once,
  places the background near the dark end, and holds that manual range. It uses
  the 20th/99.5th percentiles with headroom and a minimum 45 dB range so quiet
  noise is not expanded across the palette. Strong isolated carriers can saturate
  visually; raw readouts remain unchanged. Use this if the initial range looks
  predominantly blue. Reference Blue / Rainbow provides vivid low-level blues; Reference Dark keeps
  the earlier subdued colors. Gamma 0.85 remains the default. Existing saved range/gamma values remain intact until edited.
- **Pause** freezes display ingestion/drawing without stopping audio. Changing
  view retains pause. Resume preserves retained history and records elapsed
  missing intervals when the next spectrum arrives. This page had no previous
  display-pause control; the new control is presentation-only.
- **Reset 3D view** resets only 3D visual settings and preserves the selected mode.
  Traditional palette, auto range, averaging, peak hold and visual effects remain
  separate. Manual 3D range is stable and remaps retained numerical levels;
  palette gamma never changes geometry. Height never changes color or measurements.
- A gray horizontal interval means missing measurements. Frequency, receiver,
  source width, zoom, sample rate or row-cadence changes start a new, labeled
  history segment. Switching presentations does **not** start a segment.
- More pixels or mesh vertices do not increase the radio's FFT resolution. The
  cursor reports an actual sampled per-bin maximum from its timestamp bucket,
  not a value reconstructed from color or mesh height. It is not a power meter.

## Data and rendering boundary

`handleIqFrame` retains the existing bounded IQ packet window. The existing
animation scheduler transforms each accepted new IQ version with `FftProcessor`.
`visibleBinsForDisplay` crops and reverses bins **once**. `acceptSpectrumFrame`
then hands the raw oriented levels to `SpectrumHistory`, before Traditional's
averaging, peak hold, visual smoothing and palette conversion. Offline decorative
spectra are never ingested into the new history. Deterministic synthetic spectra
exist only in the browser validator, which calls the same adapter.

`src/dsp/spectrum-history.ts` owns a 512-row Float32 circular buffer, one owned
latest-raw array and per-row metadata. Each input sequence is accepted at most
once. Within a timestamp bucket, it takes a sampled maximum independently in each
bin. This preserves short narrow signals for that bucket only; it is explicitly
not integrated numeric power. The bucket width follows the existing display
interval times waterfall-speed setting (normally 33 ms × speed; WAN intervals
come from the existing profile). Missing buckets are not interpolated. Long gaps
are capped at the ring capacity, with no backlog replay. Changing cadence starts
a segment so old seconds are not reinterpreted. CPU numerical levels are capped
at 32 MiB; unsupported dimensions produce Traditional with an explanation.

History metadata records RX/TX receiver identity, source sequence, first/last
measurement timestamp, source FFT width, center, visible span, source sample
rate, units, mapping epoch and aggregation count. Producer buffers are copied.
The store exposes borrowed row views to renderers, which must not modify them.
The current page consumes receiver 0; this change does not invent a second RX.

`src/render/terrain.ts` owns one R32F numeric texture ring, a 1024-entry RGBA8
palette texture and a reusable indexed height-field topology. R32F uses nearest
`texelFetch`; neither float linear filtering nor float color attachments are
required. The palette uses ordinary interpolated RGBA8 sampling, not HDR output.
Only new/changed rows upload in steady state. Initial creation, resize of the
source, recovery and explicit mapping boundaries can upload a bounded full ring.
Texture head arithmetic is independent of geometry age. Strips use primitive
restart, and invalid-data triangles are discarded: newest/oldest or missing
intervals are never joined by a surface.

The constrained projection leaves the front frequency edge horizontal. The
front edge is a thin measured outline, with no opaque skirt or per-row walls.
The surface and waterfall share unlit amplitude colors. With cleanup Off, height remains linear
in clamped dB normalization. Optional cleanup applies the documented monotone
soft knee near a held noise baseline before color and height mapping. There is
zero height offset and a clip-space multiplier of 0.75 times Surface height. CPU picking uses the same projection, perspective-correct
triangle interpolation and sampled source levels; there are no runtime GPU
readbacks. Peak reduction across source bins is used for mesh columns and
waterfall pixels. When the waterfall is shorter than its 512 rows, each pixel
takes the peak over the explicit timestamp buckets it intersects, so one-row
impulses are not skipped. Pixels touching a missing bucket are conservatively
marked gray. This is display reduction; the cursor still samples the original
numeric frequency/time bucket. Optional spatial smoothing is convex and display-only; no
spline overshoot or temporal peak-hold mountains are introduced.

The shared visible-bin helper also fixes an existing high-zoom error on smaller
FFTs: it no longer forces 128 source bins into a ruler span representing fewer
bins. Normal/default Traditional extraction is unchanged. No FFT size or radio
sample rate is changed by the new view.

## Settings, lifecycle and resources

Validated settings live under `displayPrefs.terrain`, using the existing local
fallback, profile import/export and settings-save mechanism. Old profiles default
to Traditional; invalid modes, palettes, quality values and non-finite numbers
receive defaults; numeric values are bounded and floor is always below ceiling.
These fields are not part of radio preferences or radio command construction.
The existing settings/profile scope is unchanged.

There is one animation loop, the existing application's RAF loop. The active 3D
renderer uses it; inactive 3D rendering stops immediately. Its warm cache expires
after 30 seconds and is deterministically deleted, including its context and
context-loss listener. Existing Traditional renderers retain their fixed,
bounded caches to support immediate return. Their continuous draw calls are
suppressed in 3D mode. Traditional WebGL waterfall ingestion remains as before;
Canvas2D reconstructs retained history from numeric rows on return.

Context loss retires the failed renderer, shows Traditional immediately and
preserves numerical history. The status explains the failure. If the legacy
contexts are also lost, fresh canvases reuse the existing Traditional Canvas2D
renderers and rebuild measured history. A Canvas2D history taller than the numeric
store stays Traditional with an explanation, rather than silently losing its
retention when switching to 3D. Explicitly select
3D to create a fresh context and rebuild; there is no endless automatic retry.
Page hide disposes the new renderer; BFCache restoration reconstructs it. Hidden documents skip its drawing, and
resume never drains an unbounded render queue. Canvas dimensions track actual
CSS size and device pixel ratio every active frame, with pixel/memory/driver
limits applied. Overlays remain normal DOM pixels even when mesh resolution is
reduced. No new transport, framework, 3D engine, runtime dependency or CDN asset
is used.

| Quality | Maximum mesh columns × rows | DPR cap | Pixel cap | Refresh target |
| --- | --- | --- | --- | --- |
| Performance | 512 × 48 | 1 | 1.5 million | 30 fps |
| Balanced | 1024 × 128 | 1.5 | 3 million | 60 fps |
| High | 2048 × 256 | 2 | 5 million | 60 fps |
| Auto | Starts Balanced; reduces to Performance | tier dependent | tier dependent | tier dependent |

Actual geometry is also limited by available bins and the chosen recent depth.
Auto demotes after sustained slow frames with a five-second holdoff. It attempts
recovery only after twenty seconds of stability, doubling the recovery wait
(up to five minutes) after unsuccessful trials to avoid repeated oscillation. It changes geometry, pixel
resolution and visual refresh, never radio/DSP settings, row cadence or stored
levels. Targets are not claims of hardware performance.

The 3D view budgets at most approximately 96 MiB for its numeric CPU/GPU copies,
index storage and render targets; actual allocation is normally much smaller.
For 4096 × 512, samples alone are 8 MiB on the CPU and 8 MiB on the GPU. Metadata,
latest raw bins, revision arrays, palette, index buffer and estimated double
color/depth targets are additional. Diagnostics report the computed configuration,
including a separate estimate for retained Traditional buffers. Driver overhead
and browser compositor allocations are not measurable by this estimate.

## Verification and evidence

Run from `update_manager/remote-web`:

```sh
npm ci
npm test
npm run typecheck
npm run check:seam
npm run build
npm run validate:waterfall
CHROMIUM=google-chrome npm run validate:remote-next-layout
# Standalone integrated 3D checks and the requested wall-clock soak:
npm run validate:terrain
npm run validate:terrain -- --soak=1800 --output=/tmp/saturn-terrain-soak
```

`validate:waterfall` retains every previous ring-wrap, single-row-upload,
palette-continuity, tuning-shift, dB scale and Canvas2D test. It adds Reference
Rainbow continuity and invokes the integrated 3D browser validator. The latter
loads the full current template and built runtime, blocks socket construction,
labels screenshots **synthetic**, and exercises the production adapter and
renderer. It includes steady and nearby carriers, a voice-like envelope, a weak
carrier, impulses, drifting tone, known amplitude steps and missing intervals.
It checks numerical waterfall pixels, GPU leading-outline heights at known amplitude steps,
compressed single-row impulses, projection, 100 view cycles, presentation
command isolation, pause state, hidden drawing, loss/recovery, unsupported
WebGL2, actual 30-second inactive-cache expiry, page-restoration handlers,
and desktop/phone layouts. With `SATURN_TERRAIN_BASELINE` pointing to a frozen
pre-edit template, it also compares Traditional pixels against that original
renderer and captures a side-by-side screenshot. Existing TX safety and transport tests remain
in the full test suite.

Numerical tests require exact sample ownership/order and explicit timestamp
mapping. RGB comparisons allow at most 3/255 per channel for LUT/raster rounding.
Projected picking tolerates one geometry column at sampled vertices. Browser
screenshots are inspection evidence, not a substitute for these comparisons:
allow up to two physical pixels at antialiased edges and font rasterization
variation, but no frequency, time, passband or amplitude displacement. The
reference PNG was not available in the supplied workspace, so visual comparison
is against the written composition only.

The [validation report](VALIDATION_HIGH_RES_3D.md) records actual commands, screenshots,
resource estimates and timings. Chromium phone/tablet emulation is not Safari
or iOS testing. Live radio/audio continuity, hardware GPU targets, real Safari/
iOS interaction and cross-monitor movement require operator hardware and must
not be inferred from these synthetic runs. No radio deployment, appliance
restart, merge or transmission is part of this change.

## Disable / revert

Select **Traditional** and save the profile if desired; the 3D cache expires
within 30 seconds. To restore an old exported profile, import it: missing terrain
settings default to Traditional. For code rollback, revert the feature's source,
HTML and seam changes together, then rebuild the bundle and run `check:seam`.
No backend, radio firmware, service or route rollback is needed.


## Targeted rendering correction

See [rendering diagnosis and controlled before/after evidence](RENDERING_CORRECTION.md).
**Display Settings → Level / rendering diagnostics** shows CPU source and stored
bucket distributions, floor/ceiling, normalized median, clipping percentages,
height scale/offset, and effective pixel density. It updates once per second
while enabled. It performs no GPU readback. Use these values before fitting a
manual range; an unexpectedly high normalized background must not be dismissed
as a palette issue. The range remains fixed until explicitly edited or fitted.

**Grid opacity** is independent of Traditional preferences and defaults to zero.
Lower waterfall band badges are suppressed while boundary lines remain visible;
the short history status has a tooltip explaining time direction and boundaries.


## Reference Blue / Rainbow and passband refinement

See the [same-data comparisons and validation](REFERENCE_BLUE_REFINEMENT.md).
Choose **Reference Blue / Rainbow** for vivid instrument colors or **Reference
Dark** for the preceding subdued palette. Both are unlit and use the same levels
and height mapping. The 3D RX passband uses a 4% rectangular fill with thin side
boundaries; full filter values are also available in its tooltip. USB/LSB
coordinates and all operating gestures are unchanged.


## Noise-floor cleanup

See [mapping, weak-signal proof and identical-data comparisons](NOISE_FLOOR_CLEANUP.md).
**Noise-floor cleanup** defaults to 60%; set it to zero for the preceding view.
The baseline estimate is held per history segment, with an explicit **Re-estimate
noise baseline** button and manual override. No blur or time averaging is added.
Palette/gamma, height exaggeration and floor/ceiling remain separate controls.
Measurements and the raw cursor stay unchanged; cleanup softens only the displayed
near-floor values. Above baseline +8 dB, the original mapping is restored.
