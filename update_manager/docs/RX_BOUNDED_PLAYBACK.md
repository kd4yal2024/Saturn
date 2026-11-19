# Bounded RX playback reserve

The default RX AudioWorklet now waits for 48 ms of browser-rate PCM before
playing. It also re-primes after a real shortfall, increasing the reserve by
16 ms up to 80 ms. After 30 seconds of uninterrupted playback the target falls
4 ms, then another 4 ms per 10 stable seconds, to the 48 ms floor.

This reserve handles packet-delivery bursts independently of the existing
slow clock-drift correction. The correction controller follows the current
reserve target without resetting its ratio, and queue observations are withheld
while the worklet is priming or rebuffering.

The normal postMessage path uses a fixed stereo ring of at most 120 ms at the
browser's actual sample rate. A burst beyond capacity discards the oldest
samples and reports the trimmed frame count. Packet sizes cannot grow storage
or create an unbounded object queue. At 48 kHz the ring is 46,080 bytes. This
adds one bounded copy of each delivered PCM block into the worklet's fixed
storage. The optional SAB path uses its existing smaller ring (about 85 ms at
48 kHz); its capacity bounds the available reserve.

The cap describes locally queued decoded audio, not end-to-end latency or the
age of a packet delayed elsewhere. Codec framing, decoder delay, browser output
latency, TCP and VPN delays still contribute. No finite reserve can hide a long
outage; a real shortfall is reported and followed by a bounded re-prime.

Intentional flushes clear queued audio and reset priming without inventing a
network underrun. Their reason remains in the recorder. Startup/re-prime holds
with data available are reported separately from missing PCM. Gain, mute,
stereo ordering, source PCM, codec selection, WAN/LAN quality, TX/PTT ownership,
WDSP, acquisition, display transport and RF processing are unchanged. The older
BufferSource fallback retains its existing scheduling behavior.

## Operation and diagnostics

The feature is enabled by default. **Settings → Network → RX playback buffer**
shows phase, target, current queue, local capacity and recovery count. **Copy
Network Diagnostics** includes the reserve, intentional hold time, trims and
existing missing-audio recorder. Performance snapshots retain a separate copy
of the current buffer telemetry.

Append `rx_playback_buffer=off` to the existing page query for an explicit
immediate-playback comparison; preserve the other URL parameters. Reload to
apply it. Both comparison paths retain the fixed queue cap. No preference or
per-band memory is rewritten by this URL override.

## Repeatable tests

```sh
cd update_manager/remote-web
npm run typecheck
npm run check:seam
npm test
npm run build
npm run validate:rx-playback -- --output=/tmp/saturn-rx-playback
```

The virtual-clock tests execute the shipped processor and cover paired bursts,
a longer outage, adaptation/decay, an oversize burst, stereo sample retention,
intentional flushes, alternate browser rates, SAB and telemetry bounds.

The actual-page Chrome fixture starts the real RX path with 48 kHz stereo PCM
and a worker delivering 20 ms packets in pairs at 40 ms boundaries. It runs
immediate/buffered and buffered/immediate arms, records the actual worklet
telemetry and verifies Network readout/export integration. It opens no real
WebSocket, microphone or radio. The first recorded run had one immediate-path
underrun per arm and zero buffered-path underruns, with no overflows. Those
numbers describe that synthetic schedule, not a VPN qualification.

For the office comparison, keep the same band, signal, DSP and display settings.
Copy Network Diagnostics shortly after a stutter, or after a clean listening
window. Its current reserve, recovery count, packet spacing, worklet delivery
history, trims and transport backlogs help identify the next bottleneck. Browser
arrival times include main-thread scheduling; they do not isolate network jitter.
