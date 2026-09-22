# Lower waterfall sharpness correction

This is a targeted change to the High-Res 3D renderer. Traditional rendering,
radio commands, FFT processing, history ingestion, palette, cleanup curve,
surface geometry and picking calculations are unchanged.

## Diagnosis and changes

The lower waterfall shares the terrain canvas. Previously Performance capped
its backing density at 1, Balanced at 1.5 and High at 2. A DPR-2 screen running
Balanced therefore composited an undersized image, softening narrow traces.
All tiers now use the device density, including fractional density. The existing
96 MiB view budget and GPU dimension checks remain; a shared eight-million-pixel
ceiling also bounds allocation. Diagnostics expose resource-driven reduction.
Quality still controls mesh columns, recent surface rows and render cadence.
Increasing backing resolution also increases upper raster resolution; its shape,
colors and mesh topology have not changed.

The previous temporal reduction used floor(start) and ceil(end) for each pixel.
At fractional history-to-pixel ratios, adjacent pixels therefore included the
same boundary bucket. This was peak dilation, not linear blur: a one-row event
could brighten two neighboring rows and neighboring noise rows became correlated.
The revised integer calculation uses disjoint partitions when compressing:

```
top = lowerHeight - 1 - framebufferY
firstAge = floor(top * capacity / lowerHeight)
endAge = max(firstAge + 1, floor((top + 1) * capacity / lowerHeight))
```

Each compressed source bucket contributes to exactly one output row, preserving
brief events without duplicating them. When there are more pixels than retained
rows, the owning bucket is repeated without blending; this is explicitly display
magnification, not extra measurements. Temporal quantization is bounded by one
history bucket. Any missing bucket in a partition still marks that pixel gray.
Physical ring addressing happens separately for each logical age, never by
filtering across the texture seam. Source-bin maxima within disjoint output
columns still preserve narrow energy; they are not numerical power measurements.

The scalar texture was already R32F with a highp sampler, NEAREST configuration
and texelFetch. It remains so. LINEAR filtering is used only for the color LUT.
There is no waterfall spatial smoothing, temporal averaging, postprocessing blur,
scanline effect or new row decimation. Incremental row uploads are retained.
Canvas density is checked during rendering, including resize and monitor changes.

## Reproduction and evidence

`node scripts/validate-terrain.mjs --sharpness=after --output=<directory>` runs
six deterministic fixtures through the production `acceptSpectrumFrame` adapter:
constant input, noise, weak/strong and nearby narrow carriers, a brief weak event,
a broadband pulse, and a missing interval. Each uses 1,800 frames at 33 ms,
crossing the 512-row ring three times. Noise seed, timestamps, bins, viewport,
quality, range (-140 to -40 relative dB), gamma (1), cleanup (60%, baseline -129)
and smoothing (Off) are identical in each before/after pair. No radio is connected.
The frozen pre-edit bundle is selected with `--bundle` and `--sharpness=before`.

Evidence lives in `FPGA/lab/results/waterfall-sharpness/` (ignored local artifacts):
`comparison.html`, `before/`, `after/`, `controlled/`, `cleanup/`, and `lifecycle/`.
PNG files are actual renderer/page captures. The gallery includes noise and
carrier comparisons at desktop, fractional scaling and phone sizes. Open images
at their native size to avoid viewer downscaling obscuring one-pixel traces.
GPU comparisons allow 3 RGB code values for LUT/raster rounding. Full UI screenshots
are presentation evidence, not an exact cross-browser font/antialiasing contract.

Constant input was uniform before and after (zero RGB spread), so these tests do
not establish a periodic-stripe or ring-corruption defect in the former renderer.
Measured interference, gaps and source-cadence aggregation must not be mistaken
for a shader bug. No user RF recording/current screenshot was available in this
workspace; the captures demonstrate controlled synthetic input, not the user's
particular band or live hardware.

## Checks and limits

Executed: `npm ci`, `npm test` (520 tests / 66 files), `npm run typecheck`,
`npm run check:seam` (186 exports), `npm run build`, `npm run validate:waterfall`,
`CHROMIUM=google-chrome npm run validate:remote-next-layout`, and terrain validators
for lifecycle, controlled amplitude/alignment, cleanup and sharpness fixtures.
The first layout attempt could not find the default `chromium` executable;
using the installed `google-chrome` succeeded. An initial build invoked from the
repository root failed because the frontend package is in `update_manager/remote-web`;
it was rerun there successfully. npm ci reported six existing dependency audit
findings; dependencies and lockfile were not changed in this rendering pass.

Regression coverage retains Traditional WebGL2/Canvas2D checks, palette continuity,
ring wrap, single-row uploads, tuning shifts, 100 mode cycles, state retention,
context loss/recovery, cache expiration and presentation controls issuing no radio
commands. Controlled tests compare raw history/frequency alignment and retained
Traditional colors; cleanup tests still verify weak-carrier/event preservation.

Browser execution uses Chromium SwiftShader on the development host, including
phone emulation. Actual G2 live RF/audio, physical GPU frame rates, Safari/iOS and
a fresh 30-minute soak were not run. Full-density rendering costs more pixels;
30/60 fps remain targets, not claims established by these static captures.

## Measured comparison results

All 24 before/after pairs passed. Revised GPU colors at 144,276 sampled pixels
(including neighbors beside carriers) agree with numerical maxima to the stated
3-code tolerance. Weak +2.5 dB single-bin carriers and brief events remain visible;
nearby carriers remain separate. Missing intervals remain explicitly gray.

| Viewport, DPR, quality | Lower backing before → after | Estimated 3D view memory after |
| --- | --- | --- |
| 1440×1000, 1, Balanced | 914×328 → 914×328 | 21.98 MiB |
| 1137×901, 1.25, Performance | 840×437 → 1050×546 | 25.65 MiB |
| 1440×1000, 2, Balanced | 1371×491 → 1828×655 | 36.66 MiB |
| 390×844, 2, Performance | 344×328 → 688×657 | 23.66 MiB |

At DPR 1 (identical backing resolution), the deliberately boundary-positioned
single-row pulse and brief weak event each occupied two screen rows before and
one afterward. This isolates the temporal correction from resolution changes.
At DPR 1.25, the revised 546-pixel waterfall enlarges 512 buckets; two screen rows
for some buckets are legitimate magnification, without blended neighboring data.
Upper-region decoded pixels are byte-identical across all six DPR-1 comparisons.
Memory estimates include numerical CPU/GPU history, mesh indices, LUT and estimated
color/depth buffers, but exclude Traditional caches and driver overhead.

## Live-profile follow-up: source resolution before rendering

The initial fixtures above used 4096 already-visible bins. That missed a real
G2 configuration sampled on 2026-09-21: desktop layout, WAN RX transport,
384-ksps Direct-XDMA IQ, and 4× display zoom. The browser's WAN profile chose
a 2048-point FFT, leaving only **512 visible bins** for a roughly 1828-pixel
device-density waterfall. The lower image was never high-resolution in that
configuration; increasing its canvas backing size alone could not repair it.
The saved 3D floor, ceiling, gamma and cleanup were also being adjusted during
the investigation, so their contribution to the operator's perceived contrast
is not assigned a fixed value here. No saved G2 setting was changed by this work.

Desktop High-Res 3D now chooses its FFT from the actual backing width and zoom,
also seeking about one 30-Hz IQ frame of time coverage. The result is capped at
16384 points, the GPU/history texture limit, and a 50-ms FFT aperture so
lower-rate sources do not smear many history rows. At 1× zoom on an 8192-wide
GPU the target is capped at 8192 to avoid an unsupported history texture. In
the observed 384-ksps, 4× setup this selects 16384 points:
4096 visible bins, a 42.67-ms input window, and 2.24 input bins per backing
pixel. The old WAN window was 5.33 ms. Traditional and the phone WAN profile
retain their previous targets. Radio transport, IQ rate, audio, TX and FPGA
handling are unchanged. Diagnostics now expose the display profile, target FFT,
visible-bin/backing-pixel ratio and FFT window duration.

`node scripts/validate-terrain.mjs --wan-sharpness=1 --output=<directory>`
compares 2048 and 16384-point FFTs using the *same deterministic IQ waveform*,
viewport, DPR, zoom, range, palette, cleanup and quality. It feeds the actual
FFT/zoom/history/renderer path, not precolored rows. Two carriers 300 Hz apart
merge under 2048 points and separate with a dark output-pixel gap under 16384.
A 5.3-ms event early in one
IQ frame is absent from the old last-2048-sample window but appears 48.8 dB
above its neighboring row with the new window. The 512-row history remains
ordered. The fixture images and machine-readable results are in
`FPGA/lab/results/waterfall-sharpness/wan-zoom/` (ignored local artifacts).
The 50-transform Chromium software-browser average was 0.19 ms at 2048 and
1.15 ms at 16384; this is not a hardware performance or audio-continuity claim.

This follow-up was deployed to the G2 web root on 2026-09-22 (JS SHA-256
`863d45d1792b71ed20c072642d73c8881c0858f0e84415d0d175a8b1fab9086e`,
HTML SHA-256
`417910ea5850b0c656933e377705661147f97eb21285416aeec6406238c34709`).
The operator reported improved detail at 2× zoom but continuing horizontal
flicker. The original 24-pair resolution/row-boundary comparisons remain valid
for their fixed-bin inputs; they did not validate the live WAN/zoom source path.

## Follow-up: WAN history cadence flicker

The Direct-XDMA Bridge targets 30 display-IQ frames/s, but the desktop WAN
browser path presents a selected FFT no more often than every 50 ms. The old
history adapter used the *source frame's arrival timestamp* for that selected
FFT. A 60 Hz browser with steady 30 Hz IQ therefore selects rows at 50 ms
intervals whose arrival times alternate between 33 and 67 ms apart. Quantizing
those arrivals to 50 ms history buckets creates a missing bucket followed by
an aggregated bucket, despite continuous IQ. The shader truthfully draws those
artificial missing buckets gray; as they descend they appear as horizontal
flicker. This is a display timestamp-domain mismatch, not texture filtering or
an FPGA/DMA data loss finding.

The page now timestamps each history row at FFT presentation time (`now` from
the animation callback), while retaining the source sequence for deduplication.
The 50 ms render gate has 1 ms tolerance for RAF floating-point alignment. If
IQ actually stalls, the `hasNewIq` gate still prevents ingestion and the history
still records a real gap when frames resume. No source bins are averaged,
interpolated or invented; radio, audio, TX and Traditional rendering are
unchanged.

`CHROMIUM=/usr/bin/google-chrome node scripts/validate-terrain.mjs
--cadence-flicker=1 --output=/tmp/saturn-cadence-flicker-20260922` exercises the
actual page adapter and WebGL renderer with identical synthetic 30 Hz source
frames, 60 Hz RAF schedule and 50 ms WAN presentation gate. Across 41 selected
rows, arrival timestamps yielded 20 artificial missing buckets, 20 aggregated
buckets and 20 GPU gray rows; presentation timestamps yielded zero of each.
The before/after canvas captures are `cadence-arrival.png` and
`cadence-presentation.png` in that output directory. A unit regression test
also confirms a genuine source hiatus still produces missing rows. The cadence
HTML candidate was deployed to the G2 on 2026-09-22 (SHA-256
`feaaedceb04fad297a169c8f80888cc41600a6048b05d2c6b53874cabacd3d1d`).
The operator reported residual flicker after this change. Live diagnostics
then showed zero missing/aggregated rows, a stable 20 Hz accepted source and
60 fps rendering; this residual is not the earlier artificial-gap mechanism.

## Follow-up: separate lower-waterfall cleanup

The G2 diagnostics at the residual-flicker report showed a held -118 dB noise
baseline, -140 to -40 dB range, 85% shared cleanup, gamma 1.35 and the
Enhanced palette. Under the previous 4 dB cleanup shoulder, a change from
-114 to -112 dB moved the Enhanced blue channel from roughly 14 to 97, even
though no history rows were missing. The palette and scalar transfer amplified
small RF-level changes into conspicuous horizontal color changes.

The new `waterfallCleanup` setting is independent of the upper surface cleanup.
It defaults to 90%, ranges from Off to 100%, and uses a pointwise monotone
24 dB shoulder above the same held noise baseline. At the measured profile,
the -114 to -112 dB blue change is about 59 to 86 while a -118 dB background
remains near blue 16. Above baseline +24 dB, the original amplitude mapping is
restored. No source bin, history row, FFT, radio command or Traditional renderer
is altered. Separate upper/lower legends and diagnostics expose the difference.

`CHROMIUM=/usr/bin/google-chrome node scripts/validate-terrain.mjs
--cleanup=after --output=/tmp/saturn-waterfall-cleanup-20260922` runs seven
fixed-signal fixtures through the actual WebGL renderer. At 90% lower cleanup
with the Reference palette, synthetic noise mean luminance fell from 40.35 to
15.27, while a +2.5 dB single-bin carrier's RGB distance from background rose
from 19.72 to 50.05. Its footprint stayed one column; a brief one-row event
stayed one row. All upper-surface pixels were byte-identical with lower cleanup
Off and On. Noise luminance variation itself was not reduced (standard deviation
0.87 to 1.48 for the narrow-noise fixture); no pointwise color transform can
distinguish a weak signal from noise at the same measured level. The web-only
candidate was installed on the G2 on 2026-09-22 and both served-file hashes
were verified: JS
`151da35180130ada869222b9ea57b88b0559b97b8c32ff2118f40f53bd10bc77`
and HTML
`5a1a1daf3c3b93c302378ccbb07cfdd6ceb441a7c9ff2f6ba8b3417cd90ee857`.
The live visual result has **not yet been reported**; the controlled tests do
not establish that all on-air flicker is gone.
