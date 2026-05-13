# Saturn Remote State Strip Spec

Created: 2026-05-13

Scope: `/remote-next` operator state strip. This spec defines what the strip
shows, how it degrades across screen sizes, and which conditions must escalate
operator attention before the lower panels or diagnostics are opened.

`/remote` remains the stable fallback. This spec should guide `/remote-next`
only until the new remote head is validated.

## Purpose

The state strip is the always-visible operating summary for the radio head.
It answers these questions without opening a drawer:

- Is the browser connected to the bridge?
- Does this client own the operating role?
- Is the radio in RX, armed, keyed, or locked?
- Is RF TX permitted by the bridge?
- Is this LAN, Tailnet, or WAN transport?
- Is latency acceptable?
- Is RX audio healthy?
- Is there a recent operator-visible fault?

The strip is not a diagnostics console. It shows the current operating risk and
the next likely action, then lets detailed panels carry deeper data.

## Current Pill Order

The first shipped order is fixed:

1. Connection
2. Ownership
3. RX/TX
4. RF State
5. Transport
6. Latency
7. Audio
8. Fault

Reasoning:

- Connection and Ownership determine whether any other control should be
  trusted.
- RX/TX and RF State are the high-risk operating states.
- Transport, Latency, and Audio describe quality of the live link.
- Fault is last so it can carry recent alarms without displacing the primary
  state.

Do not reorder for local visual preference. If a form factor cannot fit all
items, use the density rules below.

## Pill Contract

Each pill has:

- A short label: stable noun phrase, no verbs.
- A value: current state in operator language.
- A tone: `ok`, `rx`, `tx`, `keyed`, `warn`, or `alarm`.
- A title: detailed hover/long-press explanation.

Tone meanings:

- `ok`: normal and ready.
- `rx`: normal receive/idle flow, informational.
- `tx`: transmit path is intentionally active or armed.
- `keyed`: RF is keyed or believed to be keyed.
- `warn`: operator attention needed before TX or before trusting the link.
- `alarm`: fail-closed condition, stale data, or TX fault.

Color must not be the only signal. The value text and tone shape/pattern must
carry the state.

## Labels And Values

### Connection

Label: `Connection`

Values:

- `Idle`: no bridge websocket.
- `Connecting`: websocket attempt is in progress.
- `Connected`: websocket is open.
- `Preview`: demo/preview mode.

Escalation:

- `Idle`: warn.
- `Connecting`: rx.
- `Connected`: ok.
- `Preview`: rx.
- Socket error: record a `Fault` warning.
- Link lost during TX: record a `Fault` alarm and lock TX.

### Ownership

Label: `Ownership`

Values:

- `Operator`: bridge assigned the operator role.
- `Viewer`: bridge assigned view-only role.
- `Role pending`: connected but waiting for backend role.
- `Pending`: connection in progress.
- `No owner`: no active bridge client.

Escalation:

- `Operator`: ok.
- `Viewer`: warn, TX controls blocked.
- `Role pending`: warn, TX controls blocked.
- `Pending`: rx.
- `No owner`: warn.

Backend role state is authoritative. Do not infer operator ownership solely from
local browser state.

### RX/TX

Label: `RX/TX`

Values:

- `RX`: receive path.
- `Armed`: TX request is armed but not keyed.
- `PTT armed`: held PTT path is armed.
- `ON AIR`: transmit is keyed.
- `PTT ON AIR`: held PTT path is keyed.

Escalation:

- `RX`: rx.
- `Armed` / `PTT armed`: tx.
- `ON AIR` / `PTT ON AIR`: keyed.

Any browser blur, page hide, pointer release, key release, socket close, mic
failure, bridge TX fault, role loss, or RF-disabled transition must unkey or
remain fail-closed.

### RF State

Label: `RF State`

Values:

- `TX gated`: not connected or not ready.
- `TX locked`: connected but reconfirm window is closed.
- `TX ready`: reconfirm window is open.
- `TX path active`: TX request is active.
- `RF disabled`: bridge reports RF gate disabled.
- `RF unknown`: connected but bridge has not reported RF gate state.
- `View only`: backend role blocks TX.
- `Role pending`: backend role not known.
- `OOB`: VFO is out of band.
- `TX OOB`: active TX attempt while out of band.

Escalation priority:

1. `RF disabled`
2. View-only or role-pending ownership
3. Out-of-band
4. Active TX
5. Ready window
6. Reconfirm/locked
7. Disconnected/gated

RF disabled must take precedence over OOB and readiness. The operator should
see that the bridge will not permit RF before seeing a softer local TX state.

### Transport

Label: `Transport`

Values:

- `LAN IQ`: same-site raw IQ path.
- `Tailnet`: Tailscale/MagicDNS path.
- `WAN IQ`: explicit WAN stream mode, if selected.

Escalation:

- `LAN IQ`: rx.
- `Tailnet`: rx.
- `WAN IQ`: warn until WAN transport has a lower-bandwidth FFT/audio mode.

Transport is derived from page host and stream mode. It is not proof that the
radio data path is healthy; Latency and Audio own that signal.

### Latency

Label: `Latency`

Preferred value:

- `<N> ms RTT`: websocket round-trip from `saturn_ping` / `saturn_pong`.

Fallback values:

- `<N> ms IQ`: most recent IQ frame age.
- `<N.N>s IQ`: stale IQ frame age.
- `No IQ`: connected, no IQ frames yet.
- `Idle`: disconnected.
- `Preview`: demo/preview mode.

Escalation:

- RTT <= 250 ms: ok.
- RTT > 250 ms: warn.
- RTT > 600 ms: alarm.
- IQ age <= 750 ms: ok.
- IQ age > 750 ms: warn.
- IQ age > 2500 ms: alarm.
- Connected with no IQ: warn.

RTT is preferred because it measures browser-to-bridge responsiveness. IQ age is
still useful as a fallback and as data-path freshness.

### Audio

Label: `Audio`

Values:

- `Stopped`: RX audio is not running.
- `<rate> kHz - <lead> ms`: RX audio running with buffer lead.

Escalation:

- Stopped: warn.
- Running with no lead measurement: warn.
- Lead below -10 ms: warn.
- Lead above 180 ms: warn.
- Any resync/drop event since page load: warn.
- Otherwise: ok.

The title should include playback path (`MSG`, `SAB`, or `Legacy`) and combined
resync/drop count.

### Fault

Label: `Fault`

Values:

- `Clear`: no recent operator-visible fault.
- Short fault label: recent fault in the last 60 seconds.

Escalation:

- Clear: ok.
- Warning faults: warn.
- TX faults, mic unavailable during TX, or link loss during TX: alarm.

Fault is a recent-event surface, not persistent history. Longer fault history
belongs in a diagnostics drawer or log.

## Density Rules

### Phone Portrait

Target: one horizontal strip above the VFO/panadapter.

Show at least:

1. RX/TX
2. RF State
3. Latency
4. Fault if non-clear, otherwise Connection

Ownership must remain visible when it is not `Operator`. Audio may collapse into
Latency only when healthy. Fault may replace the least urgent quality pill while
active.

Values must stay short:

- `RX`
- `Ready`
- `RF off`
- `Viewer`
- `180 ms`
- `Fault`

No multi-line pill values on phone portrait.

### Phone Landscape

Show:

1. Connection
2. Ownership
3. RX/TX
4. RF State
5. Latency
6. Fault

Audio may be omitted from the strip because the audio panel remains directly
accessible.

### Tablet

Show all eight pills. Allow compact labels and full values.

### Laptop/Desktop

Show all eight pills with labels and values. Hover titles carry detail. Future
tap-expand can open a state drawer without changing the strip order.

## Tap Or Long-Press Detail

The first detail expansion should be a single state drawer, not separate modal
dialogs per pill.

Drawer sections:

- Session: websocket URL, backend role, client id.
- TX safety: RF gate, ready-window reason, lock reason, OOB state.
- Transport: LAN/Tailnet/WAN, page origin, websocket RTT.
- Audio: sample rate, path, buffer lead, resync/drop count.
- Faults: recent fault list with timestamps.

Do not add editable settings to the state drawer. It is read-only operational
context.

## Update Cadence

- Connection, Ownership, RX/TX, RF State: immediate on command/event.
- Transport: on load and layout/stream-mode change.
- Latency: every RTT pong and once per UI tick for stale display.
- Audio: every UI tick while audio is running.
- Fault: immediate on record and expires visually after 60 seconds.

Avoid running heavy DOM work from high-rate IQ/audio handlers. Those paths
should set state only; normal UI ticks render the strip.

## Implementation Backlog

Phase-ready slices:

1. Add phone-density rendering for the current eight-pill strip.
2. Add a read-only state drawer opened from the strip.
3. Record a bounded recent fault list instead of only the latest fault.
4. Add screenshot regression checks for phone/tablet/laptop strip layout.
5. Add backend role support for real multi-client viewer/operator assignment.
6. Add future WAN transport mode labels after FFT-row/audio-codec transport
   exists.

Do not combine these with unrelated TX, DSP, or settings changes.

## Acceptance Criteria

Before promoting a state-strip change:

- Desktop, tablet, phone portrait, and phone landscape layouts show no overlap.
- RX/TX, RF disabled, viewer, OOB, stale RTT, stopped audio, and active fault
  states are distinguishable without color.
- TX remains blocked for RF disabled, viewer, role pending, and reconnect lock.
- `/remote-next` still hard-refreshes cleanly after static deploy.
- The served HTML and served `saturn-remote-next.js` match the repo/build
  artifacts.
- `/remote` is untouched.
