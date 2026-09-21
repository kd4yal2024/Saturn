# High-Res 3D validation — 2026-09-21

Implementation and operator details: [High-Res 3D guide](HIGH_RES_3D.md).

The feature was developed from clean `main` at
`c674eef032e94fba7711877a1e4a9753e89d5e06`, without branch changes. The original
Traditional template was frozen before integration (SHA-256
`85b9016e56a765da64142727ec85c83b26c2f2b356105093c51484ef00603098`).
Work was first validated in an isolated copy under `/tmp/saturn-3d` because the
application sources are outside the session's writable `FPGA/lab` directory.
The apply/check script verifies source hashes before applying the reviewed patch;
its repository command log records the subsequent checks in the actual checkout.
No deployment, service restart, radio connection, merge or transmission was performed.

## Evidence

All captured signals are **synthetic**, passed through the production display
adapter into the actual template and compiled renderer. Screenshots are not live
RF measurements. The requested reference PNG was absent from the supplied
workspace; exact reference-image comparison was not possible.

- [Desktop, 1440 × 1000](../../FPGA/lab/results/remote-highres3d-2026-09-21/desktop.png)
- [Phone viewport, 390 × 844](../../FPGA/lab/results/remote-highres3d-2026-09-21/phone.png)
- [Traditional original/current pixel comparison](../../FPGA/lab/results/remote-highres3d-2026-09-21/traditional-comparison.png)
- [Final functional results](../../FPGA/lab/results/remote-highres3d-2026-09-21/final-functional.json)
- [Full 30-minute soak samples](../../FPGA/lab/results/remote-highres3d-2026-09-21/final-soak.json)
- [Repository check results](../../FPGA/lab/results/remote-highres3d-2026-09-21/repository-checks.json)

These local artifacts are in the repository's ignored lab-results directory;
archive them separately when sharing the source change.

## Commands and coverage

The package scripts were inspected before execution. Commands run from the
isolated `update_manager/remote-web` all passed:

| Command | Result |
| --- | --- |
| `npm ci` | Installed unchanged lockfile; sandbox-blocked esbuild retried with approval |
| `npm test` | 511 tests, 65 files, including existing TX safety/transport checks |
| `npm run typecheck` | Passed |
| `npm run check:seam` | Passed, 184 template/bundle exports |
| `npm run build` | Passed |
| `npm run validate:waterfall` | Existing checks plus integrated 3D browser checks passed |
| `CHROMIUM=google-chrome npm run validate:remote-next-layout` | 24 existing responsive scenarios passed |
| `npm run smoke:remote-next` | Passed |
| `node scripts/validate-terrain.mjs --output=/tmp/saturn-3d/final-functional` | Final integrated failure/lifecycle tests passed |
| `node scripts/validate-terrain.mjs --soak=1800 --output=/tmp/saturn-3d/final-soak` | Full 1800-second synthetic soak passed |
| `SATURN_TERRAIN_BASELINE=/tmp/saturn-3d/traditional-baseline.html node scripts/validate-terrain.mjs` | 316,800 Traditional pixels compared; maximum channel difference 0 |

The final functional run additionally tests simultaneous loss of all three
WebGL contexts, recovery through the existing Canvas2D renderers with retained
numerical history, and explicit 3D retry. That exception-path addition followed
the soak launch; the renderer/shaders used by the soak are the final versions.
The independent final functional run covers the completed exception path.
Original waterfall baseline and layout checks were captured before edits.

The browser checks exercise 100 mode cycles, unchanged radio state and paused
history, real 30-second inactive-cache expiration, context loss, unsupported
WebGL2, raw cursor values, projected tuning and dragging through a QSY boundary,
and release over TX coordinates. Display/settings controls emitted no radio
commands. Tuning emitted only the expected VFO/DDS commands. Existing safety
interlocks remain covered by the full test suite. Synthetic page lifecycle
notifications verify the handlers; they are not a claim of real-device BFCache
coverage.

Fixtures cover steady/nearby carriers, a broad voice-like signal, weak signals,
a drifting tone, single-row impulses, amplitude steps and missing intervals.
The compressed waterfall test proves an impulse survives a pixel footprint that
nearest-row sampling would miss. No replayed animation frames create history.
Numeric tests cover ownership, sequence deduplication, aggregation, ring wrap,
gaps, mapping boundaries, orientation, high zoom and preference migration.

Numerical RGB comparisons permit 3/255 per channel. Known 20 dB height steps
match within two physical pixels; measured heights were 11, 23 and 34 pixels
against 11.375, 22.75 and 34.125. Picking tolerates one geometry column.
Screenshot inspection allows two pixels of edge antialiasing and differing font
rasterization, but no displacement of frequency, time, passbands or amplitude.

## Measured performance and resource limits

Machine: Intel Core i7-13700, 24 logical CPUs; Linux
6.18.33.2-microsoft-standard-WSL2 x86_64; Google Chrome 153.0.8010.52.
WebGL used **SwiftShader software rendering**, not the machine's hardware GPU.
The visualization backing store during the soak was 914 × 468 pixels.

- The final functional run's 100 warm mode cycles averaged **2.794 ms**, maximum
  **161.6 ms**. Rebuilding an expired cache took 24.7 ms. These are synchronous
  handler timings; first visible paint and hardware/browser variability are separate.
- Auto quality reduced geometry from 1024 × 128 to 512 × 48, retaining all 4096
  source bins and the 33 ms history cadence. It periodically tried recovery with
  increasing holdoff. The 180 ten-second samples reported **median 26.0 fps**,
  range **14.95–26.67 fps**. Neither the 60 fps desktop target nor stable 30 fps
  is established on this software backend. The limitation is observable in diagnostics.
- Median sampled p95 frame interval was **83.35 ms**; p95 CPU submission time
  was approximately **0.2 ms**. CPU submission excludes asynchronous GPU work.
- Numerical CPU history stayed at **8,474,624 bytes**. Estimated 3D buffers
  ranged **22,197,148–23,045,340 bytes** as quality changed. Including the
  fixture's retained Traditional buffers gives **31,546,276–32,394,468 bytes**.
  The Traditional cache in this fixture has 1024 bins; an initialized 4096-bin
  live Traditional cache adds approximately 12 MiB to that combined estimate.
- The browser's coarse JS heap estimate stayed at 15,200,000 bytes. This is not
  a precise leak measurement or a total process/GPU-memory measurement. Allocation
  estimates exclude driver/compositor/audio overhead. No accumulating loops or
  unbounded history appeared; inactive drawing stopped and cache disposal passed.
- The run ended with 56,824 accepted updates, 632 coalesced source versions /
  missing cadence buckets and one aggregated update. Software scheduling gaps
  remained marked; they were not filled with invented measurements. Retained
  time was 16.863 seconds. Performance geometry showed 1.551 recent seconds;
  Balanced showed 4.191 seconds at this cadence.

## Remaining hardware acceptance

Not run: live-radio frequency/calibration/audio continuity, actual GPU 60/30 fps,
physical phone/tablet interaction, Safari/iOS, real cross-monitor DPR movement,
or real hardware TX-interlock operation. Chromium viewport emulation is not
Safari/iOS testing. The existing mobile TX tab remains accessible; the full TX
panel follows the existing scrolling layout. Exact resemblance to the absent
reference image remains an operator review item. These limitations are not
inferred away from fixture screenshots or unit tests.
