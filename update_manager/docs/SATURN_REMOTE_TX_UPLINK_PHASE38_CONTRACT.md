# Saturn Remote Phase 38 TX Uplink Contract

Created: 2026-05-19
Implemented: 2026-05-19

Scope: ratified contract and implementation notes for the first slow-VPN TX
uplink slice. This phase is guard, telemetry, and fail-closed visibility only. It
does not change mic payload format, sample rate, Opus, WDSP setup, or the
existing bridge RF-keying safety predicate.

## Incident Context

Slow VPN links stress the opposite direction from Phase 35. During TX, the
browser sends mic frames upstream to the bridge. If those mic frames build a
TCP backlog, PTT release and MOX-off can sit behind old audio. That failure
mode is worse than dropping audio because it can put delayed speech on the air.

The current bridge safety model already has the important backend guard:
`can_key_rf()` requires keyable IQ plus recent mic audio. The TX uplink phase
adds earlier browser-side congestion detection, explicit mic-frame sequence
telemetry, bridge-authoritative `uplink_late` faults, and operator visibility.

## Core Invariant

Bulk media is bounded and droppable. Safety/control is tiny and must not sit
behind media.

Phase 35 applies this invariant to bridge-to-browser display/audio downlink.
Phase 38 TX-A applies the same invariant to browser-to-bridge mic uplink.

The browser must decide whether to send or drop a mic frame before calling
`WebSocket.send()`. Once `send()` is called, the bytes have been handed toward
TCP and cannot be canceled without closing the connection.

## Authority Boundary

The browser is a leading congestion sensor. It watches
`WebSocket.bufferedAmount`, reports degraded uplink state, and drops mic frames
before they enter TCP when needed.

The bridge is authoritative for TX safety. It owns `can_key_rf()`, detects
late/stale mic arrival from sequence gaps and mic age, forces RX on safety
failure, and publishes the hard fault:

`tx_fault:0,uplink_late,<age_ms>,<limit_ms>;`

Browser degradation is warning-level. Bridge `tx_fault:uplink_late` is
safety-level and immediate.

## Scope Clamp

TX-A safety enforcement applies only while the client is actually keyed/on-air:
MOX requested, PTT keyed, or bridge TX active.

TX-ready or TX-armed without RF is UI state only. A bad uplink in armed/ready
state may show a degraded indicator, but it must not force RX, publish
`tx_fault:uplink_late`, or auto-unkey.

## Provisional Thresholds

These numbers are provisional so implementation and regression tests have
pass/fail bars. Real VPN measurement should revise them later.

- Browser `tx_uplink_degraded` flips on when
  `bufferedAmount > 2 * (RTT * mic_byte_rate)`.
- Browser preemptive mic drop starts when
  `bufferedAmount > 4 * (RTT * mic_byte_rate)`.
- Bridge `tx_fault:uplink_late` fires when
  `mic_age_ms > 250` is sustained for `100 ms`.
- Auto-unkey latency target: bridge-to-wire force-RX must happen within one
  mic-frame interval, `<= 20 ms`, after breach detection.

`RTT` should use the fresh Phase 22 bridge websocket RTT when available.
`mic_byte_rate` should be computed from the active mic transport format.

## Browser Behavior

Each TX mic frame carries a monotonically increasing `tx_mic_seq`.

Before each mic-frame send, the browser checks `WebSocket.bufferedAmount`:

- Below degraded threshold: send the mic frame.
- Above degraded threshold: publish/record degraded state; keep sending until
  the drop threshold is crossed.
- Above drop threshold: do not call `WebSocket.send()` for that mic frame;
  increment `tx_mic_dropped_count`.

PTT release, MOX-off, and other TX safety/control commands must not be placed
behind queued mic data. Dropping mic frames before `send()` is the browser-side
mechanism that preserves that invariant.

## Browser Telemetry

Browser-emitted counters ride the control class at 1 Hz with delta values.
`tx_uplink_degraded` also publishes immediately when it changes state.

Required browser fields:

- `tx_uplink_degraded`
- `tx_mic_seq`
- `tx_mic_dropped_count`
- `tx_uplink_buffered_bytes`
- `tx_uplink_buffered_hwm_bytes`

Telemetry must not be emitted per mic frame. Under congested TX, telemetry
must not become its own backpressure source.

## Bridge Behavior

The bridge records the last arrived mic sequence and detects sequence gaps.
It also tracks current mic age from the last accepted mic frame arrival.

Required bridge-side counters:

- `tx_mic_seq_gap_count`
- `tx_mic_age_ms_current`
- `tx_mic_last_arrived_seq`
- Browser-reported `tx_mic_dropped_count`
- Browser-reported `tx_uplink_buffered_bytes`
- Browser-reported `tx_uplink_buffered_hwm_bytes`

Bridge diagnostics and the Bridge Diagnostics tab should expose both browser
and bridge sides so a VPN TX session can correlate "browser dropped early" vs.
"bridge observed late mic".

`tx_fault:0,uplink_late,<age_ms>,<limit_ms>;` is immediate safety telemetry,
not part of the 1 Hz heartbeat.

## Implemented TX-A Runtime

The deployed TX-A runtime keeps the browser mic payload as Float32 at 48 kHz
but writes `tx_mic_seq` into the existing binary header at bytes `32..36`.

The browser:

- Computes RTT-scaled bufferedAmount thresholds from the current bridge RTT,
  falling back to `200 ms` when no fresh RTT exists.
- Marks `tx_uplink_degraded` above the `2x RTT` threshold.
- Drops mic frames before `WebSocket.send()` above the `4x RTT` threshold.
- Sends `tx_uplink_stats:0,<degraded>,<last_seq>,<dropped_count>,<buffered_bytes>,<buffered_hwm_bytes>;`
  at 1 Hz and immediately when degraded state flips.

The bridge:

- Tracks browser-reported dropped mic count and uplink buffer watermarks.
- Tracks bridge-arrived mic sequence, mic sequence gaps, and current mic age.
- Publishes `remote_tx_uplink:0,<degraded>,<browser_drops>,<buffered>,<hwm>,<last_arrived_seq>,<seq_gaps>,<mic_age_ms>;`
  at the same 1 Hz diagnostics cadence as Phase 35.
- Publishes `tx_fault:0,uplink_late,<age_ms>,250;` and forces RX only while
  the radio is actually keyed/on-air and mic age has exceeded `250 ms` for
  `100 ms`.

The `/remote-next` UI parses both messages. Operator State shows TX uplink
state, browser mic drops, bridge mic gaps, and mic age. Existing bridge-fault
handling records an alarm and locks TX when `uplink_late` arrives.

Post-review cleanup:

- Bridge TX mic-age telemetry and the bridge safety detector use the same
  parser-stamped receive timestamp for each mic frame.
- `uplink_late` bridge faults remove the earlier browser-side `TX audio late`
  warning from the page-session fault history before recording the alarm fault.
- `txMicByteRate()` carries the TX-B note that changing Float32 mic frames to
  `s16@48k` changes only the bytes-per-sample parameter.

## Required Regression Tests

TX-A is not complete without tests for these cases:

- Simulated congested VPN at `200 ms` RTT and about `30 kB/s` budget:
  browser drops mic frames, `tx_mic_dropped_count` climbs, and the bridge does
  not fire `uplink_late` because early drops keep the stream safe.
- Sustained complete uplink stall mid-TX: bridge fires `uplink_late` within
  the configured limit, forces RX, locks TX, and the fault appears in the
  operator state strip or Fault pill.
- Armed-but-not-keyed with bad uplink: no force-RX, no `uplink_late`, and no
  auto-unkey; only degraded UI/telemetry is allowed.
- Decide-before-send: a mocked browser WebSocket proves no `send()` call is
  made for mic frames while `bufferedAmount` is above the drop threshold.
- Reconnect or client disconnect mid-TX: existing fail-closed force-RX path is
  unchanged.
- TX start: the first mic frame after MOX/PTT arms must not falsely trip
  `uplink_late` from the initial mic-age window.

## Acceptance Test

Under simulated VPN congestion at `200 ms` RTT and `30 kB/s` with TX keyed at
48 kHz Float32 mic, the operator must observe one of two outcomes:

1. Clean enough speech with `tx_mic_dropped_count` climbing, proving the
   browser is dropping before TCP backlog becomes dangerous.
2. Force-RX within `<= 350 ms` of a true mic stall, calculated as `250 ms`
   detection plus `100 ms` breach window, with
   `tx_fault:uplink_late` visible.

A multi-second delayed transmission is the explicit failure mode. If any test
produces delayed on-air speech, TX-A is not done.

## Later Phases

TX-B: switch mic payload from `Float32@48k` to `s16@48k`, with no sample-rate
change and no browser/bridge resampler. This should roughly halve mic uplink
bandwidth while keeping the signal path simple.

TX-C: add Opus only if measured VPN TX sessions still show real
`tx_fault:uplink_late` events after TX-B. Do not commit to Opus
speculatively; let measurements decide.
