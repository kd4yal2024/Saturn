# Saturn Bridge Direct XDMA

## Goal

Saturn Bridge will support two appliance-wide radio backends:

- `p2`: the stable Protocol 2 path through `p2app.service`
- `xdma`: direct FPGA register and DMA access modelled on piHPSDR

Protocol 2 remains the default until the direct backend passes the complete
receive, audio, transmit-safety, recovery, and soak-test gates.

Backend ownership is global to the appliance. It is not selected independently
for each browser connection because P2_app and Saturn Bridge must never drive
the same FPGA registers or DMA channels concurrently.

## Operator Switching Contract

Saturn Remote will ultimately expose the backend selector to its operator.
Changing it must be a transactional control-plane operation:

1. Force RX and disable RF output.
2. Disconnect and release the active backend.
3. Stop `p2app.service` when moving to XDMA, or release XDMA before starting P2.
4. Claim and validate the requested backend.
5. Re-bootstrap any connected client against the new backend.
6. Start its RX path and verify data-plane health.
7. Persist the selection only after readiness succeeds.
8. Automatically restore the previous backend if any step fails.

Service state alone is not backend readiness. With a client connected, the P2
backend must complete discovery and show advancing high-priority and DDC packet
counters. The direct XDMA backend will require advancing DMA/FIFO counters.
With no client connected, the selected backend may remain available and idle;
the first client connection must complete the same validation before the UI
reports it active.

The UI must show the active backend separately from the requested backend and
must not represent Phase 1 probing as an operational radio connection.

## Phase 1: Identity and Safe Lifecycle

Phase 1 adds a one-shot probe to the installed `saturn-bridge` binary:

```bash
sudo systemctl stop p2app.service
sudo -u pi /opt/saturn-go/bin/saturn-bridge --xdma-probe
sudo systemctl start p2app.service
```

The probe:

- refuses to run while `p2app.service` is active
- opens `/dev/xdma0_user` through the `saturn-radio` group policy
- verifies Saturn product ID, golden/primary image ID, clock status, and FPGA
  firmware major version
- preserves unrelated register bits while forcing MOX, TX enable, PA relay,
  CW keyer, TX watchdog override, and DUC streaming into their safe state
- repeats the safe-state operation during cleanup

`SATURN_BRIDGE_XDMA_USER_DEVICE` may override the register device path for
fixture testing. It is not an operational backend-selection setting.

Phase 1 does not open C2H/H2C DMA streams and cannot receive or transmit.

## Phase 2: RX-only DDC IQ

Phase 2 adds a one-shot, RX-only capture:

```bash
sudo systemctl stop p2app.service
sudo -u pi /opt/saturn-go/bin/saturn-bridge --xdma-rx-probe
sudo systemctl start p2app.service
```

The capture:

- refuses to run while `p2app.service` is active
- keeps MOX, TX enable, PA relay, CW keyer, and DUC streaming disabled
- configures hardware DDC6 for ADC1 at 192 kHz
- reads page-aligned DMA blocks from `/dev/xdma0_c2h_0`
- validates every 64-bit rate header and packed 24-bit I/Q frame
- reports a synthesized frame sequence, observed sample rate, DMA throughput,
  FIFO high-water/error counters, header resynchronization, RMS, and peak
- disables DDC streaming, clears the rate word, and resets the FIFO on every
  normal or error exit

The following test-only environment settings are supported:

- `SATURN_BRIDGE_XDMA_RX_FREQUENCY_HZ` (default `14200000`)
- `SATURN_BRIDGE_XDMA_RX_DURATION_MS` (default `2000`, range `250..10000`)
- `SATURN_BRIDGE_XDMA_RX_DEVICE` (default `/dev/xdma0_c2h_0`)

Phase 2 does not send the captured samples to clients. P2 remains the only
operational backend while the direct data path is validated in isolation.

## Phase 3: Codec microphone and speaker DMA

Phase 3 adds a one-shot codec-audio probe:

```bash
sudo systemctl stop p2app.service
sudo -u pi /opt/saturn-go/bin/saturn-bridge --xdma-audio-probe
sudo systemctl start p2app.service
```

The probe:

- refuses to run while `p2app.service` is active
- captures 48 kHz signed 16-bit microphone samples from
  `/dev/xdma0_c2h_1` at AXI offset `0x40000`
- honors the FPGA's current local/network byte-order setting
- writes only zero-valued 48 kHz stereo speaker frames to
  `/dev/xdma0_h2c_1` at AXI offset `0x40000`
- asserts the hardware speaker mute before opening the audio path and leaves
  it asserted during cleanup
- verifies that the speaker FIFO accepts the DMA write and begins draining
- reports microphone level/rate and per-stream DMA/FIFO telemetry
- resets both codec FIFOs and preserves the Phase 1 receive-safe RF state on
  every normal or error exit

The following test-only environment settings are supported:

- `SATURN_BRIDGE_XDMA_AUDIO_DURATION_MS` (default `2000`, range `250..10000`)
- `SATURN_BRIDGE_XDMA_MIC_DEVICE` (default `/dev/xdma0_c2h_1`)
- `SATURN_BRIDGE_XDMA_SPEAKER_DEVICE` (default `/dev/xdma0_h2c_1`)

Phase 3 does not route codec audio to clients and does not enable DUC or RF
transmit. P2 remains the only operational client backend.

## Planned Data Paths

| Function | XDMA node |
| --- | --- |
| FPGA registers | `/dev/xdma0_user` |
| DDC receive IQ | `/dev/xdma0_c2h_0` |
| Microphone input | `/dev/xdma0_c2h_1` |
| DUC transmit IQ | `/dev/xdma0_h2c_0` |
| Speaker output | `/dev/xdma0_h2c_1` |

## Migration Gates

1. Phase 1: identity, compatibility, ownership, and safe shutdown
2. RX-only DDC IQ with FIFO and framing telemetry
3. Microphone and speaker DMA
4. DUC IQ with RF forcibly disabled
5. Guarded RF transmit and failure-injection tests
6. Transactional client-selectable switching and rollback
7. Long-duration soak testing before considering XDMA as the default
