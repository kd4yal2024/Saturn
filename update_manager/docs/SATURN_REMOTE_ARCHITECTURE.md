# Saturn Remote Architecture

## Purpose

`saturn-remote` is the browser frontend for the `saturn-bridge` backend.

It ships as a single page:

- `/remote-next` — rendered from `saturn-remote-next.html` plus the Vite-built IIFE bundle `saturn-remote-next.js`. The HTML loads the bundle via `<script src="/remote-assets/remote-next.js">`; the bundle assigns its API to `globalThis.SaturnRemoteNext`, which the inline page script destructures to reuse extracted helpers (clamps, normalizers, DSP/FFT, app-state, settings, TCI commands).

The legacy inline `/remote` page (`saturn-remote.html` and its `saturn-remote-*.js` modules) was retired on 2026-07-14; `/remote`, `/saturn-remote`, and the TLS root `/` now redirect to `/remote-next`. Auth (basic-auth gate), the `saturn-bridge` TCI websocket, and persisted state (`remote_settings.json`, `remote_profiles.json`) are unchanged. Ongoing migration work now targets narrowing the template/bundle seam inside `/remote-next` itself.

The template/bundle seam is the flat api object in `remote-web/src/remote-next-entry.ts`. It is kept exact — no unused exports, no unresolved template references — by `remote-web/scripts/check-template-seam.mjs` (`npm run check:seam`, enforced in CI).

The Saturn Go proxy dials the bridge with lane-declaring websocket paths (`/control`, `/media`), so the bridge filters each split socket to its lane from the moment of connect — media sockets never receive the text state snapshot. The in-band `session_lane` command remains the pairing mechanism and the fallback for direct `:50001` clients.

It is intentionally not a browser skin wrapped directly around `p2app`.
The backend boundary is:

- `p2app`
  - radio runtime engine
- `saturn-bridge`
  - downstream Protocol 2 client
  - radio/session model
  - protocol adapters like TCI
  - future WDSP DSP host
- `saturn-remote`
  - browser control and rendering client

## Frontend Split

### DOM Layer

Keep these in normal HTML/CSS/JS:

- connection status
- endpoint selection
- VFO and DDS entry
- mode buttons
- filter controls
- meter readouts
- session policy and diagnostics
- logs and support-facing detail

Why:

- easier to maintain
- easier to test
- no reason to force ordinary UI into the GPU path

### GPU Layer

Keep these in WebGL2:

- panadapter trace
- waterfall
- dense cursor/passband overlays
- future multi-slice spectral layers

Why:

- high-DPI support without blurry canvas scaling
- better headroom for dense redraws
- cleaner future path to larger FFT and multi-receiver display work

## Current Browser Contract

The first page targets the current `saturn-bridge` TCI endpoint.

Text control subset:

- `ready`
- `vfo`
- `dds`
- `modulation`
- `rx_filter_band`
- `rx_smeter`
- `tx_power`
- `swr`
- `iq_samplerate`
- `iq_start`
- `iq_stop`

Binary IQ subset:

- TCI-style 64-byte header
- float32 interleaved IQ payload
- browser computes FFT locally for the panadapter

TX control subset:

- PTT/MOX is control-plane first; browser UI and `trx` command update
  immediately and do not wait for microphone capture startup.
- Browser mic frames are intentionally small so release commands are not
  trapped behind large audio buffers.
- `saturn-bridge` drains pending websocket frames in bounded batches.
- Browser TX commands feed a dedicated bridge-side TX owner thread. That thread
  owns WDSP TXA, DUC IQ output, and the P2 high-priority TX bit so stale mic
  frames cannot re-key the radio after release.
- PTT-on arms WDSP and DUC setup first. The bridge asserts P2 TX only after it
  has a complete nonzero DUC IQ packet ready, avoiding a keyed/no-audio RF
  state. If no usable TX IQ appears within the arm timeout, the TX owner
  disarms without keying the radio.
- The TX WDSP channel follows piHPSDR's Protocol 2 shape: 512 mic samples at
  48 kHz into WDSP, 2048-sample DSP blocks at 96 kHz, explicit `TXASetNC(2048)`,
  and 192 kHz DUC IQ output to P2_app.
- PTT-off is authoritative: the bridge stops accepting mic frames for that TX
  request, clears desired TX state, and sends repeated RX high-priority packets
  before completing TX teardown.
- Browser release handling is also fail-closed: pointer/key release, page hide,
  and window blur all send `trx:false` and stop local mic capture even if the UI
  already appears to be in RX.
- RF TX is enabled by default in the bridge environment for Saturn Remote
  operation. Operators can set `SATURN_REMOTE_TX_RF_ENABLED=0` to disable RF TX
  without changing the browser, auth, or Tailscale controls.

This is the right first-step contract because it keeps the frontend close to the actual radio stream instead of hiding complexity too early.

## LAN vs WAN

### LAN / same-site first

Current approach:

- raw IQ to the browser
- browser-side FFT
- future browser-side or backend-fed audio

This is acceptable for same-LAN experimentation and feature bring-up.

### WAN / internet later

Do not assume raw IQ remains the default transport.

Expected direction:

- FFT row transport for panadapter/waterfall
- compressed receive audio
- explicit TX session ownership
- supervised split-lane reconnection with bounded exponential backoff, browser
  online/offline handling, bridge-ready gating, and RX-only session replay
- authenticated and proxied WebSocket path

That keeps the browser UI responsive and cuts bandwidth substantially.

## Why WebGL2 First

Use `WebGL2` as the production baseline for now.

Reasons:

- broad browser support
- stable enough for appliance-style deployment
- enough capability for spectrum and waterfall work

`WebGPU` can remain a later upgrade path once the page, bridge, and data model are mature.

## Planned Evolution

1. Current page:
   - DOM controls
   - WebGL2 display
   - raw IQ ingest from `saturn-bridge`
2. Add WDSP RX audio in `saturn-bridge`
3. Add TX path with WDSP TXA -> DUC IQ
4. Add remote-authenticated bridge proxying through Saturn Go / nginx
5. Add multi-client roles and arbitration in `saturn-bridge`
