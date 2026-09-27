# Saturn Remote UI redesign: Phase 0 discovery

Source baseline: `origin/main` at `533d4ed`, inspected in the `ui-redesign-phase0` worktree. This document is a source audit, not a redesign. The owner's version 1.0 build sheet governs later phases.

## Phase 0 result and required stop

**Stop after Phase 0.** Transmit readiness and arming logic is in the inline UI script: `armTxReady`, `lockTx`, `setPtt`, and `bindPttButton` in `update_manager/templates/saturn-remote-next.html` (roughly lines 17213–18245). The Phase 3 panel move would have to rebind or adapt those controls. The current code implements pointer cancellation, pointer capture loss, window blur, visibility changes, and page hide release paths; preserving them needs an explicit safety review and test plan before moving controls. This is one of section 3's mandatory stop conditions. No Phase 1 implementation is started here.

The spectrum data pipeline is not intrinsically tied to the page grid: WebSocket frames and FFT/history objects feed renderers separately. The current canvas sizing and overlay positioning *do* read `.display-stack`, `.spectrum-shell`, and `.waterfall-shell` geometry, so moving canvases requires adapting those references. Source inspection found no CDN URL in the Remote template, but a live network-panel check remains for Phase 1.

## Framework, build, and serving

`/remote-next` is a hybrid: a 21,380-line HTML template with inline CSS and JavaScript, plus TypeScript modules bundled by Vite 5 as an IIFE. It is not React/Svelte or a fully buildless page. `src/remote-next-entry.ts` exports the `window.SaturnRemoteNext` seam consumed by the inline script. The Rust TLS listener on port 8443 serves `/remote-next`, its locally hosted `/remote-assets/remote-next.js` bundle, and `/remote-assets/inter.woff2`; the deploy script copies the HTML and bundle into the web root. The default published URL uses `transport=split&tx_opus=1&tx_cfc=1`. Other local Saturn Go pages and their shared assets are separate from this operator console.

The existing Vite build answers the Phase 0 framework question operationally: this app already has a small build step. No new framework or build tooling is necessary to implement the sheet. If a later phase proposes changing that architecture, the owner must approve that separate decision. The `remote_asset_handler` in `update_manager/rust-server/src/remote_tls.rs` explicitly allows only `remote-next.js` and `inter.woff2`; simply adding Phase 1's IBM Plex fonts, icons, or `tokens.css` under the web root will not serve them. The build sheet forbids backend changes, so a later phase must keep these assets in source files and include them in an already served HTML/bundle response, or ask the owner to revise that constraint. The deploy script also validates `terrain-canvas`, `view-3d`, and `TerrainRenderer` markers before installing, so later phases must keep those deployment checks aligned with any rename.

### Front-end file map

| Area | Files / role |
| --- | --- |
| Entry and styling | `update_manager/templates/saturn-remote-next.html`: DOM, inline CSS, inline control code, WebGL/Canvas2D traditional renderers. `update_manager/remote-web/src/remote-next-entry.ts`: bundled seam. No separate CSS source is loaded by this page today. |
| Render and DSP | `src/render/terrain.ts` (Hi-Res 3D WebGL2), `src/dsp/{fft,display,display-cleanup,peak-assist,spectrum-history}.ts`. |
| State and preferences | `src/state/{app-state,apply-prefs,prefs-from-state,perf-snapshot}.ts`, `src/settings/{defaults,normalize,terrain,types}.ts`, `src/runtime/{session,storage}.ts`. |
| Control helpers | `src/controller/{create-controller,types}.ts`, `src/radio/{band,frequency,passband,dsp-presets}.ts`, `src/ui/{operations-drawer,meter-math,display-layout,control-context,setup-navigation,responsive-layout,tx-presentation}.ts`. |
| TCI and transport | `src/tci/{commands,apply,parser,state}.ts`, `src/transport/{tci-frame,rx-frame,rx-state,rx-apply,reconnect-supervisor,legacy-socket-adapter,split-sockets,spectrum-row,transport-mode,tx-uplink}.ts`. |
| Audio | `src/audio/{constants,scope,satp,resample,rx-telemetry,rx-profile,rx-opus,tx-audio-profile,tx-opus-encoder}.ts`. |
| Build and tests | `package.json`, `package-lock.json`, `vite.config.ts`, `tsconfig.json`, `tests/*.test.ts`, `scripts/*.mjs`, `deploy-remote-next.sh`. |
| Adjacent assets | The console uses a local Inter WOFF2 today. `update_manager/templates/assets/` contains shared Saturn Go CSS, JS, fonts, and vendored libraries for other pages; these are not CDN requests from `/remote-next`. |

## State and WebSocket updates

The inline script creates `state = SaturnRemoteNext.createAppState()` near line 9106 and keeps some renderer, media, and settings objects alongside it. The WebSocket message callback near line 18453 routes text to `handleTciText` and binary to `handleBinaryFrame`. Text messages pass through `SaturnRemoteNext.applyTciText(text, state)` and UI side effects, then schedule an `updateUi` refresh. Binary frame types are dispatched by the existing 64-byte TCI header: RX IQ type 0, TX IQ type 3, RX audio (PCM/Opus), and server spectrum row type 16. The renderer uses the same state that the current labels and controls use. Phase 3 must preserve the message handlers as the source of truth, including server echo reconciliation and TX ownership/lock state.

Outbound controls funnel through existing handlers and `sendTci` near line 16078, or through local-only UI state/persistence. The inventory below groups handlers by their current binding function and calls out the state/message family. No new TX path is authorized by the redesign.

## Spectrum and waterfall pipeline

LAN display consumes interleaved complex float32 IQ frames. `buildRenderIqWindow` and `FftProcessor` use a Hann window and FFT; magnitudes become relative dB with `20 * log10(magnitude + 1e-8)`. Base target FFT is 4096 bins; WAN targets 2048 and phone WAN 1024, with adaptive sizes up to 262144 for high resolution. IQ sample rate comes from the stream header/state and can be 192 or 384 kHz in current operating modes. There is no stable fixed incoming frame rate: the animation loop gates on new data and a rendering interval.

For WAN-like conditions the negotiated server spectrum row bypasses raw IQ copy and the browser FFT. Its type-16 frame carries a 64-byte header, 256–4096 quantized bins, and an interval clamped to 33–250 ms (about 4–30 rows/s). Dequantization uses `dbOffset + byte * dbStep`, yielding the same relative dB domain. Both paths enter `drawDisplayBins` (around line 19313), which updates `SpectrumHistory`, averaging/range logic, and display renderers. `animationLoop` uses `requestAnimationFrame`, advances only for fresh frame versions, and invokes terrain rendering.

Traditional mode uses inline `SpectrumRenderer` and `WaterfallRenderer` classes. Each attempts WebGL2 then Canvas 2D fallback. Hi-Res 3D uses `TerrainRenderer` from `src/render/terrain.ts`, requires WebGL2, and can fall back to Traditional mode on failure. Three current canvases are `#spectrum-canvas`, `#waterfall-canvas`, and `#terrain-canvas`. `setTerrainMode` switches views; `layoutTerrainCanvas` sizes the terrain canvas from display wrapper rectangles. Existing display settings and the current terrain/classic palette must remain available.

## Query parameters and compatibility

The inline boot code reads `transport` (plus `phase42_split`), `tx_opus` (plus `phase44_tx_opus`), `tx_cfc` (plus `phase44_tx_cfc`), `tx_diagnostic`, `tx_source`, `tx_level_db`, `display_profile`, `rx_audio_adaptive`, `tx_noise_gate`, `tx_noise_gate_db`, `tx_cfc_precomp_db`, `tx_voice_eq`, `rx_audio_codec`, `force_legacy_ws`, and `force_same_origin_ws`. The `phase40_*` aliases for diagnostic/source/level/display profile and `phase44_*` aliases for TX Opus/EQ/gate/CFC are canonicalized during boot; `client_bust` is removed as cosmetic URL cleanup. Section 9's table omits this query-surface inventory; Phase 3 parity tests must keep each meaning.

## Persistence and theme

| Key/family | Current value or purpose |
| --- | --- |
| `saturn.remote.settings` | JSON settings snapshot, also synchronized to remote settings/profile endpoints. |
| `saturn.remote.theme` | Explicit `dark`/`light`; absent key follows `prefers-color-scheme`. The current button alternates dark/light; Auto is an implicit default, not a selectable mode yet. `applyTheme` sets `document.documentElement.dataset.theme` and `colorScheme`. |
| `saturn.remote.layout`, `phonePanels`, `phoneSpectrumMode`, `phoneWaterfall`, `spectrumWaterfallRatio`, `operationsDrawer`, `setupPanel`, `controlContext` | Layout, phone panel openness, display split/view, drawer and setup positions. |
| `saturn.remote.displayPrefs`, `displayZoom`, `instrumentMeterMode` | Display styling, zoom, and meter presentation. |
| `saturn.remote.radioPrefs`, `bandMemory`, `vfoTuneStepHz`, `freqLock`, `keepScreenAwake` | Radio/UI preferences and tuning behavior. Band memory includes radio state and must not be cleared by UI restore-defaults. |
| `saturn.remote.wsUrl`, `streamMode`, `activeProfile`, `audioInputDeviceId`, `audioOutputDeviceId` | Connection, profile, and device choices. |
| `saturn.remote.txOpus`, `saturn.remote.splitTransport` | Transport overrides; legacy `saturn.phase44.txOpus` and `saturn.phase42.splitTransport` remain accepted. |

Current preference reads/writes are partly wrapped in `try/catch`, partly direct `localStorage` calls. Phase 1/3 persistence work must preserve old keys while adding the requested `saturn.ui.*` namespace and storage-unavailable fallback.
The bridge's remote settings/profile API also persists these preferences through `GET/POST /remote_settings` and profile endpoints. Its Rust `RemoteSettings` and `RemoteDisplayPrefs` types include flattened extra fields, so key migration must preserve server-stored values as well as browser storage.

The current app has 16 `keydown` listeners across tuning, menus, shortcuts, frequency entry, and TX. Space-bar hold-to-key already exists; the first press while locked only arms. Those bindings need per-control migration checks. The one native `window.confirm` is for deleting a remote profile; other safety prompts use page UI or existing safeguards.

## Tests and validation

From `update_manager/remote-web`: `npm test` runs Vitest unit/template tests; `npm run typecheck` runs TypeScript without emit; `npm run build` builds the IIFE; `npm run check:seam` checks template/bundle exports; `npm run validate:remote-next-layout`, `validate:waterfall`, `validate:terrain`, and `validate:wan-display` exercise layout and display contracts. The suite includes `tx-safety-template.test.ts`, `operator-controls-template.test.ts`, `terrain.test.ts`, `spectrum-row.test.ts`, and `remote-next-smoke.test.ts`. Browser/device QA in the sheet remains separate; source tests alone do not prove those criteria.

## Control inventory key

The static DOM inventory below is generated from all `button`, `input`, `select`, `summary`, and `a` elements in the Remote template (274 elements). Each row gives its current source location. The handler column names the binding area; controls in one area often call a shared function before sending TCI. Read-only labels and status indicators are listed afterward. A row with no ID is identified by its text or position in the named group. Dynamic controls are listed separately. This is the baseline for `migration.md` in Phase 3; section 9 omissions follow the inventory.

| Handler area | Current binding and state/message family |
| --- | --- |
| Header / session | `initInteractions`, `bindOperatorStateDetail`, `bindShortcutHelp`, `bindKeyboardTuneControls`; connection, layout/theme, VFO, band/mode, TX/RX ownership. VFO/radio actions use existing TCI command helpers. |
| Setup profiles | `initInteractions` and setup navigation; saved profile/settings endpoints, active/startup profile and UI state. |
| Display setup | `initInteractions`, `applyDisplayPrefsObject`, `bindTerrainControls`; local/remote display preferences and renderers, no radio message except tuning/peak actions. |
| DSP setup | `initInteractions` and generated EQ handlers; RX NR/ANF/EQ settings and existing DSP TCI messages. |
| TX setup | `initInteractions`, `setTwoToneEnabled`, `setPtt`; TX DSP/gate/filter/two-tone/PureSignal/CFC settings and existing TX TCI messages. Treat as safety-sensitive. |
| Network / audio / advanced | `initInteractions`; stream mode, device selection, WebSocket URL, sample-rate request and diagnostics. |
| Meter | `initInteractions`, `initMultimeter`; meter selection/average/peak and TX tool shortcuts, mostly local UI state. |
| Receive | `initInteractions`; audio start/stop, RX volume/squelch/AGC/NR/NB/filter and their existing TCI commands. |
| Radio path / band / mode | `initInteractions`, `setRxAttenuation`, band memory handlers; RX ADC/antenna/ATT, band recall and demod TCI commands. |
| Display toolbar / passband | `initInteractions`, `bindPassbandDragControls`, `setTerrainMode`, `bindDisplayWorkspaceResizer`; display prefs, tuning, RX filter TCI commands. |
| Transmit | `bindPttButton`, `setPtt`, `initInteractions`; TX arm/PTT/MOX/lock safety plus source, MON, drive, mic and processing TCI commands. |
| Operations / phone / frequency | `updateOperationsDrawer`, `initPhonePanels`, `initControlContextRail`, `bindFrequencyEntrySheet`, `bindFrequencyDigitControl`; panel state or delegates to existing control handlers. |
| Details / terrain | `bindOperatorStateDetail`, `bindShortcutHelp`, `bindTerrainControls`; detail copy/help and display preference/render state. |

### Static interactive elements

| Line | Current DOM location | Control | Handler area | State / existing message |
| ---: | --- | --- | --- | --- |
| 6941 | Header / System | `summary` — System | Header / session | Connection and UI preferences |
| 6944 | Header / System | `#static-wsdiag-btn` — WS Probe | Header / session | Connection and UI preferences |
| 6945 | Header / System | `#static-wsdiag-copy-btn` — Copy Log | Header / session | Connection and UI preferences |
| 6946 | Header / System | `#saturn-manage-home` — Saturn Go | Header / session | Connection and UI preferences |
| 6947 | Header / System | `#saturn-manage-monitor` — Monitor | Header / session | Connection and UI preferences |
| 6950 | Header / System | `#shortcut-help-btn` — ? | Header / session | Connection and UI preferences |
| 6951 | Header / System | `#layout-btn` — Phone | Header / session | Connection and UI preferences |
| 6952 | Header / System | `#theme-btn` — Theme: Dark | Header / session | Connection and UI preferences |
| 6953 | Header / System | `#header-setup-btn` — ⚙ | Header / session | Connection and UI preferences |
| 7038 | Session / VFO | `#rx-att-quick-btn` — ATT Off | Header / session | Tuning, RX path, connection |
| 7050 | Session / VFO | `#vfo-a-select-btn` — A RX | Header / session | Tuning, RX path, connection |
| 7051 | Session / VFO | `#vfo-b-select-btn` — B | Header / session | Tuning, RX path, connection |
| 7052 | Session / VFO | `#split-toggle-btn` — SPLIT OFF | Header / session | Tuning, RX path, connection |
| 7063 | Session / VFO | `#go-live-btn` — Go Live | Header / session | Tuning, RX path, connection |
| 7065 | Session / VFO | `#setup-menu-btn` — Setup | Header / session | Tuning, RX path, connection |
| 7073 | Setup / Profiles | `#setup-close-btn` — Close | Setup profiles | Profile/settings API, active profile |
| 7077 | Setup / Profiles | `#setup-tab-profiles` — Profiles | Setup profiles | Profile/settings API, active profile |
| 7078 | Setup / Profiles | `#setup-tab-display` — Display | Setup profiles | Profile/settings API, active profile |
| 7079 | Setup / Profiles | `#setup-tab-dsp` — DSP | Setup profiles | Profile/settings API, active profile |
| 7080 | Setup / Profiles | `#setup-tab-tx` — Transmit | Setup profiles | Profile/settings API, active profile |
| 7081 | Setup / Profiles | `#setup-tab-network` — Network | Setup profiles | Profile/settings API, active profile |
| 7082 | Setup / Profiles | `#setup-tab-audio` — Audio | Setup profiles | Profile/settings API, active profile |
| 7083 | Setup / Profiles | `#setup-tab-advanced` — Advanced | Setup profiles | Profile/settings API, active profile |
| 7091 | Setup / Profiles | `#setup-profile-select` | Setup profiles | Profile/settings API, active profile |
| 7095 | Setup / Profiles | `#setup-profile-load-btn` — Load | Setup profiles | Profile/settings API, active profile |
| 7096 | Setup / Profiles | `#setup-profile-save-btn` — Save | Setup profiles | Profile/settings API, active profile |
| 7097 | Setup / Profiles | `#setup-profile-save-as-btn` — Save As | Setup profiles | Profile/settings API, active profile |
| 7100 | Setup / Profiles | `#setup-profile-delete-btn` — Delete | Setup profiles | Profile/settings API, active profile |
| 7104 | Setup / Profiles | `#setup-startup-profile-select` | Setup profiles | Profile/settings API, active profile |
| 7106 | Setup / Profiles | `#setup-startup-apply-btn` — Set Startup | Setup profiles | Profile/settings API, active profile |
| 7107 | Setup / Profiles | `#setup-startup-clear-btn` — Clear Startup | Setup profiles | Profile/settings API, active profile |
| 7117 | Setup / Display | `#display-transceiver-clarity` — Transceiver clarity | Display setup | Display preferences/rendering |
| 7118 | Setup / Display | `#display-appearance-undo` — Undo appearance | Display setup | Display preferences/rendering |
| 7122 | Setup / Display | `#display-spectrum-auto-range` (`checkbox`) | Display setup | Display preferences/rendering |
| 7123 | Setup / Display | `#display-waterfall-auto-range` (`checkbox`) | Display setup | Display preferences/rendering |
| 7128 | Setup / Display | `#display-spectrum-floor` (`range`) | Display setup | Display preferences/rendering |
| 7133 | Setup / Display | `#display-spectrum-ceiling` (`range`) | Display setup | Display preferences/rendering |
| 7138 | Setup / Display | `#display-waterfall-floor` (`range`) | Display setup | Display preferences/rendering |
| 7143 | Setup / Display | `#display-waterfall-ceiling` (`range`) | Display setup | Display preferences/rendering |
| 7148 | Setup / Display | `#display-spectrum-average` (`range`) | Display setup | Display preferences/rendering |
| 7153 | Setup / Display | `#display-spectrum-trace-color` (`color`) | Display setup | Display preferences/rendering |
| 7158 | Setup / Display | `#display-spectrum-trace-smoothing` (`range`) | Display setup | Display preferences/rendering |
| 7163 | Setup / Display | `#display-spectrum-trace-fill` (`range`) | Display setup | Display preferences/rendering |
| 7168 | Setup / Display | `#display-spectrum-peak-glow` (`range`) | Display setup | Display preferences/rendering |
| 7173 | Setup / Display | `#display-spectrum-glass-sheen` (`range`) | Display setup | Display preferences/rendering |
| 7178 | Setup / Display | `#display-waterfall-speed` (`range`) | Display setup | Display preferences/rendering |
| 7183 | Setup / Display | `#display-waterfall-contrast` (`range`) | Display setup | Display preferences/rendering |
| 7188 | Setup / Display | `#display-waterfall-smoothing` — Display-only smoothing of small level fluctuations. Preserves steady-level colors; ch (`range`) | Display setup | Display preferences/rendering |
| 7196 | Setup / Display | `#display-waterfall-palette` — Classic  Reference Blue / Rainbow Reference Dark  Enhanced (Thetis-style)  Ember  Ice | Display setup | Display preferences/rendering |
| 7205 | Setup / Display | `#display-spectrum-peak-hold` (`checkbox`) | Display setup | Display preferences/rendering |
| 7206 | Setup / Display | `#display-spectrum-enhanced-colors` (`checkbox`) | Display setup | Display preferences/rendering |
| 7207 | Setup / Display | `#display-peak-tune-assist` (`checkbox`) | Display setup | Display preferences/rendering |
| 7208 | Setup / Display | `#display-show-grid` (`checkbox`) | Display setup | Display preferences/rendering |
| 7209 | Setup / Display | `#display-show-center-line` (`checkbox`) | Display setup | Display preferences/rendering |
| 7210 | Setup / Display | `#display-show-band-edges` (`checkbox`) | Display setup | Display preferences/rendering |
| 7219 | Setup / DSP | `#setup-dsp-tab-nr` — NR | DSP setup | RX DSP TCI/settings |
| 7220 | Setup / DSP | `#setup-dsp-tab-anf` — ANF | DSP setup | RX DSP TCI/settings |
| 7221 | Setup / DSP | `#setup-dsp-tab-eq` — RX EQ | DSP setup | RX DSP TCI/settings |
| 7227 | Setup / DSP | `#setup-dsp-nr-mode` — Off  NR1 / LMS ANR  NR2 / EMNR  NR3 / RNNR  NR4 / SBNR | DSP setup | RX DSP TCI/settings |
| 7238 | Setup / DSP | `#setup-dsp-nr-level` (`range`) | DSP setup | RX DSP TCI/settings |
| 7244 | Setup / DSP | `#setup-dsp-nr2-gain-method` — Gaussian  Gaussian Log  Gamma  Trained | DSP setup | RX DSP TCI/settings |
| 7253 | Setup / DSP | `#setup-dsp-nr2-npe-method` — OSMS  MMSE  NSTAT | DSP setup | RX DSP TCI/settings |
| 7262 | Setup / DSP | `#setup-dsp-nr2-post-filter` (`checkbox`) | DSP setup | RX DSP TCI/settings |
| 7267 | Setup / DSP | `#setup-dsp-anr-taps` (`number`) | DSP setup | RX DSP TCI/settings |
| 7271 | Setup / DSP | `#setup-dsp-anr-delay` (`number`) | DSP setup | RX DSP TCI/settings |
| 7275 | Setup / DSP | `#setup-dsp-anr-gain` (`number`) | DSP setup | RX DSP TCI/settings |
| 7279 | Setup / DSP | `#setup-dsp-anr-leakage` (`number`) | DSP setup | RX DSP TCI/settings |
| 7283 | Setup / DSP | `#setup-dsp-anr-thetis-btn` — Thetis LMS Default | DSP setup | RX DSP TCI/settings |
| 7284 | Setup / DSP | `#setup-dsp-anr-wide-btn` — Wideband ANR | DSP setup | RX DSP TCI/settings |
| 7289 | Setup / DSP | `#setup-dsp-anf-enabled` (`checkbox`) | DSP setup | RX DSP TCI/settings |
| 7293 | Setup / DSP | `#setup-dsp-anf-taps` (`number`) | DSP setup | RX DSP TCI/settings |
| 7297 | Setup / DSP | `#setup-dsp-anf-delay` (`number`) | DSP setup | RX DSP TCI/settings |
| 7301 | Setup / DSP | `#setup-dsp-anf-gain` (`number`) | DSP setup | RX DSP TCI/settings |
| 7305 | Setup / DSP | `#setup-dsp-anf-leakage` (`number`) | DSP setup | RX DSP TCI/settings |
| 7309 | Setup / DSP | `#setup-dsp-anf-thetis-btn` — Thetis ANF Default | DSP setup | RX DSP TCI/settings |
| 7310 | Setup / DSP | `#setup-dsp-anf-sharp-btn` — Sharp Carrier Hunt | DSP setup | RX DSP TCI/settings |
| 7318 | Setup / DSP | `#rx-eq-enable-btn` — EQ Off | DSP setup | RX DSP TCI/settings |
| 7330 | Setup / Transmit | `#setup-tx-timeout-enabled` (`checkbox`) | TX setup | TX DSP/TCI and safety gates |
| 7335 | Setup / Transmit | `#setup-tx-timeout-seconds` (`number`) | TX setup | TX DSP/TCI and safety gates |
| 7344 | Setup / Transmit | `#setup-tx-phase-rotator-enabled` (`checkbox`) | TX setup | TX DSP/TCI and safety gates |
| 7348 | Setup / Transmit | `#setup-tx-phase-rotator-auto` (`checkbox`) | TX setup | TX DSP/TCI and safety gates |
| 7352 | Setup / Transmit | `#setup-tx-phase-rotator-corner` (`range`) | TX setup | TX DSP/TCI and safety gates |
| 7359 | Setup / Transmit | `#two-tone-btn` — 2-Tone Test Off | TX setup | TX DSP/TCI and safety gates |
| 7364 | Setup / Transmit | `#two-tone-freq1` (`range`) | TX setup | TX DSP/TCI and safety gates |
| 7369 | Setup / Transmit | `#two-tone-freq2` (`range`) | TX setup | TX DSP/TCI and safety gates |
| 7374 | Setup / Transmit | `#two-tone-level` (`range`) | TX setup | TX DSP/TCI and safety gates |
| 7379 | Setup / Transmit | `#two-tone-delay` (`range`) | TX setup | TX DSP/TCI and safety gates |
| 7383 | Setup / Transmit | `#two-tone-invert-lsb` (`checkbox`) | TX setup | TX DSP/TCI and safety gates |
| 7388 | Setup / Transmit | `#setup-tx-puresignal-enabled` (`checkbox`) | TX setup | TX DSP/TCI and safety gates |
| 7389 | Setup / Transmit | `#setup-tx-puresignal-auto-attenuate` (`checkbox`) | TX setup | TX DSP/TCI and safety gates |
| 7393 | Setup / Transmit | `#setup-tx-puresignal-attenuation` (`range`) | TX setup | TX DSP/TCI and safety gates |
| 7397 | Setup / Transmit | `#setup-tx-puresignal-reset` — Reset Calibration | TX setup | TX DSP/TCI and safety gates |
| 7423 | Setup / Transmit | `#tx-eq-enable-btn` — EQ Off | TX setup | TX DSP/TCI and safety gates |
| 7433 | Setup / Transmit | `#cfc-enable-btn` — CFC Off | TX setup | TX DSP/TCI and safety gates |
| 7437 | Setup / Transmit | `#cfc-precomp` (`range`) | TX setup | TX DSP/TCI and safety gates |
| 7449 | Setup / Network | `#setup-stream-mode` — LAN — 48 kHz stereo audio  WAN / VPN — 12 kHz mono audio | Network / audio / advanced | Stream mode/diagnostics |
| 7463 | Setup / Network | `#setup-network-copy-diagnostics-btn` — Copy Network Diagnostics | Network / audio / advanced | Stream mode/diagnostics |
| 7478 | Setup / Audio | `#setup-audio-open-rx-btn` — Open RX Controls | Network / audio / advanced | Audio devices and panel state |
| 7479 | Setup / Audio | `#setup-audio-open-tx-btn` — Open TX Controls | Network / audio / advanced | Audio devices and panel state |
| 7486 | Setup / Audio | `#setup-audio-input-select` — System Default | Network / audio / advanced | Audio devices and panel state |
| 7491 | Setup / Audio | `#setup-audio-output-select` — System Default | Network / audio / advanced | Audio devices and panel state |
| 7495 | Setup / Audio | `#setup-audio-refresh-devices-btn` — Refresh Device List | Network / audio / advanced | Audio devices and panel state |
| 7505 | Setup / Advanced | `#ws-url` | Network / audio / advanced | WebSocket URL/sample rate |
| 7510 | Setup / Advanced | `#sample-rate` — 48 kHz  96 kHz  192 kHz  384 kHz | Network / audio / advanced | WebSocket URL/sample rate |
| 7574 | Meter / tools | `#instrument-meter-mode` — Signal strength  Forward power  Reflected power  SWR  ALC  Compression  Microphone le | Meter | Meter and TX tools UI state |
| 7591 | Meter / tools | `#instrument-meter-average-btn` — AVG | Meter | Meter and TX tools UI state |
| 7592 | Meter / tools | `#instrument-meter-peak-btn` — PEAK | Meter | Meter and TX tools UI state |
| 7612 | Meter / tools | `summary` — More meter details & TX tools | Meter | Meter and TX tools UI state |
| 7621 | Meter / tools | `#tx-tool-select` — TX Meters  Mic Gain  Gate  TX EQ  CFC  Leveler | Meter | Meter and TX tools UI state |
| 7629 | Meter / tools | `#tx-tool-open-btn` — Open | Meter | Meter and TX tools UI state |
| 7646 | Meter / tools | `#tx-meter-mode` — Peak  Avg | Meter | Meter and TX tools UI state |
| 7668 | Receive / DSP | `#rx-audio-toggle-btn` — Start Audio | Receive | RX audio/DSP/filter TCI |
| 7678 | Receive / DSP | `#rx-volume` (`range`) | Receive | RX audio/DSP/filter TCI |
| 7701 | Receive / DSP | `#rx-ssql-btn` — SQL Off | Receive | RX audio/DSP/filter TCI |
| 7704 | Receive / DSP | `#rx-ssql-threshold` — Squelch threshold (`range`) | Receive | RX audio/DSP/filter TCI |
| 7710 | Receive / DSP | `#rx-anf-btn` — ANF Off | Receive | RX audio/DSP/filter TCI |
| 7713 | Receive / DSP | `button` — Off | Receive | RX audio/DSP/filter TCI |
| 7714 | Receive / DSP | `button` — Long | Receive | RX audio/DSP/filter TCI |
| 7715 | Receive / DSP | `button` — Slow | Receive | RX audio/DSP/filter TCI |
| 7716 | Receive / DSP | `button` — Med | Receive | RX audio/DSP/filter TCI |
| 7717 | Receive / DSP | `button` — Fast | Receive | RX audio/DSP/filter TCI |
| 7723 | Receive / DSP | `#agc-gain` (`range`) | Receive | RX audio/DSP/filter TCI |
| 7732 | Receive / DSP | `button` — Off | Receive | RX audio/DSP/filter TCI |
| 7733 | Receive / DSP | `button` — NR1 | Receive | RX audio/DSP/filter TCI |
| 7734 | Receive / DSP | `button` — NR2 | Receive | RX audio/DSP/filter TCI |
| 7735 | Receive / DSP | `button` — NR3 | Receive | RX audio/DSP/filter TCI |
| 7736 | Receive / DSP | `button` — NR4 | Receive | RX audio/DSP/filter TCI |
| 7738 | Receive / DSP | `#rx-nr-level` (`range`) | Receive | RX audio/DSP/filter TCI |
| 7744 | Receive / DSP | `button` — Off | Receive | RX audio/DSP/filter TCI |
| 7745 | Receive / DSP | `button` — NB1 | Receive | RX audio/DSP/filter TCI |
| 7746 | Receive / DSP | `button` — NB2 | Receive | RX audio/DSP/filter TCI |
| 7747 | Receive / DSP | `button` — NB3 | Receive | RX audio/DSP/filter TCI |
| 7749 | Receive / DSP | `#rx-nb-threshold` (`range`) | Receive | RX audio/DSP/filter TCI |
| 7761 | Receive / DSP | `#filter-low` (`range`) | Receive | RX audio/DSP/filter TCI |
| 7766 | Receive / DSP | `#filter-high` (`range`) | Receive | RX audio/DSP/filter TCI |
| 7784 | Radio path | `#rx-adc` — ADC1 | Radio path / band / mode | ADC/antenna/ATT TCI |
| 7790 | Radio path | `#rx-antenna` — ANT1  ANT2  ANT3 | Radio path / band / mode | ADC/antenna/ATT TCI |
| 7798 | Radio path | `#rx-attenuation` — ATT Off  10 dB  20 dB  30 dB | Radio path / band / mode | ADC/antenna/ATT TCI |
| 7819 | Band / mode | `button` — 160m | Radio path / band / mode | Band recall/demod TCI |
| 7820 | Band / mode | `button` — 80m | Radio path / band / mode | Band recall/demod TCI |
| 7821 | Band / mode | `button` — 60m | Radio path / band / mode | Band recall/demod TCI |
| 7822 | Band / mode | `button` — 40m | Radio path / band / mode | Band recall/demod TCI |
| 7823 | Band / mode | `button` — 30m | Radio path / band / mode | Band recall/demod TCI |
| 7824 | Band / mode | `button` — 20m | Radio path / band / mode | Band recall/demod TCI |
| 7825 | Band / mode | `button` — 17m | Radio path / band / mode | Band recall/demod TCI |
| 7826 | Band / mode | `button` — 15m | Radio path / band / mode | Band recall/demod TCI |
| 7827 | Band / mode | `button` — 12m | Radio path / band / mode | Band recall/demod TCI |
| 7828 | Band / mode | `button` — 10m | Radio path / band / mode | Band recall/demod TCI |
| 7829 | Band / mode | `button` — 6m | Radio path / band / mode | Band recall/demod TCI |
| 7830 | Band / mode | `button` — FM | Radio path / band / mode | Band recall/demod TCI |
| 7846 | Band / mode | `button` — USB | Radio path / band / mode | Band recall/demod TCI |
| 7847 | Band / mode | `button` — LSB | Radio path / band / mode | Band recall/demod TCI |
| 7848 | Band / mode | `button` — AM | Radio path / band / mode | Band recall/demod TCI |
| 7849 | Band / mode | `button` — SAM | Radio path / band / mode | Band recall/demod TCI |
| 7850 | Band / mode | `button` — FM | Radio path / band / mode | Band recall/demod TCI |
| 7851 | Band / mode | `#wfm-mode-btn` — WFM | Radio path / band / mode | Band recall/demod TCI |
| 7852 | Band / mode | `button` — DIGU | Radio path / band / mode | Band recall/demod TCI |
| 7853 | Band / mode | `button` — DIGL | Radio path / band / mode | Band recall/demod TCI |
| 7854 | Band / mode | `button` — CWU | Radio path / band / mode | Band recall/demod TCI |
| 7855 | Band / mode | `button` — CWL | Radio path / band / mode | Band recall/demod TCI |
| 7864 | Band / mode | `#wbfm-deemphasis` — North America 75 us  Europe 50 us  Off | Radio path / band / mode | Band recall/demod TCI |
| 7887 | Display toolbar/well | `#spectrum-average-toolbar-btn` — AVG 1 | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7888 | Display toolbar/well | `#spectrum-peak-toolbar-btn` — PEAK OFF | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7889 | Display toolbar/well | `#spectrum-tune-peak-btn` — Tune Peak | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7891 | Display toolbar/well | `#display-center-btn` — Center | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7892 | Display toolbar/well | `#wake-lock-btn` — Stay Awake | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7893 | Display toolbar/well | `#freq-lock-btn` — Lock | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7894 | Display toolbar/well | `#waterfall-toggle-btn` — View Both | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7898 | Display toolbar/well | `#view-traditional` — Traditional | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7899 | Display toolbar/well | `#view-3d` — High-Res 3D | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7900 | Display toolbar/well | `#terrain-settings-open` — Display Settings | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7901 | Display toolbar/well | `#display-pause` — Pause | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7910 | Display toolbar/well | `#display-zoom` (`range`) | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7939 | Display toolbar/well | `button` — Adjust lower RX passband edge | Display toolbar / passband | Display prefs/tune/filter TCI |
| 7940 | Display toolbar/well | `button` — Adjust upper RX passband edge | Display toolbar / passband | Display prefs/tune/filter TCI |
| 8009 | Transmit | `#tx-arm-btn` — TX UNAVAILABLE | Transmit | TX safety/source/MON/DSP TCI |
| 8012 | Transmit | `#ptt-btn` — HOLD PTT | Transmit | TX safety/source/MON/DSP TCI |
| 8013 | Transmit | `#mox-btn` — MOX | Transmit | TX safety/source/MON/DSP TCI |
| 8034 | Transmit | `#tx-audio-source` — Browser microphone Native SATP | Transmit | TX safety/source/MON/DSP TCI |
| 8035 | Transmit | `#satp-pair-btn` — Pair native sender | Transmit | TX safety/source/MON/DSP TCI |
| 8040 | Transmit | `#satp-key` (`password`) | Transmit | TX safety/source/MON/DSP TCI |
| 8041 | Transmit | `#satp-copy-key` — Copy key | Transmit | TX safety/source/MON/DSP TCI |
| 8042 | Transmit | `#satp-hide-key` — Hide key | Transmit | TX safety/source/MON/DSP TCI |
| 8045 | Transmit | `#tx-mon-btn` — MON OFF | Transmit | TX safety/source/MON/DSP TCI |
| 8047 | Transmit | `#tx-mon-level` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8053 | Transmit | `#tx-lock-btn` — Lock TX | Transmit | TX safety/source/MON/DSP TCI |
| 8066 | Transmit | `#tx-drive` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8073 | Transmit | `#tx-mic-gain` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8078 | Transmit | `summary` — TX Setup  Gate, DEXP, processor, CESSB and filter   Gate ON \| 50-3050 Hz | Transmit | TX safety/source/MON/DSP TCI |
| 8094 | Transmit | `#apply-voodoo-38k-btn` — Apply Voodoo 3.8k | Transmit | TX safety/source/MON/DSP TCI |
| 8103 | Transmit | `#tx-dexp-enabled` (`checkbox`) | Transmit | TX safety/source/MON/DSP TCI |
| 8109 | Transmit | `#tx-dexp-threshold` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8114 | Transmit | `#tx-dexp-expansion` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8119 | Transmit | `#tx-speech-processor-enabled` (`checkbox`) | Transmit | TX safety/source/MON/DSP TCI |
| 8125 | Transmit | `#tx-speech-processor-gain` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8128 | Transmit | `#tx-cessb-enabled` (`checkbox`) | Transmit | TX safety/source/MON/DSP TCI |
| 8137 | Transmit | `#tx-noise-gate-enabled` (`checkbox`) | Transmit | TX safety/source/MON/DSP TCI |
| 8143 | Transmit | `#tx-noise-gate-threshold` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8153 | Transmit | `#tx-filter-low` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8160 | Transmit | `#tx-filter-high` (`range`) | Transmit | TX safety/source/MON/DSP TCI |
| 8174 | Operations | `#operations-tab-memory` — Memory | Operations / phone / frequency | Drawer or existing operations action |
| 8175 | Operations | `#operations-tab-audio` — Audio | Operations / phone / frequency | Drawer or existing operations action |
| 8176 | Operations | `#operations-tab-network` — Network | Operations / phone / frequency | Drawer or existing operations action |
| 8177 | Operations | `#operations-tab-dsp` — DSP | Operations / phone / frequency | Drawer or existing operations action |
| 8178 | Operations | `#operations-tab-radio` — Radio | Operations / phone / frequency | Drawer or existing operations action |
| 8179 | Operations | `#operations-tab-log` — Log | Operations / phone / frequency | Drawer or existing operations action |
| 8180 | Operations | `#operations-close-btn` — Close | Operations / phone / frequency | Drawer or existing operations action |
| 8186 | Operations | `#operations-memory-save-btn` — Save Current | Operations / phone / frequency | Drawer or existing operations action |
| 8196 | Operations | `#operations-audio-close-btn` — Collapse Scope | Operations / phone / frequency | Drawer or existing operations action |
| 8233 | Phone dock | `button` — Radio | Operations / phone / frequency | Phone panel or delegated RX action |
| 8234 | Phone dock | `button` — RX | Operations / phone / frequency | Phone panel or delegated RX action |
| 8235 | Phone dock | `button` — TX | Operations / phone / frequency | Phone panel or delegated RX action |
| 8236 | Phone dock | `button` — DSP | Operations / phone / frequency | Phone panel or delegated RX action |
| 8237 | Phone dock | `button` — More | Operations / phone / frequency | Phone panel or delegated RX action |
| 8240 | Phone dock | `#mobile-att-btn` — ATT Off | Operations / phone / frequency | Phone panel or delegated RX action |
| 8241 | Phone dock | `#mobile-nr-btn` — NR Off | Operations / phone / frequency | Phone panel or delegated RX action |
| 8242 | Phone dock | `#mobile-nb-btn` — NB Off | Operations / phone / frequency | Phone panel or delegated RX action |
| 8243 | Phone dock | `#mobile-anf-btn` — ANF Off | Operations / phone / frequency | Phone panel or delegated RX action |
| 8244 | Phone dock | `#mobile-filter-btn` — FIL 3.0k | Operations / phone / frequency | Phone panel or delegated RX action |
| 8245 | Phone dock | `#mobile-wake-lock-btn` — Awake Off | Operations / phone / frequency | Phone panel or delegated RX action |
| 8256 | Frequency sheet | `#freq-entry-cancel-btn` — Cancel | Operations / phone / frequency | Tuning or sheet UI state |
| 8261 | Frequency sheet | `button` — 160m | Operations / phone / frequency | Tuning or sheet UI state |
| 8262 | Frequency sheet | `button` — 80m | Operations / phone / frequency | Tuning or sheet UI state |
| 8263 | Frequency sheet | `button` — 60m | Operations / phone / frequency | Tuning or sheet UI state |
| 8264 | Frequency sheet | `button` — 40m | Operations / phone / frequency | Tuning or sheet UI state |
| 8265 | Frequency sheet | `button` — 30m | Operations / phone / frequency | Tuning or sheet UI state |
| 8266 | Frequency sheet | `button` — 20m | Operations / phone / frequency | Tuning or sheet UI state |
| 8267 | Frequency sheet | `button` — 17m | Operations / phone / frequency | Tuning or sheet UI state |
| 8268 | Frequency sheet | `button` — 15m | Operations / phone / frequency | Tuning or sheet UI state |
| 8269 | Frequency sheet | `button` — 12m | Operations / phone / frequency | Tuning or sheet UI state |
| 8270 | Frequency sheet | `button` — 10m | Operations / phone / frequency | Tuning or sheet UI state |
| 8271 | Frequency sheet | `button` — 6m | Operations / phone / frequency | Tuning or sheet UI state |
| 8272 | Frequency sheet | `button` — FM | Operations / phone / frequency | Tuning or sheet UI state |
| 8275 | Frequency sheet | `button` — 1 | Operations / phone / frequency | Tuning or sheet UI state |
| 8276 | Frequency sheet | `button` — 2 | Operations / phone / frequency | Tuning or sheet UI state |
| 8277 | Frequency sheet | `button` — 3 | Operations / phone / frequency | Tuning or sheet UI state |
| 8278 | Frequency sheet | `button` — 4 | Operations / phone / frequency | Tuning or sheet UI state |
| 8279 | Frequency sheet | `button` — 5 | Operations / phone / frequency | Tuning or sheet UI state |
| 8280 | Frequency sheet | `button` — 6 | Operations / phone / frequency | Tuning or sheet UI state |
| 8281 | Frequency sheet | `button` — 7 | Operations / phone / frequency | Tuning or sheet UI state |
| 8282 | Frequency sheet | `button` — 8 | Operations / phone / frequency | Tuning or sheet UI state |
| 8283 | Frequency sheet | `button` — 9 | Operations / phone / frequency | Tuning or sheet UI state |
| 8284 | Frequency sheet | `button` — . | Operations / phone / frequency | Tuning or sheet UI state |
| 8285 | Frequency sheet | `button` — 0 | Operations / phone / frequency | Tuning or sheet UI state |
| 8286 | Frequency sheet | `button` — Del | Operations / phone / frequency | Tuning or sheet UI state |
| 8287 | Frequency sheet | `button` — Clear | Operations / phone / frequency | Tuning or sheet UI state |
| 8290 | Frequency sheet | `button` — MHz | Operations / phone / frequency | Tuning or sheet UI state |
| 8291 | Frequency sheet | `button` — kHz | Operations / phone / frequency | Tuning or sheet UI state |
| 8292 | Frequency sheet | `button` — Hz | Operations / phone / frequency | Tuning or sheet UI state |
| 8305 | Detail / Help | `#operator-detail-copy-rx-btn` — Copy RX Latency | Details / terrain | Detail/copy/help UI state |
| 8306 | Detail / Help | `#operator-detail-close-btn` — Close | Details / terrain | Detail/copy/help UI state |
| 8320 | Detail / Help | `#shortcut-close-btn` — Close | Details / terrain | Detail/copy/help UI state |
| 8367 | Terrain settings | `button` — Close | Details / terrain | 3D renderer/display prefs |
| 8369 | Terrain settings | `#terrain-traditional-settings` — Traditional appearance settings | Details / terrain | 3D renderer/display prefs |
| 8371 | Terrain settings | `#terrain-mode` — Traditional High-Res 3D | Details / terrain | 3D renderer/display prefs |
| 8372 | Terrain settings | `#terrain-height` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8373 | Terrain settings | `#terrain-depth` (`number`) | Details / terrain | 3D renderer/display prefs |
| 8374 | Terrain settings | `#terrain-elevation` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8375 | Terrain settings | `#terrain-perspective` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8376 | Terrain settings | `#terrain-floor` (`number`) | Details / terrain | 3D renderer/display prefs |
| 8377 | Terrain settings | `#terrain-ceiling` (`number`) | Details / terrain | 3D renderer/display prefs |
| 8378 | Terrain settings | `#terrain-fit-range` — Fit range to current signals | Details / terrain | 3D renderer/display prefs |
| 8380 | Terrain settings | `#terrain-gamma` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8381 | Terrain settings | `#terrain-cleanup` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8382 | Terrain settings | `#terrain-waterfallCleanup` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8383 | Terrain settings | `#terrain-cleanupBaseline` (`number`) | Details / terrain | 3D renderer/display prefs |
| 8384 | Terrain settings | `#terrain-noise-estimate` — Re-estimate noise baseline | Details / terrain | 3D renderer/display prefs |
| 8387 | Terrain settings | `#terrain-smoothing` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8388 | Terrain settings | `#terrain-palette` — Reference Blue / Rainbow Reference Dark Classic Ember Ice Forest Enhanced | Details / terrain | 3D renderer/display prefs |
| 8391 | Terrain settings | `#terrain-split` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8392 | Terrain settings | `#terrain-quality` — Auto Performance (30 fps target) Balanced (60 fps target) High (60 fps target) | Details / terrain | 3D renderer/display prefs |
| 8393 | Terrain settings | `#terrain-gridOpacity` (`range`) | Details / terrain | 3D renderer/display prefs |
| 8394 | Terrain settings | `#terrain-diagnostics-enabled` (`checkbox`) | Details / terrain | 3D renderer/display prefs |
| 8396 | Terrain settings | `#terrain-reset` — Reset 3D view | Details / terrain | 3D renderer/display prefs |

Static element count: **274**.

### Other user-facing and dynamic controls

- Operator state strip: eight clickable `role=button` pills `operator-conn-pill`, `operator-owner-pill`, `operator-rxtx-pill`, `operator-rf-pill`, `operator-transport-pill`, `operator-latency-pill`, `operator-audio-pill`, and `operator-fault-pill` open the operator detail overlay through `bindOperatorStateDetail`; their values come from connection, bridge ownership/TX, transport, latency, audio and fault state.
- VFO readouts and digit controls: `#dds-readout` and `#operator-center-readout` are interactive via `bindFrequencyDigitControl`; wheel/keyboard/double-click/direct entry ultimately tune through the current frequency handler/TCI path. The secondary VFO has a readout and separate A/B select buttons. `#spectrum-shell` and `#waterfall-shell` are keyboard-focusable panadapters with click/wheel interactions. The `data-passband-drag="body"` element and its lower/upper handles adjust the RX filter through `bindPassbandDragControls`; those controls are separate from the canvas pixels.
- Runtime-created phone panel/context controls and operations buttons are built in `initPhonePanels`, `initControlContextRail`, and `updateOperationsDrawer` (roughly lines 11545–11970); they change the same layout/context/operations state as their static counterparts. RX and TX EQ slider rows are generated near lines 20813 and 20862 and send their corresponding EQ settings.
- Non-control readouts to preserve include connection/role/RX-TX/RF/transport/latency/audio/fault pills; VFO A/B frequencies, mode/band, ADC/ANT/ATT/IQ chain; live WebSocket URL; sample rate/RTT; analog and digital S-meter plus L/R dBFS; display span/zoom/average/peak/history and band-edge/passband labels; RX rate/lead/resync/path/buffer; TX lock reason, mic/power/SWR, native SATP status/key, MON destination; operations log/network diagnostics; terrain legend/diagnostics.

## Section 9 checklist gaps and source conflicts

Add explicit migration rows for setup profiles/startup profile; system WS probe/copy log and management links; detailed display appearance controls and undo/clarity; NR2 gain/NPE/post-filter and ANR/ANF tuning; RX/TX EQ sliders and presets; TX timeout, phase rotator, PureSignal calibration, two-tone test and its parameters, CFC precomp; network stream mode/diagnostics and WebSocket URL; input/output device selectors; WBFM deemphasis; TX drive, mic gain, DEXP, speech processor, CESSB, noise gate and TX filter; SATP key copy/hide; phone dock/quick controls; frequency keypad; operator detail copy; all terrain geometry, cleanup, quality, palette, diagnostics, reset, and legacy look options. Keyboard shortcuts and all query parameters also need rows. Section 9's “all 13 bands” conflicts with the current 12 visible band buttons (160m through 6m, plus FM); confirm the full band list with the owner in Phase 3 rather than inventing a thirteenth button.

The build sheet's default Dark theme differs from the current unsaved preference, which follows OS color scheme. That is an intentional future presentation change, but migration must preserve a user’s explicitly saved dark/light choice. Its request for a default new colormap and a Classic option must retain the existing palettes and 3D look. The display spec assumes a flat waterfall below 3D; today's `#terrain-canvas` is laid over the existing stack, so Phase 4 must verify simultaneous layout without dropping Traditional mode.

The current Hold PTT pointer handler is momentary and releases on multiple page lifecycle events, but `setPtt(true)` can itself call `armTxReady` and return false when disarmed. The redesign must not turn that into an alternate keying path. The build sheet's requested visual “Disarmed” state must map to these actual gate conditions without changing them. The bridge has 750 ms TCI-mic and 1500 ms operator-text inactivity watchdogs in `saturn-bridge/src/xdma_backend.rs`; these apply only when their inputs go stale. The mic watchdog applies only with the TCI audio source, not Native SATP. Normal `saturn_ping` and S-meter text traffic can keep the control watchdog fresh, so neither watchdog guarantees release if a live UI fails to send `trx:0,false;`. The existing immediate browser release paths remain necessary. The build sheet mentions a separate “Reference” control, while current markup provides a VFO B state label that can read “Reference” and an A/B selector; no independent Reference button was found. Phase 3 must preserve the existing A/B/split behavior and flag this difference for the owner.

## Improvement discussion

SilverForge opened Agent Mail topic `ui-redesign`, message 97, with GoldMarsh and RubyMill and copied HumanOverseer. RubyMill's source review in reply 98 confirmed the TX release paths, query/storage omissions, asset allowlist, and deploy markers above. A promising Phase 3 implementation idea is to move existing TX nodes without recreating them, retain their IDs, keep release behavior in `setPtt`/`lockTx`, and test pointerup, pointercancel, mouse pointerleave, lost capture, blur, visibility hidden, pagehide, Escape, and socket close. This is a proposal for the owner/safety review after the Phase 0 stop, not an implementation decision. RubyMill also highlighted iOS audio gesture/WebCodecs fallback, DPR changes, WebGL context loss, phone safe areas/landscape, touch passbands, WAN spectrum negotiation, and long sessions as high-value QA cases.
GoldMarsh's bridge review in reply 100 identified the conditional TX watchdogs and confirmed the asset route restriction. Its suggested Rust asset-route change conflicts with the build sheet's backend prohibition, so asset bundling/inlining is the current compliant path.

RubyMill's further review in message 104 found that section 11.3's exact `s = 0.55 + 0.45 * d` / `y = H * 0.18 + d * H * 0.80` prescription would undo the current terrain far-field perspective model, which replaced a hard-coded 0.18 compression. Section 1 says preserve current behavior and flag conflicts, so Phase 4 must retain the current perspective model until the owner reviews side-by-side results. RubyMill also identified RX codec negotiation (`audio_gain`/`audio_codec`), RX codec diagnostics, reconnect supervisor state, and live client trace as additional parity rows. Performance measurement must distinguish LAN raw-IQ processing from WAN server-row processing, record heap growth after warm-up, and use browser-specific heap tools rather than assume Chrome's `performance.memory` works everywhere.
GoldMarsh verified two more section 11.3 conflicts: the current terrain renderer forbids depth-based color tinting, enforced by `scripts/terrain-controlled.mjs`, and shipped history tiers are 48/128/256 rows with a user depth range of 8–256, not the sheet's 64/48/28 rows and 128 cap. The renderer's WebGL2 requirement also means a Canvas 2D context-loss fallback is new Phase 4 capability. GoldMarsh mapped the existing six TX presentation code states to the sheet's five visual rows; the `engaging` state needs an Armed-family “Keying…” sublabel until the bridge confirms keyed. Its existing ready countdown, five-minute expiry, and six lock reason strings are additional migration items.
