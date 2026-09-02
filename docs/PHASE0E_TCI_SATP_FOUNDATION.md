# Phase 0E — TCI + SATP Native Audio Foundation

## Boundary

**VERIFIED CURRENT BEHAVIOR:** Saturn Bridge is the Rust radio-facing process.
It owns the TCI server, authoritative `RadioModel`, WDSP lifecycle, TX safety,
and the selected P2 or direct-XDMA backend.

**VERIFIED CURRENT BEHAVIOR:** Saturn Go is the Rust management/security web
service. It proxies Saturn Remote's authenticated WebSocket lanes and exposes
configuration and diagnostics.

**INTENDED DESIGN:** TCI remains the control plane. SATP/UDP is a separate PCM
media plane. SATP does not change frequency, mode, RX state, PTT, or MOX.

**VERIFIED CURRENT BEHAVIOR:** The Phase 0E SATP sink is
`NullTxAudioSink`. No SATP sample reaches WDSP, DUC, XDMA, P2, FPGA, or RF.

```text
Windows native audio bridge
          |
          | SATP v1 / UDP
          v
Saturn Bridge receiver -> validator -> fixed packet ring -> playout clock
                                                        -> NullTxAudioSink

TCI clients -> TCI 2.0 WebSocket -> RadioModel -> existing radio control
```

## SATP v1 wire contract

**VERIFIED CURRENT BEHAVIOR:** The receiver accepts exactly one initial stream
format:

| Field | Value |
|---|---|
| Magic | `SAT1` |
| Version | 1 |
| Packet type | 1 (`TX_AUDIO`) |
| Sample rate | 48,000 Hz (session contract) |
| Sample format | 1 (`FLOAT32_LE`) |
| Channels | 1 |
| Frames per packet | 128 |
| Datagram bytes | 544 |

The 32-byte header is:

| Offset | Bytes | Field |
|---:|---:|---|
| 0 | 4 | magic |
| 4 | 1 | version |
| 5 | 1 | packet type |
| 6 | 1 | channels |
| 7 | 1 | sample format |
| 8 | 4 | session ID, little-endian |
| 12 | 4 | stream ID, little-endian |
| 16 | 4 | sequence, little-endian |
| 20 | 8 | sample counter, little-endian |
| 28 | 2 | frame count, little-endian |
| 30 | 2 | flags, little-endian |

The payload is 128 little-endian finite `float32` mono samples. Incompatible,
misaligned, malformed, or non-finite packets are rejected and counted.

## Receiver and playout

**VERIFIED CURRENT BEHAVIOR:** The UDP socket requests a 1 MiB receive buffer
and reports the actual kernel value. It is not assumed to have been granted.

**VERIFIED CURRENT BEHAVIOR:** The default jitter target is 512 frames and the
fixed capacity is 4096 frames. Both values are packet-aligned and bounded. No
unbounded channel, queue, or slice growth is used for buffered audio.

**VERIFIED CURRENT BEHAVIOR:** Packets are placed by `sample_counter`, not UDP
arrival time. The steady playout cadence is 128 / 48,000 seconds. A packet that
arrives before its cursor is played may be reordered. A packet behind the
cursor is late and dropped.

**VERIFIED CURRENT BEHAVIOR:** When a playout position is absent, exactly 128
silence frames are produced, `packets_missing` is incremented, and the cursor
continues forward. `gap_events` records discontinuities observed in arrival
sequence separately from missing playout packets. `playout_gap_events` records
the start of each contiguous silence-insertion run, so internal playout gaps
cannot be mistaken for network sequence gaps.

**VERIFIED CURRENT BEHAVIOR:** The playout clock is unarmed until the bounded
ring reaches its target. It then holds the complete target duration before
steady playout begins, keeping the four-packet Windows burst around the target
instead of racing it at an empty ring. Session changes, PTT rising edges, and
audio-loss recovery reset that clock; it is re-anchored only after a fresh
target is available. This prevents an old wall-clock deadline from racing the
timeline forward and classifying current packets as late after a sender pause.

**VERIFIED CURRENT BEHAVIOR:** A new `session_id` flushes the packet ring and
resets sequence, timeline, and playout state without treating the sender epoch
change as packet loss.

## PTT boundary

**VERIFIED CURRENT BEHAVIOR:** SATP receives continuously while the bridge is
in RX or TX. The null sink is written only while the existing authoritative
radio model indicates TX intent/armed/keyed state.

**VERIFIED CURRENT BEHAVIOR:** A TX rising edge flushes buffered pre-PTT audio
and waits for a fresh target of current packets. A TX falling edge immediately
stops null-sink delivery while reception continues.

**TECHNICAL DEBT:** Audio health is reported as healthy below 50 ms, degraded
from 50 ms through the configured timeout (250 ms default), and lost after the
timeout. Phase 0E does not request dekey on SATP loss because SATP is not yet a
selectable live TX source. Doing so now could incorrectly dekey an unrelated
browser-microphone transmission. Phase 0F must add explicit audio-source
selection, then route a loss request through the normal TX control path.

## Configuration and status

**VERIFIED CURRENT BEHAVIOR:** Saturn Go exposes:

```text
GET/PUT /api/v1/tci/config
GET     /api/v1/tci/status
GET/PUT /api/v1/satp/config
GET     /api/v1/satp/status
```

The legacy `/tci_status`, `/tci_settings`, `/satp_status`, and
`/satp_settings` routes remain available to the appliance UI.

**VERIFIED CURRENT BEHAVIOR:** Configuration is validated twice, written as an
atomic systemd drop-in, and rolled back if an active bridge fails to restart.
An inactive bridge remains inactive, preserving P2-first clean-boot ownership.

**VERIFIED CURRENT BEHAVIOR:** Runtime status is atomically published at
`/run/saturn-bridge/satp-status.json` and surfaced by Saturn Go. It includes
source/session identity, packet and frame counts, sequence/timeline counters,
buffer occupancy, silence insertion, packet/effective rate, health, socket
buffer size, and null-sink delivery counts.

## Exit gate and next phase

**PROPOSED CHANGE:** Complete a 30-minute synthetic sender soak and a 30-minute
real Windows/Voicemeeter/Saturn Native Audio Bridge soak. Acceptance requires
bounded occupancy and stable RSS/CPU with diagnostics visible in Saturn Go.

The repository synthetic sender reproduces the Windows callback pattern (four
back-to-back 128-frame packets every 10.667 ms):

```bash
python3 update_manager/scripts/satp-synthetic-sender.py \
  192.168.0.139 --port 50100 --seconds 1800
```

**PROPOSED CHANGE:** Only after those results are reviewed may Phase 0F add an
`XDMATxAudioSink`, first to a non-RF diagnostic path. Live DUC/RF connection is
explicitly outside Phase 0E.
