# RX audio A/B: PCM versus Opus over the WS media lane

Phase 0 of the RX audio work adds shared Opus encoders in the bridge and a
WebCodecs decoder in the browser. It also moves RX volume authority to the
browser (`audio_gain:client`) and makes the bridge emit neutral WDSP audio, which
fixes the long-standing double-gain bug. Both changes must be measured on the
same signal and the same link before deciding whether WebRTC RX (Phase 1) is
needed.

## Arms

| Arm | URL | Codec | Volume authority | Expected display bandwidth |
|---|---|---|---|---|
| PCM (control) | `remote-next?rx_audio_codec=pcm` | raw f32, type 1 | browser | 12 kHz mono ≈ 384 kbit/s payload |
| Opus (auto) | `remote-next` (default) | Opus, type 17 | browser | 24 kbit/s mono, 80 kbit/s stereo payload |

`?rx_audio_codec=pcm` is the only knob that differs. It keeps client-side gain,
so both arms are the same loudness and the comparison is codec plus transport
profile, not volume.

## Preconditions

1. Bridge and browser both from the Phase 0 change set (uncommitted today).
2. The browser must report `"capability": "supported"`. If it reports
   `"WebCodecs unavailable"`, that browser has no WebCodecs Opus decoder (older
   iOS Safari) and only the PCM arm is meaningful there — record it as a
   capability-matrix data point rather than a failure.
3. Record the link (LAN / Tailscale / public WAN), the browser and version, the
   band and mode (include WBFM; it is a separate gain path), and the volume
   slider position.

## Procedure

For each arm, on the same signal and link:

1. Open the URL, connect, press **Start Audio**.
2. Confirm the browser is actually on the intended codec: **Network → Copy RX
   latency diagnostic** and read the `RX codec A/B` block, or
   `window.SaturnRemotePerf.snapshot().rxAudioCodec` in the console.
   Check `accepted` is `opus` for the auto arm and `pcm` for the control arm.
3. Soak for at least 5 minutes with audio actually playing. Talk through a
   known signal, and include a tuning sweep and one WBFM segment.
4. Note audible breakup/dropouts by ear, with a timestamp.
5. Capture the diagnostic block at the end of the soak.
6. On the bridge, capture `perf.json` for the same window (the bridge owns the
   encoder-side drop counters).
7. Repeat for the other arm on the same signal and link, then reload once with
   the other codec to confirm the switch is clean and recovery works.
8. Finish with one recovery check per arm: kill and restore the radio-side
   link (or toggle Wi-Fi), and record time-to-audio and any residual artifacts.

## What to record

Browser side, from the `RX codec A/B` diagnostic block:

| Field | Meaning |
|---|---|
| `accepted` / `capability` | Which codec is live, and whether WebCodecs was available |
| `clientGainEcho` | Bridge confirmed `audio_gain:client` (volume is applied once) |
| `fallbackReason` | Why Opus was abandoned, if it was |
| `pcmBytesPerSec` / `opusBytesPerSec` | Measured receive rate for the live codec |
| `pcmFrames` / `opusFrames` | Frames received per codec; with the byte totals this gives bytes/frame |
| `decodedFrames` / `decoderErrors` / `lateDrops` | Opus decode health |
| `malformedFrames` / `malformedRun` | Wire-contract mismatches (one is a dropped frame; a run falls back to PCM) |
| `resyncs` | Decoder stalls recovered by skip-and-resume |
| `sequenceGaps` / `missingPackets` | Media-lane loss as seen by the browser |
| `jitterP99Ms` | Packet-arrival jitter percentiles |
| `workletUnderruns` / `workletOverflows` | Playback ring-buffer health |
| `queueMs` | Playback lead in ms |

Bridge side:

| Field | Where it comes from | Meaning |
|---|---|---|
| `rx_opus_ingress:0,<overflow>,<contention>;` | Live text line pushed to connected clients — **not** in `perf.json` | Encoder-feed samples dropped because the 80 ms queue was full, and offers skipped because the RX callback lost the queue lock |
| `rx_audio_frames_s`, `rx_audio_samples_s`, `audio_dropped_s` | `perf.json` | Aggregate WDSP audio rate and per-client queue drops |
| `rx_fifo_*`, `host_buffer_drops`, `header_errors` | `perf.json` | Data-plane health, to separate a radio problem from an audio-path problem |

The per-client `rx_opus_*` totals are only ever sent as that text line; they were
never wired into `perf.json`. The browser receives the line but does not yet
surface it in the diagnostics snapshot, so for a manual capture read it from the
client's WebSocket trace, or add it to the snapshot in a follow-up change.

Latency and recovery are browser-side observations: `queueMs` plus
`audioContextBaseLatencyMs` for the playback budget, and the recovery check for
time-to-audio after a link change. The browser's Opus path has no jitter buffer;
packets are decoded and played as they arrive, exactly like PCM, so any latency
change between the arms should be small.

## Automated arms

```bash
cd update_manager/remote-web
npm run build
npm run validate:rx-audio-ab        # CHROMIUM=/path/to/chrome if needed
```

The harness loads the real template and bundle in headless Chrome against a stub
bridge and asserts the negotiation order, the gain-echo gate, the WebCodecs
capability fallback, the malformed-run policy, the decoder-backlog resync, and
the on-wire bytes per arm, for both the WAN mono and LAN stereo profiles. It
cannot judge audio quality — that is the live run.

Latest local numbers (headless Chrome, real WebCodecs probe present, stub bridge):

| Measurement | Value |
|---|---|
| PCM, WAN 12 kHz mono f32 | 1024 B per 20 ms frame on the wire (384 kbit/s payload) |
| Opus, WAN mono | 124 B per 20 ms frame on the wire (24 kbit/s payload) → 49.6 kbit/s on the wire |
| PCM, LAN 48 kHz stereo f32 | 7744 B per 20 ms frame on the wire (3.07 Mbit/s payload) |
| Opus, LAN stereo | 264 B per 20 ms frame on the wire (80 kbit/s payload) → 105.6 kbit/s on the wire |
| Measured on-wire reduction | WAN 8.26×, LAN 29.33× |

Note the difference between payload and on-wire figures: the 64-byte TCI header
is re-sent on every 20 ms packet, so a 60-byte mono Opus payload costs 124 bytes
on the wire and a 200-byte stereo payload costs 264. The advertised 16×/38× are
against payload only; on the wire they are about 8.3× and 29.3×. This matters for
the Phase 1 decision, because the mono WAN stream is ~50 kbit/s, not ~24 kbit/s.

## Live probe without the authenticated proxy

The bridge's TCI server listens on loopback, so the real browser code can be
driven against the real bridge, real libopus and the real WebCodecs decoder with
no proxy session and no radio reconfiguration:

```bash
ssh -N -L 127.0.0.1:15001:127.0.0.1:50001 pi@<radio> &
npm run build
node scripts/live-rx-opus-probe.mjs --ws ws://127.0.0.1:15001/ --mode lan --seconds 20
node scripts/live-rx-opus-probe.mjs --ws ws://127.0.0.1:15001/ --mode wan --seconds 20
```

It loads the real template and bundle in headless Chrome, disables split
transport (the loopback TCI is a single lane), starts RX audio, and reports both
the browser's `rxAudioCodec` snapshot and a trace of every WebCodecs decode
timestamp versus every output timestamp. It listens only: RX audio and display,
no keying, no state changes. It does add one RX audio listener while it runs.

## Live findings that the stub harness could not see

**Chrome's Opus decoder does not always echo `EncodedAudioChunk.timestamp` on the
output `AudioData`.** Measured on the live LAN profile (20 s, ~1050 packets):
1045 outputs for 1046 decodes, and 42 input timestamps that never appeared on any
output — the decoder renumbers a subset of frames.

The first implementation keyed its pending-packet map by input timestamp and
looked it up by `data.timestamp`, so every renumbered output leaked one entry
forever. Pending grew monotonically until the backlog guard fired a resync, which
discarded real audio: on the radio this showed up as
`RX Opus resync: decoder backlog 33 packets` every ~17 seconds, each one dropping
~660 ms of audio (and 42 worklet underruns, 17 overflows, a 216 ms worst-case
output gap in a 20 s window).

Fix: pair decoder output to packets **in order** (`createRxOpusPendingQueue`),
count timestamp disagreement as `timestampRewrites` instead of breaking the map,
age out undecoded packets after 1 s, and escalate to PCM if the decoder consumes
100 packets without producing a single frame of audio. The stub harness could not
have caught this because its fake decoder echoed timestamps; it now has an arm
that deliberately renumbers them (`opus-renumbered-timestamps-still-decode`).

Same probe, same link, before and after (LAN stereo, 20 s):

| Measurement | Before | After |
|---|---|---|
| Resyncs | 1 (and repeating every ~17 s) | 0 |
| Packets stuck pending | 10 | 0 |
| Worklet underruns | 42 | 5 |
| Worklet overflows | 17 | 0 |
| Worst output gap | 216 ms | 87 ms |
| Timestamp renumbers handled | 42 leaked | 39 counted |

WAN mono, 15 s after the fix: 821 packets, 821 decoded, 0 resyncs, 0 pending,
3 underruns, 21 renumbers counted, p50 output interval 20 ms.
