# Saturn Remote Responsive UI Audit

Date: 2026-08-15
Scope: `/remote-next` presentation layer before the responsive transceiver redesign

## Executive summary

Saturn Remote is not a React application. It is a Vite-built TypeScript library exposed as
`globalThis.SaturnRemoteNext`, plus a large server-rendered HTML template that owns the DOM,
CSS, browser orchestration, WebGL renderers, event wiring, audio worklets, and the remaining
mutable application state. The backend boundary is sound and should remain intact.

The redesign can begin without changing Protocol 2, DSP, websocket framing, audio transport,
or the TX owner. The safest seam is the existing DOM IDs and `data-*` attributes in
`templates/saturn-remote-next.html`: new semantic containers and CSS can rehouse the controls
while current event handlers continue to target the same elements.

The largest maintainability risk is the 15,000-line template. The incremental architecture
should first restyle and reflow it, then extract view components and renderers into typed
`remote-web/src/ui` modules without duplicating radio state or command logic.

## 1. Current UI and component architecture

### Delivery path

- `update_manager/templates/saturn-remote-next.html` is the only live page. It contains the
  markup, two generations of inline CSS, inline AudioWorklet source, WebGL/Canvas renderers,
  state-to-DOM rendering, control event handlers, connection orchestration, TX mic capture,
  and startup.
- `update_manager/remote-web/src/remote-next-entry.ts` builds an IIFE named
  `saturn-remote-next.js` and publishes a flat helper API as `globalThis.SaturnRemoteNext`.
- `update_manager/remote-web/scripts/check-template-seam.mjs` keeps the bundle/template API
  seam exact. Any extracted helper used by the template must be exported through this seam.
- `update_manager/rust-server/src/remote_tls.rs` serves the HTML, bundle, and Inter font behind
  the existing authenticated TLS routes. Release scripts copy the template and built bundle.

### Current visual tree

```text
.page.console-page
├── header.console-header
├── section.operator-state-strip
├── section.command-strip
│   ├── primary VFO deck
│   ├── Go Live / Setup controls
│   └── SVG S / power / SWR meters
├── section.audio-control-strip
│   └── volume, AGC/ANF, NR/NB, RX filter
└── section.console-layout (display: contents on wide screens)
    ├── .left-rail
    │   ├── Routing
    │   ├── Bands
    │   └── Demod/mode
    ├── .console-stage
    │   ├── panadapter/waterfall display card
    │   └── permanently expanded Operator Log
    └── .right-rail
        └── TX/interlock, PTT/MOX, TX controls
```

Overlays/sheets are siblings after the page: frequency entry, operator-state detail, and TX
tools. Setup is currently nested in the command strip as an anchored menu.

### State-to-view flow

```text
WebSocket text ─> tci/parser.ts ─> tci/apply.ts ─> mutable AppState ─> updateUi()
WebSocket binary ─> template frame classifier ─┬─> IQ history/FFT/render loop
                                               └─> RX audio scheduling/worklet
DOM event ─> normalize/build command ─> sendTci() ─> bridge ─> echoed authoritative state
```

`updateUi()` and a requestAnimationFrame-coalesced refresh update the existing DOM by ID.
Meter and panadapter animation run independently in the persistent animation loop.

## 2. Panadapter and waterfall rendering paths

### IQ ingest and FFT

- Binary socket messages are classified in the template and dispatched by
  `handleBinaryFrame()`/`handleIqFrame()`.
- IQ frames use the existing 64-byte TCI-style header and interleaved float32 samples.
- `handleIqFrame()` records RX or TX IQ packets, sample rate, freshness, and frame version.
- `buildRenderIqWindow()` combines bounded recent packets.
- `remote-web/src/dsp/fft.ts` supplies `FftProcessor`; `remote-web/src/dsp/display.ts` supplies
  zoom cropping, bin shifting, smoothing, band edges, scales, and auto-ranging.
- `animationLoop()` computes only when a new IQ frame is available and observes LAN/WAN/phone
  frame-rate and FFT-size limits.

### Spectrum

- `SpectrumRenderer` is defined in the template.
- WebGL2 is primary; Canvas2D is the fallback.
- It resizes from its host bounds using `devicePixelRatio`, renders a purple/configurable trace,
  fill and restrained glow, and leaves scales/passband/band edges as DOM overlays.
- `#spectrum-shell` owns tuning gestures and `#filter-window` owns draggable passband edges/body.
  These handlers ultimately call the existing VFO and RX-filter command paths.

### Waterfall

- `WaterfallRenderer` is defined in the template.
- WebGL2 uses a scrolling RGBA texture; Canvas2D is the fallback.
- It shares the same processed bins, center frequency, display span, band-edge overlay, and
  filter overlay as the spectrum, which preserves alignment.
- Frequency changes shift existing waterfall pixels before new data settles. Speed, palette,
  contrast, smoothing, auto range, and phone/WAN throttling already exist.

### Reuse decision

Keep both renderers, FFT/display helpers, frame throttling, gesture math, and existing canvas
elements during the shell redesign. Later extract the two renderer classes from the template
without changing their public behavior.

## 3. Radio state and control sources

### Browser state

- `remote-web/src/state/app-state.ts` defines and initializes the comprehensive mutable
  `AppState`: connection, radio, RX DSP, filters, TX, meters, display, audio, diagnostics,
  profiles, layout, and runtime objects.
- `remote-web/src/tci/state.ts` defines the typed bridge-owned radio subset.
- `remote-web/src/controller/create-controller.ts` is a typed controller path for settings,
  TCI text, and binary frames. The live template still directly orchestrates much of the same
  state, so this is a migration seam rather than the sole runtime owner today.
- `state/prefs-from-state.ts` and `state/apply-prefs.ts` copy between live state and persisted
  radio/display preference objects.

### Authoritative inbound state

- `tci/parser.ts` tokenizes TCI text.
- `tci/apply.ts` applies bridge messages for ready state; VFO/DDS; modulation; ADC/antenna;
  sample rate; RX volume/NR/NB/ANF/AGC/filter; meters; bridge latency/backpressure; client role;
  RF enable; TX phase/fault/codec/uplink; EQ/CFC/PureSignal/two-tone; and audio state.
- `syncTciUiSideEffects()` applies the parsed state, handles readiness/reconnect recovery,
  updates affected controls, responds to authoritative TX release/fault, and starts safety
  timeouts for armed/keyed phases.

### Outbound controls

- `tci/commands.ts` contains reusable builders for RX/TX filters, ANR, ANF, two-tone, codec
  capability, and complete radio preference replay.
- Frequency, band, mode, antenna/ADC, sample rate, volume, AGC, NR, NB, ANF, filters, TX drive,
  mic gain, EQ/CFC, phase rotator, PureSignal, and stream commands are sent through the existing
  template `sendTci()` boundary.
- Band recall stores frequency plus partial radio/display preferences and replays them through
  the same control path.
- `transport/split-sockets.ts`, `legacy-socket-adapter.ts`, and
  `reconnect-supervisor.ts` preserve the control/media lanes and supervised reconnect behavior.

The new UI must bind to these existing state fields and command builders. It must not create a
second radio store or desktop/mobile-specific command paths.

## 4. TX, PTT, MOX, and interlock path

### Operator request path

1. `bindPttButton()` binds momentary pointer PTT, latched MOX, manual lock, Escape, page-hide,
   visibility, focus-loss, and microphone-device-change handling.
2. `setPtt(true)` checks, in order, bridge RF enable/WFM RX-only policy, bridge role/readiness,
   socket connection, and the local reconfirm window.
3. If reconfirm is closed, `armTxReady()` opens the bounded ready window and returns without
   transmitting. A second deliberate PTT/MOX action is required.
4. A permitted request marks the local request, prioritizes the TX media lane, mutes RX,
   switches display handling for TX IQ, sends `trx:0,true,tci;`, and starts mic capture (or the
   explicitly selected generated diagnostic source).
5. The bridge reports `tx_state: armed|keyed|rx`, `trx`, or `tx_fault`; `tci/apply.ts` treats
   that report as authoritative.

### Release and fail-closed behavior

- `setPtt(false)` immediately clears local TX state, sends `trx:0,false;`, stops mic capture,
  restores RX/audio, and optionally locks the reconfirm window.
- Pointer release/cancel/lost capture, page hide, visibility loss, pointer focus loss, Escape,
  socket close, mic failure/device change, role loss, and bridge TX fault all release or lock TX.
- The bridge-side TX owner remains responsible for WDSP/DUC/P2 sequencing and only asserts RF
  after a valid nonzero TX IQ packet exists. Browser UI work must not weaken this boundary.
- `updateTxZone()` is the current presentation adapter for disabled, locked, ready, armed,
  keyed, viewer/role-pending, WFM RX-only, RF-disabled, and diagnostic states.

### Redesign rule

Reuse the exact buttons/IDs and `setPtt()`/`lockTx()` path in early phases. New TX visuals may
mirror these states, but no new element may send `trx` directly or infer permission from color.

## 5. Profile and settings persistence

### Server persistence

- Working settings: `GET/POST /remote_settings` -> `remote_settings.json`.
- Profile catalog: `GET /remote_profiles` and save/delete/startup POST endpoints ->
  `remote_profiles.json`.
- `rust-server/src/state.rs` defines the persisted server structures;
  `rust-server/src/main.rs` performs atomic file load/save; TLS routes enforce auth and CSRF.
- Profiles contain websocket/network selection, display/layout/theme preferences, phone panel
  state, radio preferences, display preferences, and band memories.

### Browser fallback and fast preferences

- `runtime/storage.ts` defines the typed `saturn.remote.settings` fallback.
- The template also retains compatibility keys for websocket URL, layout, theme, phone panels,
  phone waterfall, tuning step, display zoom, frequency lock, wake lock, band memories,
  active profile, display preferences, and radio preferences.
- Server saves are debounced; local fallback is written first. Startup profiles load before
  the live radio preference replay.

UI-only preferences should be added to the display/layout portion of this contract rather than
to radio preferences. A migration must continue accepting older stored documents.

## 6. Existing desktop/mobile responsive behavior

- Wide layout is CSS Grid with left/stage/right columns and separate header/state/command rows.
- Existing media queries are centered at 1520, 1360, 1200/1080, and 780 px. Several are from an
  older stylesheet and are then overridden by `#saturn-operator-layout-v2`, making precedence
  difficult to reason about.
- Below 1200 px the latest shell changes to a two-column rail/stage layout; below 780 px it
  becomes a vertical stack.
- Runtime layout has only `desktop|phone` and is selected manually/persisted. Phone mode adds
  per-panel Show/Hide controls and an optional waterfall; it does not currently model tablet as
  a first-class mode.
- Phone-specific render throttling also activates for coarse pointers, WAN mode, Tailnet, or
  high RTT. This performance behavior is useful and should be retained independently of visual
  layout.
- Existing touch tuning and draggable passband handling are reusable. Current mobile control
  order is still document-flow based, and TX is not yet a true persistent operating dock.

The new shell should standardize four CSS ranges: >=1440 wide desktop, 1024-1439 compact
desktop/large tablet, 768-1023 tablet, and <768 phone, with a landscape-phone height override.

## 7. Components and systems to reuse

- Websocket/session transport, split-lane adapter, reconnect supervisor, and TCI parser/apply.
- Typed app/radio state, normalization, preference extraction/application, and command builders.
- Spectrum and waterfall canvases/renderers, FFT/display math, DPR resizing, range controls,
  frequency scale, band edges, and render throttling.
- Passband overlay math and existing tune/drag/filter command handlers.
- Frequency formatting, digit-step selection, keypad entry, keyboard tuning, and band memory.
- SVG meter math and requestAnimationFrame attack/decay/peak-hold state.
- Operator state derivation and detail data.
- TX reconfirm, interlock, PTT/MOX, mic capture/uplink, release, timeout, and fault paths.
- RX audio worklet/message fallback, resampling, jitter telemetry, and recovery behavior.
- Profile/settings server APIs, local fallback, and setup field bindings.

## 8. Components to replace or rehouse

- Replace the visual shell and competing inline layout layers with one tokenized responsive
  shell. Preserve IDs until view bindings are extracted.
- Replace the generic hero/header actions with a compact application header and quiet status
  strip. Move management/diagnostic links to System/Diagnostics.
- Replace the full-width command strip appearance with a center instrument deck.
- Rehouse Routing under Advanced/Network instead of the normal left rail.
- Rehouse RX audio/DSP controls into contextual RX and DSP surfaces.
- Replace the single TX side card appearance with a shared `TXInterlock`/`PTTControl` visual,
  while retaining its command handlers.
- Replace the permanently expanded Operator Log with a collapsed operations drawer.
- Replace the anchored setup menu with a right-side drawer/full-height sheet.
- Replace manual phone-only collapsing with shared responsive panels and a reachable mobile dock.
- Extract renderer/view classes from the template after the shell is stable; do not rewrite
  their algorithms as part of the visual migration.

## 9. Proposed file and component architecture

The current application uses framework-free TypeScript, so introducing React during a
radio-console redesign would add risk without improving the backend boundary. Use typed DOM
components with explicit `mount/update/destroy` lifecycles and keep radio commands in the
existing controller/TCI modules.

```text
remote-web/src/
├── app/
│   ├── bootstrap.ts                 page startup and dependency assembly
│   └── ui-controller.ts             state-to-view scheduling only
├── controller/                      existing radio/session controller
├── state/                           existing app state and preference adapters
├── radio/                           existing frequency/band/passband policy
├── tci/                             existing parser, apply, command builders
├── transport/                       existing websocket and media transport
├── audio/                           existing RX/TX audio paths
├── dsp/                             existing FFT/display math
└── ui/
    ├── foundation/
    │   ├── tokens.css
    │   ├── reset.css
    │   └── responsive.ts
    ├── shell/
    │   ├── RadioShell.ts
    │   ├── RadioHeader.ts
    │   ├── StatusStrip.ts
    │   ├── BottomDrawer.ts
    │   ├── MobileControlDock.ts
    │   └── SetupDrawer.ts
    ├── vfo/
    │   ├── VfoDeck.ts
    │   ├── VfoFrequency.ts
    │   └── VfoSelector.ts
    ├── meters/
    │   ├── MeterFrame.ts
    │   ├── NeedleMeter.ts
    │   ├── SMeter.ts
    │   ├── PowerMeter.ts
    │   ├── SwrMeter.ts
    │   ├── AudioMeter.ts
    │   └── BarMeter.ts
    ├── spectrum/
    │   ├── PanadapterPanel.ts
    │   ├── SpectrumRenderer.ts
    │   ├── WaterfallRenderer.ts
    │   ├── PassbandOverlay.ts
    │   └── PanadapterGestures.ts
    ├── controls/
    │   ├── BandSelector.ts
    │   ├── ModeSelector.ts
    │   ├── RxControls.ts
    │   ├── DspControls.ts
    │   └── TxControls.ts
    ├── tx/
    │   ├── TxInterlock.ts
    │   └── PttControl.ts
    └── diagnostics/
        ├── TelemetryPanel.ts
        └── OperatorLog.ts
```

During extraction, `RadioShell` should move existing elements rather than clone them. One DOM
control must continue to represent each radio command, and all form factors must consume the
same state and action objects.

## 10. Phased implementation plan

### Phase 1 - Audit (complete in this document)

Record component/state/render/TX/persistence boundaries; run the current build, typecheck,
smoke tests, seam check, and layout validation to establish a baseline.

### Phase 2 - Design tokens

Add the graphite workspace, semantic RX/ready/caution/TX/fault colors, UI/instrument fonts,
spacing, radii, elevation, focus, touch target, and motion tokens. Map legacy variables to the
new tokens so existing controls adopt the system without logic changes. Add reduced-motion and
high-contrast-safe focus behavior.

### Phase 3 - Responsive shell

Create the header/status/deck/left/workspace/right/drawer grid areas. Implement the four target
ranges and landscape-phone override. Keep the panadapter first in mobile operating order, add
shell navigation hooks, and verify no horizontal overflow at 390x844, 844x390, 768x1024,
1024x768, 1440x900, 1920x1080, and 2560x1440.

### Phase 4 - Instrument deck

Extract the VFO presentation and reusable SVG meter framework. Preserve tuning handlers and
meter animation state. Add responsive meter detail, overload annunciation, TX meter mode, audio
dBFS meters, tabular numeric figures, and reduced-motion behavior.

### Phase 5 - Panadapter workspace

Rehouse the existing renderers and overlays, then extract their classes. Add the operator-first
toolbar, aligned scale treatment, resizable spectrum/waterfall split, and phone gesture policy.
Keep IQ/FFT/render scheduling unchanged unless profiling justifies a measured change.

### Phase 6 - Radio controls

Build shared band/mode selectors and RX/DSP/TX contextual surfaces. Move websocket/routing and
engineering telemetry out of the primary rails. Bind every surface to existing state/actions.

### Phase 7 - TX workflow (safety-critical)

Wrap the existing reconfirm/interlock and `setPtt()` path in shared TX components. Implement
explicit disabled/locked/available/armed/transmitting/fault states, sticky phone controls,
momentary PTT >=64 px, distinct MOX, TX-frequency annunciation, and TX-focused meters. Add
pointer/keyboard/visibility/socket/mic/role regression tests before visual polish.

### Phase 8 - Operations drawer

Add Memory, Audio, Network, DSP, Radio, and Log sections. Move the operator log and detailed
telemetry out of the permanent workspace. Persist drawer section/size as UI preferences.

### Phase 9 - Setup drawer

Move current setup content into the responsive settings drawer/sheet, expand navigation to
Profiles, Display, DSP, Transmit, Network, Audio, and Advanced, and preserve server/local profile
contracts.

### Phase 10 - Polish and acceptance

Tune meter damping, focus order, touch gestures, alert transitions, high-DPI visuals, and
performance. Execute the full RX/IQ/audio/control/TX/profile/reconnect regression matrix and
the desktop/tablet/phone operator workflows from the redesign specification.

## Implementation status (2026-08-25)

The presentation refactor described above is now implemented on
`feature/saturn-remote-responsive-redesign` through Phase 16. Existing DSP, audio transport,
and TX safety ownership remain in place; new controls extend their established state and command
paths rather than adding a second radio implementation.

- Phases 2-3: semantic console tokens, wide/compact/tablet/phone layouts, compact status
  header, primary VFO deck, and thumb-reachable mobile navigation.
- Phase 4: shared SVG meter math, damped S/power/SWR readings, peak hold, TX-oriented meter
  presentation, overload/fault annunciation, and responsive meter detail.
- Phase 5: the existing WebGL/Canvas spectrum and waterfall renderers are rehoused in the
  primary workspace with aligned scales, band/filter overlays, operator controls, an
  accessible resize divider, high-DPI resizing, and deliberate phone gestures.
- Phases 6-7: shared RX/DSP/TX contextual controls and explicit disabled, locked, available,
  armed, transmitting, and fault TX presentation states wrap the existing interlock/PTT/MOX
  command path. Arming still never keys the radio.
- Phases 8-9: Memory/Audio/Network/DSP/Radio/Log operations drawer and the seven-section
  responsive setup drawer replace the permanent log and oversized setup surface.
- Phase 10: UI-only spectrum/waterfall preferences, three phone display proportions,
  keyboard resizing, touch tap/drag/pinch/double-tap/long-press handling, ResizeObserver
  isolation, visible focus, and reduced-motion behavior are in place.
- Phases 11-12: desktop VFO and meter instrumentation is approximately 20% more compact,
  meter motion is less rigid, and the complete appliance console is constrained to a
  1920x1080 LCD viewport without hiding the waterfall or collapsed Operations drawer.
- Phase 13: the supplied desktop and phone design references are reflected more directly
  in the production shell: desktop status moves into the application header, VFO A and
  the shared meter bank dominate the instrument deck, session/setup actions no longer
  consume a meter column, band selection uses a four-column operator grid, and phone
  keeps all eight safety states in a compact two-row strip with settings in the header.
- Phase 14: the 1920x1080 RX context rail no longer clips common controls behind an
  artificial short scroll region. RX volume and health, AGC/ANF, filter controls, and the
  authoritative TX arm/PTT/MOX surface share the visible operating rail; the TX tab still
  expands the complete transmit setup without creating a second command path.
- Phase 15: the primary band rail includes 60 metres, VFO B is visible as a deliberately
  subdued reference, spectrum average and peak
  hold are promoted to the operator toolbar, and engineering links are grouped under the
  System menu. Phone adds persistent NR/NB/ANF/filter shortcuts above its operating dock.
  The Operations Audio panel now renders a throttled stereo RX waveform and calibrated L/R
  dBFS peak bars from the existing decoded audio frames without joining the IQ render loop.
- Phase 16: VFO A/B selection and SPLIT now route the actual RX and TX frequencies through
  the existing TCI/model/Protocol 2 and direct-XDMA paths. RX attenuation drives the Saturn
  ADC attenuator; speech squelch drives WDSP SSQL in supported voice/data modes. The shared
  telemetry path now reports reflected power, ALC, compression, microphone peak, ADC peak,
  and ADC overflow, with an explicit OVF annunciator in the instrument deck. Operator
  follow-up moved ATT into the always-visible VFO path and phone quick bar, added compact
  desktop L/R RX audio meters, added an explicit Operations close control, corrected OVF
  hidden-state rendering, and made direct-XDMA squelch commands resynchronize WDSP.

The following controls remain intentionally deferred rather than rendered as non-functional
switches: a distinct RX preamp has no verified Saturn wire/register command; VOX needs a
pre-key microphone analysis path and the same arm, timeout, role, disconnect, and stale-audio
interlocks as PTT; TUNE needs a single-tone RF source plus the existing TX qualification and
power/SWR trip behavior; and TX monitor needs a dedicated echo-safe browser audio route.

Automated acceptance currently passes 50 test files / 405 tests and 195 bridge tests,
TypeScript typechecking,
the 167-entry template/bundle seam check, the production build, and responsive Chromium
geometry validation across phone portrait/landscape, tablet portrait/landscape, compact
desktop, 1440 desktop, 1920 HD, and 2560 ultrawide, including representative drawer and
setup-open states.

Live RF acceptance remains deliberately separate. RX/IQ/audio and TX interlock/PTT/MOX must
be exercised against the intended Saturn G2 with the operator-confirmed RF-safe load and
hardware ownership before any transmit command is sent. No radio was keyed during this UI
implementation pass.

## Initial implementation boundary

The first code increment after this audit is limited to Phase 2 and Phase 3 foundations:
semantic tokens, one coherent responsive shell, calm default styling, and layout/navigation
scaffolding. It will not change bridge commands, DSP algorithms, audio transport, panadapter
rendering, or TX safety logic.

## Visual references reviewed

The three PNGs in `/home/pi/Pictures` were reviewed after the structural audit:

- `ChatGPT Image Aug 15, 2026, 09_59_51 PM.png` reflects the current desktop composition and
  confirms the oversized routing/log surfaces and center-workspace constraints described above.
- `ChatGPT Image Aug 15, 2026, 09_59_47 PM.png` is the desktop design-intent reference: compact
  header, left operator rail, integrated VFO/meter deck, dominant aligned spectrum/waterfall,
  contextual right rail, and shallow lower operations region.
- `ChatGPT Image Aug 15, 2026, 09_59_33 PM.png` is the phone design-intent reference: large VFO,
  adaptive meters, persistent spectrum/waterfall context, operating tabs, and a thumb-reachable
  TX interlock/PTT surface.

The implementation will use their hierarchy, density, and semantic emphasis, not reproduce
their generated text, proportions, or control artifacts pixel-for-pixel.
