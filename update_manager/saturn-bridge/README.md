# Saturn Bridge

`saturn-bridge` is the first standalone backend for direct G2 remote operation.

Current scope:

- owns one downstream Protocol 2 session to `p2app`
- keeps a canonical `radio_model` with desired vs observed state
- sends Saturn/Thetis-style startup traffic to `p2app`
- ingests RX-only DDC IQ and high-priority status from `p2app`
- exposes a first TCI WebSocket frontend for RX-only remote control/panadapter
- has a first browser frontend path in `update_manager/templates/saturn-remote.html`

This is the `v2` architecture boundary, even though the current feature set is
still intentionally small.

## Why It Is Separate

`p2app` remains the radio transport/runtime engine.

`saturn-bridge` is where these responsibilities now belong:

- session ownership
- bridge protocol adapters like TCI
- WDSP-based RX/TX DSP
- multi-client arbitration
- remote-specific telemetry and policy

That keeps the real-time SDR engine stable while giving Saturn a clean place to
grow browser remote control, native clients, and role-based access later.

## Same-Host P2 Model

The bridge is designed to run on the same Pi as `p2app`.

Important detail:

- the bridge binds one local UDP port, default `127.0.0.1:12000`
- discovery/general traffic is sent to `p2app` on `127.0.0.1:1024`
- `p2app` replies and streams RX data back to the bridge's source port
- the bridge demuxes incoming traffic by the SDR source ports:
  - `1025` high-priority from SDR
  - `1035+` DDC IQ streams

That avoids changing `p2app` while still allowing the bridge to act like a
normal Protocol 2 client.

## Current Wire Setup

Startup packets currently sent by the bridge:

- discovery request on `1024`
- general packet on `1024`
- DDC specific on `1025`
- DUC specific on `1026`
- periodic high-priority to SDR on `1027`

Current RX ingest:

- high-priority from SDR on `1025`
- DDC0 IQ from SDR on `1035`

Current TCI frontend:

- binds `127.0.0.1:50001` by default
- single-client for now
- supports:
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
- sends IQ binary frames in a TCI-style 64-byte header + float32 I/Q payload

## Build

```bash
cargo build --manifest-path /home/pi/github/Saturn/update_manager/saturn-bridge/Cargo.toml
```

## Run

```bash
cargo run --manifest-path /home/pi/github/Saturn/update_manager/saturn-bridge/Cargo.toml
```

Useful environment variables:

- `SATURN_BRIDGE_RADIO_HOST`
- `SATURN_BRIDGE_RADIO_PORT`
- `SATURN_BRIDGE_CLIENT_HOST`
- `SATURN_BRIDGE_CLIENT_PORT`
- `SATURN_BRIDGE_TCI_HOST`
- `SATURN_BRIDGE_TCI_PORT`
- `SATURN_BRIDGE_ENABLE_DISCOVERY`
- `SATURN_BRIDGE_HP_PERIOD_MS`
- `SATURN_BRIDGE_DDC0_FREQUENCY_HZ`
- `SATURN_BRIDGE_DDC0_SAMPLE_RATE_KHZ`
- `SATURN_BRIDGE_DDC0_SAMPLE_SIZE_BITS`

## Next Steps

1. WDSP RX audio path
2. TX path with WDSP TXA -> DUC IQ
3. multi-client roles and arbitration
4. proxied/authenticated remote deployment through Saturn Go and nginx
