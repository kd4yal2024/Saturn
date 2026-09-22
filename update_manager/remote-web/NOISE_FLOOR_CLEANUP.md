# Display-only noise-floor cleanup

This document records the original shared cleanup pass. The current High-Res
3D view has separate upper-surface and lower-waterfall cleanup controls; see
[WATERFALL_SHARPNESS.md](WATERFALL_SHARPNESS.md) for the later lower-waterfall
transfer curve and validation. The measurements below remain historical results
for the original mapping.

This pass starts from `d24a76c`. It preserves the rich Reference palette,
Traditional mode, fence-free connected surface, passband presentation and radio
controls. It changes only the display mapping near a held baseline. There is
**no new spatial smoothing, temporal averaging, blur, AGC or receiver-gain change**.
Raw numerical history, timestamp buckets and cursor levels are unchanged.

## Operator controls

In **Display Settings**:

- **Noise-floor cleanup**: default 60%; zero is Off; maximum 85%. It reduces
  near-floor color prominence and background height variation together. It is
  separate from color gamma and the global Surface height multiplier.
- **Cleanup baseline**: enter a relative-dB value, or leave blank for the held
  estimate. **Re-estimate noise baseline** samples the current raw frame once.
- Palette/gamma, Surface height, and display floor/ceiling retain their existing
  independent settings. No cleanup action edits them.

The estimate is the median of finite, valid bins in the first accepted frame of
each mapping/history segment. It remains fixed across subsequent frames, mode
switches and renderer reconstruction. Retuning/zooming/source changes that already
start a new segment allow a fresh estimate. Explicit re-estimation or a manual
baseline is available; nothing follows every frame or continuously moves the
range. Thus a genuine broadband increase is not normalized away.

A median is an estimate, not a signal/noise classifier. A mostly occupied band or
an unusual initial frame can bias it. Use the diagnostics, manual baseline, or
Off for comparison. A real weak signal and random noise at the same instantaneous
level cannot be distinguished by this pointwise mapping. Raw data remains available.

## Mapping and peak preservation

Let `n = clamp((db-floor)/(ceiling-floor), 0, 1)` and baseline `b`. The soft knee
starts at `b+4 dB` and ends at `b+8 dB` (shifted down if necessary to finish at the
selected ceiling). Across this four-dB shoulder, `x` runs from 0 to 1 and
`s = x*x*(3-2*x)`. The displayed normalized value is:

```
display = n * (1 - cleanupStrength * (1 - s))
color   = existingPalette(display ** colorGamma)
height  = display * surfaceHeight * 0.75
```

At 60% cleanup, very-near-floor variation has 40% of its previous normalized
slope. The gradual shoulder restores the original mapping above baseline +8 dB.
The ceiling still maps to 1. There is no extra hard clip or dead zone: values
inside the selected range remain strictly ordered and positive. Turning cleanup
Off reproduces the previous renderer exactly in the captured comparisons.

This pointwise, monotone function uses **no neighboring samples**. The existing
peak-preserving reduction remains before it, so narrow peaks retain their source
bin/time footprint. No frequency convolution or history averaging widens carriers
or prolongs brief events. Vertex heights, CPU projected picking and fragment
colors use matching mathematics. The legend includes the same cleanup mapping;
the cursor and amplitude diagnostics still expose the original sampled levels.

The first implementation used a narrower quiet region. A broader-noise test
showed that could emphasize its upper fluctuations. The final 4–8 dB shoulder
passes both ±1.2 dB and ±4 dB background tests. This is a conservative visual
mapping, not a claim to identify every weak station automatically.

## Identical-data screenshots and measurements

[Before/after gallery](../../FPGA/lab/results/terrain-noise-cleanup/comparison.html)
contains actual GPU renderer captures using the same frames, timestamps,
914 × 468 backing viewport, 4096 bins, 1800 frames (three wraps), 33 ms cadence,
−140/−40 relative-dB range, gamma 1, height 0.65, Reference palette, Balanced
quality and smoothing Off. The comparison baseline is fixed at −129 relative dB;
the automatic first-frame estimate is within 0.003 dB of that value.

- Noise: [before](../../FPGA/lab/results/terrain-noise-cleanup/before/noise-off.png) / [after](../../FPGA/lab/results/terrain-noise-cleanup/after/noise-on.png)
- Broader noise: [before](../../FPGA/lab/results/terrain-noise-cleanup/before/noise-wide-off.png) / [after](../../FPGA/lab/results/terrain-noise-cleanup/after/noise-wide-on.png)
- Weak + strong carriers: [before](../../FPGA/lab/results/terrain-noise-cleanup/before/carriers-off.png) / [after](../../FPGA/lab/results/terrain-noise-cleanup/after/carriers-on.png)
- Brief weak event: [before](../../FPGA/lab/results/terrain-noise-cleanup/before/brief-off.png) / [after](../../FPGA/lab/results/terrain-noise-cleanup/after/brief-on.png)

All seven **Off** PNGs match the frozen preceding implementation byte-for-byte.
Cleanup changes neither raw history nor its revision. These are synthetic tests,
not live RF captures or a hardware reception/sensitivity measurement.

| Measurement | Before | Cleanup 60% |
| --- | --- | --- |
| Narrow-noise background mean pixel luminance | 40.69 | 19.94 |
| Narrow-noise pixel luminance standard deviation | 0.539 | 0.122 (77% lower) |
| Broader-noise pixel luminance standard deviation | 1.794 | 0.702 (61% lower) |
| Nominal background RGB | 6, 35, 159 | 3, 17, 85 (deep blue, not black) |
| +2.5 dB weak carrier RGB distance from baseline | 19.72 | 11.45 |
| That distance divided by background luminance variation | 36.6 | 93.7 (2.56×) |
| Weak stationary carrier footprint | 1 column, all 328 waterfall pixel rows | Identical |
| One-row weak event footprint | 1 column × 1 row | Identical |

The RGB-distance/variation ratio is a descriptive display metric, not RF SNR or
a perceptual guarantee. Absolute weak-carrier color distance decreases, but the
background fluctuation decreases more. GPU tests explicitly verify that the
weak carrier remains distinguishable and that its column/row footprint is intact.
Signals at least 8 dB above baseline—including the strong-carrier test—retain
original color and height mapping. An injected six-row +10 dB broadband increase
is preserved rather than chased by the estimate.

Every fixture checks 1312 GPU pixel positions against independently computed
mapping and source chronology. No continuous GPU readbacks were introduced;
readbacks exist only in browser tests. Runtime baseline estimation sorts a CPU
copy once per segment or explicit request; rendering adds scalar shader math,
not new textures, frame queues or FFT work.

## Validation and limitations

Passed commands from `update_manager/remote-web`:

- `npm test`: 520 tests in 66 files, including TX-safety/transport tests.
- `npm run typecheck`, `npm run check:seam` (186 exports), `npm run build`.
- `npm run validate:waterfall`: existing waterfall/3D checks, controlled geometry
  tests, and seven new cleanup fixtures with weak-carrier and brief-event checks.
- `CHROMIUM=google-chrome npm run validate:remote-next-layout`: 24 scenarios.
- Frozen old bundle/template with `validate-terrain.mjs --cleanup=before` and
  the new renderer with `--cleanup=after`.

The existing linear-mapping checks now explicitly select Cleanup Off; none were
removed or relaxed. New cleanup tests exercise the enabled path, monotonicity,
soft-knee continuity, high-level identity, held-estimate behavior, user controls,
raw-history ownership and GPU event preservation. UI changes emit no radio
commands. The existing 100-switch, tuning, passband, release-over-TX and fallback
checks remain in the full browser gate. RGB tolerance remains 3/255.

These are Chromium/SwiftShader and emulated-size tests. Live-radio weak-signal
readability, audio continuity, hardware GPU performance and Safari/iOS were not
measured. No new 30-minute soak was run for this refinement. No deployment,
service restart, DSP change or transmission was performed.
