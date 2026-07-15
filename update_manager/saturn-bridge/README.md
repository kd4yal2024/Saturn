# Saturn Bridge

`saturn-bridge` is the standalone backend for direct G2 remote operation.
It owns the Protocol 2 session to `p2app`, runs the WDSP DSP chain for both
RX and TX, and exposes a TCI WebSocket frontend to browser clients.

## Feature Set

### RX Chain
- Owns one downstream Protocol 2 UDP session to `p2app`
- Ingests DDC IQ streams and high-priority status packets
- Full WDSP RX DSP channel (channel 0):
  - All demodulation modes: USB, LSB, CWU, CWL, AM, SAM, FM, WFM, DIGU, DIGL
  - Full AGC: OFF, LONG, SLOW, MEDIUM, FAST with per-mode time constants
  - Noise Blanker 1 (NB1 — preemptive EXTANB): threshold-controlled impulse blanking
  - Noise Blanker 2 (NB2 — interpolating EXTNOB): zero/sample-and-hold modes
  - Noise Reduction: NR1 (ANR), NR2 (EMNR), and compatibility handling for
    NR3/NR4 when WDSP 2.00 is built without piHPSDR RNNR/SBNR extensions
  - WDSP 2.00 NR2 gain, noise-estimator, and psychoacoustic post-filter controls
  - WFM stereo at a 192 kHz DSP rate with stereo-lock and 50/75 us de-emphasis
  - Auto Notch Filter (ANF)
  - Calibrated S-meter via `GetRXAMeter(RXA_S_AV)`
  - 48 kHz stereo audio output (both channels from panel copy mode)

### TX Chain
- WDSP TX DSP channel (channel 1):
  - Bandpass filter, ALC, CFIR mic conditioning
  - Adjustable mic gain via `SetTXAPanelGain1`
  - PTT-controlled channel state with slew-down on release
  - WDSP 2.00 phase rotator with manual/automatic corner control
  - Configurable two-tone generator for controlled TX tests
  - PureSignal 3.0 synchronized TX-reference/PA-feedback calibration and
    correction with automatic feedback attenuation and fault telemetry
- DUC IQ packets: 240 samples × 6 bytes signed 24-bit big-endian (1444 bytes)
  sent to `p2app` port 1029
- Mic audio received from TCI client as binary audio frames (stream_type 1 or 2)

### TCI Frontend
- WebSocket server, default `127.0.0.1:50001`
- Split control/media sessions, explicit operator/viewer roles, and controller
  ownership arbitration
- The websocket request path pre-declares the split lane: `/control` and
  `/media` (used by the Saturn Go proxy) filter outbound traffic to that
  lane from the moment of connect, and `/media` connects skip the text state
  snapshot entirely. Any other path (e.g. `/`) keeps the legacy any-lane
  behavior; the in-band `session_lane` command still performs pairing.
- Intended browser path is the Saturn Go same-origin proxy (`/tci`); expose
  raw `:50001` only when you explicitly need direct access and trust the LAN.
- **Commands received from client:**
  - `vfo:0,{ch},{hz}` — VFO A/B frequency
  - `dds:0,{hz}` — IQ center frequency
  - `modulation:0,{mode}` — demodulation mode
  - `rx_filter_band:0,{low},{high}` — RX passband
  - `rx_volume:0,0,{db}` — RX audio gain
  - `rx_nr_mode:0,{mode}` — NR mode (OFF/NR1/NR2/NR3/NR4/ANR/EMNR/RNNR/SBNR)
  - `rx_nr:0,{bool}` — NR on/off
  - `rx_nr_level:0,{pct}` — NR level 0–100
  - `rx_nb:0,{0|1|2}` — noise blanker mode (OFF/NB1/NB2)
  - `rx_nb_threshold:0,{val}` — NB threshold
  - `rx_anf:0,{bool}` — auto notch filter on/off
  - `rx_agc:0,{mode}` — AGC mode (OFF/LONG/SLOW/MEDIUM/FAST)
  - `rx_adc:0,{0|1|2}` — ADC select (ADC1/ADC2/TXSAMPLES)
  - `rx_antenna:0,{1|2|3}` — RX antenna
  - `iq_samplerate:{hz}` — IQ sample rate
  - `iq_start` / `iq_stop` — IQ stream on/off
  - `audio_start` / `audio_stop` — audio stream on/off
  - `audio_samplerate:{hz}` — audio sample rate
  - `trx:0,{bool}` — PTT (transmit/receive)
  - `tx_drive:0,{0–255}` — TX drive level
  - `tx_mic_gain:0,{db}` — TX mic gain dB
  - Binary audio frame — mic audio for TX (stream_type 1 or 2)
- **Messages sent to client:**
  - Full radio state snapshot on connect
  - `rx_smeter:0,0,{dbm}` — S-meter (per IQ frame)
  - `tx_power:0,{w}` / `swr:0,{ratio}` — TX telemetry
  - IQ binary frames (64-byte TCI header + float32 I/Q)
  - Audio binary frames (64-byte TCI header + float32 stereo)

## Architecture

```
Browser (saturn-remote-next.html + saturn-remote-next.js)
        │  WebSocket /tci (same-origin proxy)
        ▼
saturn-go / remote TLS proxy
        │  WebSocket 127.0.0.1:50001 (TCI)
        ▼
saturn-bridge
  ├── TciFrontend    — WebSocket accept + message routing
  ├── RadioModel     — desired vs observed state
  ├── WdspRxEngine   — WDSP channel 0 RX DSP (owned by the saturn-rx thread)
  ├── WdspTxEngine   — WDSP channel 1 TX DSP (owned by the saturn-tx thread)
  └── P2Session      — UDP client to p2app

Threads: the main thread is the control plane (TCI commands, model
reconciliation, TX safety watchdogs). The saturn-rx thread blocks on the
P2 socket, runs RX DSP, and publishes IQ/audio frames directly to TCI
clients; high-priority status packets are forwarded to the main thread.
The saturn-tx thread owns the TX DSP chain and DUC IQ pacing.
        │  UDP (Protocol 2)
        ▼
p2app / radio firmware
```

## Same-Host P2 Port Map

| Traffic              | Direction        | Port  |
|----------------------|------------------|-------|
| Discovery / General  | bridge → radio   | 1024  |
| DDC specific         | bridge → radio   | 1025  |
| DUC specific         | bridge → radio   | 1026  |
| High-priority to SDR | bridge → radio   | 1027  |
| DUC IQ               | bridge → radio   | 1029  |
| High-priority from SDR | radio → bridge | 1025  |
| DDC IQ stream 0      | radio → bridge   | 1035  |
| DDC IQ stream N      | radio → bridge   | 1035+N |
| Bridge UDP bind      | local            | 12000 |

## Build

```bash
SATURN_BRIDGE_BUILD_ONLY=1 \
SATURN_BRIDGE_OUTPUT_BIN=/tmp/saturn-bridge \
bash ../scripts/install-saturn-bridge.sh
```

The installer provisions pinned sparse upstream checkouts, builds the patched
WDSP 2.00 Linux/ARM archive, verifies WFM/phase-rotator/PureSignal symbols, and
links the bridge against that archive. No cloud-init or prebuilt piHPSDR tree
is required. See `WDSP2_SPIKE.md` for source pins and overrides.

CI can run parser/control tests without the piHPSDR tree by setting
`SATURN_BRIDGE_STUB_NATIVE=1`. That mode links a local native stub and is not a
runtime or release build mode.

## Deploy

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/install_saturn_go_nginx.sh
```

Fresh installs build Saturn Go, Remote UI assets, WDSP 2.00, and Saturn Bridge
as one operation. `update-saturn-go.sh` also stages a matching bridge binary
with the web assets and backend before its detached deployment helper runs.

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `SATURN_BRIDGE_RADIO_HOST` | `127.0.0.1` | p2app host |
| `SATURN_BRIDGE_RADIO_PORT` | `1024` | p2app discovery port |
| `SATURN_BRIDGE_CLIENT_HOST` | `0.0.0.0` | local UDP bind host |
| `SATURN_BRIDGE_CLIENT_PORT` | `12000` | local UDP bind port |
| `SATURN_BRIDGE_TCI_HOST` | `127.0.0.1` | TCI WebSocket bind host |
| `SATURN_BRIDGE_TCI_PORT` | `50001` | TCI WebSocket bind port |
| `SATURN_BRIDGE_ENABLE_DISCOVERY` | `true` | send P2 discovery on start |
| `SATURN_BRIDGE_HP_PERIOD_MS` | `200` | high-priority send interval |
| `SATURN_BRIDGE_RX_DDC_INDEX` | `2` | DDC stream index (0–9) |
| `SATURN_BRIDGE_DDC0_FREQUENCY_HZ` | `14200000` | initial VFO frequency |
| `SATURN_BRIDGE_DDC0_ADC` | `0` | ADC selection (0=ADC1, 1=ADC2) |
| `SATURN_BRIDGE_DDC0_SAMPLE_RATE_KHZ` | `192` | IQ sample rate kHz |
| `SATURN_BRIDGE_DDC0_SAMPLE_SIZE_BITS` | `24` | IQ sample bit depth |

## Remaining Work

1. CESSB coordinated compressor/filter/level transition
2. TX audio monitoring / sidetone
3. Per-radio PureSignal delay calibration workflow
