# Saturn Remote Web — Extraction Handoff

This directory is the TypeScript extraction of the monolithic
`templates/saturn-remote-next.html` browser code. The Vite build emits a single
IIFE bundle (`dist/saturn-remote-next.js`) loaded by `/remote-next` via a plain
`<script>` tag. The inline HTML delegates to the bundle where available, with
fallback to the original inline code when not.

## Current State (2026-04-26)

### What's Extracted

| Module | Contents | Tests |
|--------|----------|-------|
| `src/tci/parser.ts` | TCI text message parser | 4 |
| `src/tci/apply.ts` | TCI state applicator | — |
| `src/tci/commands.ts` | TCI command string builders (filter, ANR, ANF, two-tone, full radio prefs) | 9 |
| `src/radio/frequency.ts` | Frequency formatting, digit stepping, `formatFrequencyMarkup` | 5 |
| `src/radio/passband.ts` | Passband conversion, default RX/TX passbands per mode, filter cuts | 9 |
| `src/radio/band.ts` | Band lookup, IQ sample rate capping, `HAM_BAND_EDGES_HZ` | 5 |
| `src/radio/dsp-presets.ts` | ANR/ANF presets (thetis/wide, default/sharp) | 4 |
| `src/transport/rx-frame.ts` | Binary frame decode (IQ + audio) | 3 |
| `src/transport/tci-frame.ts` | 64-byte binary TCI frame header parser | 2 |
| `src/transport/rx-apply.ts` | IQ/audio frame state application | — |
| `src/audio/constants.ts` | Audio pipeline constants (frame size, ring, worklet) | 1 |
| `src/audio/resample.ts` | `resampleChannel`, `prepareAudioForPlayback`, `ringBufferFreeFrames` | 9 |
| `src/dsp/fft.ts` | `FftProcessor` class (windowed FFT with smoothing) | 5 |
| `src/settings/types.ts` | All type definitions (RadioPrefs, DisplayPrefs, BandMemory, etc.) | — |
| `src/settings/normalize.ts` | All clamp/normalize functions, band memory normalizer | — |
| `src/settings/defaults.ts` | Default settings state factory | — |
| `src/controller/create-controller.ts` | Remote controller factory | 4 |
| `src/runtime/session.ts` | Session runtime | 3 |
| `src/runtime/storage.ts` | Storage runtime | 2 |

**Bundle size**: 31.26 KB (9.34 KB gzip)
**Tests**: 88 passing across 17 test files

### What's Still Inline

These remain in `saturn-remote-next.html` and are **not** good extraction candidates
without a full framework migration (Phase 3):

- **WebGL Spectrum/Waterfall Renderers** (~600 lines) — tightly coupled to
  canvas/WebGL2 context. The `FftProcessor` math is extracted; rendering stays.
- **AudioWorklet + AudioContext setup** (~400 lines) — DOM APIs, worklet
  registration, SAB ring buffer writes. Pure `resampleChannel` is extracted.
- **DOM event binding** (~1000 lines) — button clicks, slider changes, keyboard
  shortcuts. These delegate to bundle functions where possible.
- **WebSocket management** — `sendTci`, connect/disconnect, binary frame dispatch.
- **Inline clamp/normalize duplicates** — ~40 functions that already exist in the
  bundle but are still present inline as fallbacks. Safe to remove once bundle
  loading is guaranteed.

### What's Working on /remote-next

- TCI connect, IQ display (spectrum + waterfall), RX audio (worklet + postMessage)
- All 9 demod modes: USB, LSB, AM, SAM, FM, DIGU, DIGL, CWU, CWL
- Band memory: saves/restores frequency, radio prefs, display prefs, zoom per band
- Band recall: 160m through 6m (10 bands)
- Panadapter drag tuning with 80ms throttle and 10Hz snap
- Profile persistence (localStorage + bridge server)
- MOX / PTT (TX relay works; disengage timing is a bridge-side issue)
- Performance: audio frame 2048, worklet queue cap 8, overflow drop 250ms,
  MOX recovery drop 180ms

### Bug Fixes (2026-04-27)

- **Inline duplicates removed** — 36 clamp/normalize functions replaced with single
  `const { ... } = globalThis.SaturnRemoteNext` destructuring block (~130 lines removed)
- **Filter sliders fixed** — bundle `clampFilterLowHz`/`clampFilterHighHz` used
  `Number.isFinite(value)` on string inputs (returns false); fixed to `Number(value)` first
- **TX drive unclamped** — bundle had `Math.min(10, ...)` instead of `Math.min(100, ...)`
- **TX power target bridge cap** — `SATURN_REMOTE_TX_MAX_WATTS=100` added to systemd service; bridge maps 1-100 W targets to a conservative calibrated P2 drive byte
- **Slider performance** — filter, volume, TX drive, mic gain sliders now use lightweight
  readout-only updates during drag instead of full `updateUi()` (50+ DOM elements)
- **Panadapter drag tuning** — uses `updateTuneVisuals()` (4 DOM updates) instead of
  full `updateUi()` on every pointermove
- **Reconnect frequency** — VFO/DDS frequency re-sent after bridge restart (was lost)
- **TCI flood coalescing** — `handleTciText` now coalesces `updateUi()` via
  `requestAnimationFrame` instead of calling it per-message during connect flood

### Known Issues

- **MOX slow to disengage** — bridge-side TX state machine timing, not UI
- **Slow RX recovery after MOX** — WDSP AGC + DSP settings (NR2 100%, NB1, SLOW AGC)
- **ADC2 receives nothing** — same behavior on `/remote`, likely bridge/hardware issue
- **Band switch latency** — improved (batched TCI commands, no double-remember),
  but the bridge still processes 30+ commands sequentially

## Architecture

```
Browser                          Bridge (Rust)
┌──────────────────────┐         ┌────────────────┐
│ saturn-remote-next   │  TCI    │ saturn-bridge   │
│   .html              │◄──WSS──►│   tci.rs        │
│                      │  text+  │                 │
│ saturn-remote-next   │  binary │                 │
│   .js (Vite bundle)  │         └────────────────┘
└──────────────────────┘
        │
        ▼ globalThis.SaturnRemoteNext
   Pure TS functions called by inline JS
```

The HTML loads the bundle via `<script src="/remote-assets/remote-next.js">`.
Inline JS destructures bundle functions at load time via
`const { clampXxx, ... } = globalThis.SaturnRemoteNext`. No fallbacks — bundle is required.

## Commands

```bash
npm install          # install dependencies
npm test             # run Vitest tests
npm run typecheck    # tsc --noEmit
npm run build        # vite build → dist/saturn-remote-next.js
npm run validate:remote-next-layout  # headless Chromium state-strip layout check
```

`validate:remote-next-layout` uses the current template/CSS with runtime scripts
stripped, applies representative operator state-strip values, captures phone
portrait, phone portrait drawer, phone landscape, phone landscape drawer,
tablet, tablet drawer, laptop, and laptop drawer screenshots under
`/tmp/saturn-remote-next-layout`, and fails on state-strip pill overlap,
container overflow, drawer sheet overflow, drawer group overlap, drawer text
overflow, or missing drawer content. It is a Chromium layout gate, not a
Safari/iOS substitute.

## Deploying

```bash
# Build
npm run build

# Deploy bundle + HTML to webroot
sudo cp dist/saturn-remote-next.js /var/lib/saturn-web/saturn-remote-next.js
sudo cp ../templates/saturn-remote-next.html /var/lib/saturn-web/saturn-remote-next.html
```

The rust-server (`saturn-go`) maps `/remote-assets/remote-next.js` →
`saturn-remote-next.js` in `remote_tls.rs`.

## Next Steps

### Phase 2 Remaining (incremental)
1. ~~**Remove inline duplicates**~~ — DONE (2026-04-27). 36 functions destructured from bundle.
2. **Remove remaining fallback guards** — `signedPassbandFromUiCuts`,
   `uiCutsFromSignedPassband`, `formatFrequencyMarkup`, `resampleChannel` still have
   inline definitions with `if (next)` guards. Safe to replace with direct bundle calls.
3. **Optimize remaining slider handlers** — several sliders still call full `updateUi()`
   on input events (NR level, NB threshold, AGC gain, EQ bands, CFC bands, zoom).
4. **Extract remaining pure helpers** — `displayPassbandHz`, `displayPercentForOffsetHz`,
   waterfall palette definitions, frequency digit tuning helpers.

### Phase 3: Single-Page Vite Build
1. Create a Vite HTML entry that replaces the monolithic template
2. Move WebGL renderers into TS modules (import WebGL utils)
3. Move AudioWorklet source into a separate TS file built by Vite
4. Move DOM event binding into TS modules
5. Retire `saturn-remote-next.html` entirely
6. Update rust-server to serve Vite's output HTML

### Promotion
Once `/remote-next` is verified equivalent to `/remote`:
1. Update deployment to serve Vite-built assets for `/remote`
2. Remove hand-crafted JS files from `templates/`
3. Retire the old monolithic `saturn-remote.html`

## File Map

```
remote-web/
├── src/
│   ├── remote-next-entry.ts    ← bundle entry, exposes globalThis.SaturnRemoteNext
│   ├── audio/
│   │   ├── constants.ts        ← RX_AUDIO_*, ring buffer constants
│   │   └── resample.ts         ← resampleChannel, prepareAudioForPlayback
│   ├── controller/
│   │   └── create-controller.ts
│   ├── dsp/
│   │   └── fft.ts              ← FftProcessor (windowed FFT + smoothing)
│   ├── radio/
│   │   ├── band.ts             ← bandKeyForFrequency, effectiveIqSampleRate
│   │   ├── dsp-presets.ts      ← anrPreset, anfPreset
│   │   ├── frequency.ts        ← formatFrequencyHz, formatFrequencyMarkup
│   │   └── passband.ts         ← passband conversion, default passbands
│   ├── runtime/
│   │   ├── session.ts
│   │   └── storage.ts
│   ├── settings/
│   │   ├── defaults.ts
│   │   ├── normalize.ts        ← all clamp/normalize functions
│   │   └── types.ts            ← RadioPrefs, DisplayPrefs, BandMemory, etc.
│   ├── tci/
│   │   ├── apply.ts
│   │   ├── commands.ts         ← buildAllRadioPrefsCommands, filter/ANR/ANF/TT
│   │   └── parser.ts
│   └── transport/
│       ├── rx-apply.ts
│       ├── rx-frame.ts
│       └── tci-frame.ts
├── tests/                       ← 17 test files, 88 tests
├── dist/                        ← gitignored, built by `npm run build`
├── vite.config.ts               ← IIFE library mode
├── tsconfig.json
└── package.json
```
