# Saturn Remote Architecture

## Purpose

`saturn-remote` is the browser frontend for the `saturn-bridge` backend.

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
- `saturn-bridge` drains pending websocket frames in bounded batches and stops
  WDSP TX without waiting for slew-down, allowing the high-priority RX recovery
  packet to go out immediately.

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
- reconnect policy and buffering
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
