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

Bridge side, from `perf.json`:

| Field | Meaning |
|---|---|
| `rx_opus_overflow_samples_total` | Encoder-feed samples dropped because the 80 ms queue was full |
| `rx_opus_contention_drops_total` | Encoder-feed offers skipped because the RX callback lost the queue lock |
| audio drop/queue counters | Existing per-client audio queue health |

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
