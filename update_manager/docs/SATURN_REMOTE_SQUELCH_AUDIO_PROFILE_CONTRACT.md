# Saturn Remote Squelch and Audio Profile Contract

Created: 2026-05-13

Scope: pre-implementation contract for adding RX squelch and quick audio
profiles to `/remote-next`.

## Current Status

Phase 34 is a contract and readiness phase. It does not add a visible squelch
control yet, because the Saturn bridge does not currently publish or accept RX
squelch state. The next implementation phase should add bridge state and WDSP
binding first, then expose the UI after the backend contract is testable.

## Goals

- Avoid dead controls in `/remote-next`.
- Keep the primary audio strip compact and useful on phone, tablet, and desktop.
- Make squelch fail open: missing, invalid, or unsupported state must not
  silently mute receive audio.
- Keep TX safety behavior unchanged. Squelch and audio profiles must not bypass
  TX reconfirm, RF-disabled state, page-hide fail-closed behavior, or
  operator/viewer role filtering.
- Keep TCI-compatible text commands available for external clients while leaving
  room for a later browser-native control protocol.

## Existing Evidence

The current Saturn bridge has model and TCI support for RX volume, AGC, NR, NB,
ANF, RX filter cuts, and TX noise gate. It does not yet have RX squelch fields,
TCI commands, or WDSP bindings.

Local peer-project and WDSP evidence shows a practical first slice:

- `wdsp.h` exposes SSB syllabic squelch functions:
  - `SetRXASSQLRun(int channel, int run)`
  - `SetRXASSQLThreshold(int channel, double threshold)`
  - `SetRXASSQLTauMute(int channel, double tau_mute)`
  - `SetRXASSQLTauUnMute(int channel, double tau_unmute)`
- NereusSDR maps a user slider from `0..100` to WDSP SSQL threshold `0.0..1.0`.
- NereusSDR defaults SSQL off with a threshold near `16` on the `0..100` UI
  scale.
- AM and FM squelch exist in peer code on different scales, so they should not
  be mixed into the first Saturn implementation without separate validation.

## Initial Feature Boundary

The first bridge implementation should support SSB/voice syllabic squelch only:

- Modes: LSB, USB, DSB, and SSB-like voice modes already routed through the RX
  WDSP chain.
- Explicitly deferred: AM squelch, FM squelch, tone squelch, CTCSS, and
  automatic mode-specific squelch remapping.
- Default: disabled.
- Default threshold: `16` on the UI scale, mapped to `0.16` internally.

If the current demodulation mode is not supported by the first SSQL
implementation, the bridge should preserve the requested state but publish a
future capability/status field before the UI claims that squelch is active for
that mode.

## TCI Text Contract

Browser/operator commands:

```text
rx_ssql:0,true;
rx_ssql:0,false;
rx_ssql_threshold:0,16;
```

Bridge published state:

```text
rx_ssql:0,false;
rx_ssql_threshold:0,16;
```

Rules:

- `rx_ssql` accepts `true`, `false`, `1`, and `0`.
- `rx_ssql_threshold` accepts a browser/operator value from `0..100`.
- The bridge clamps bad threshold values into `0..100`; parse failures leave the
  previous value unchanged and should publish an operator-visible fault only if
  the command came from a browser operator session.
- Internal WDSP threshold is `clamp(threshold, 0, 100) / 100.0`.
- Bridge snapshots should include both fields so a refreshed browser renders the
  real backend state instead of a stale local preference.
- Viewer clients must not be allowed to send these commands. Phase 31 command
  filtering needs the new commands in the operator-only control set.

## Bridge State

Add fields to the bridge radio model:

```text
rx_ssql_enabled: bool     default false
rx_ssql_threshold: f64    default 16.0
```

The state publish path should emit `rx_ssql` and `rx_ssql_threshold` with the
same cadence and initial-snapshot behavior as existing audio/DSP settings.

The WDSP sync layer should only call SSQL functions when the model value changes
or when the RX channel is rebuilt. Disabling squelch must call
`SetRXASSQLRun(channel, 0)` and must immediately restore normal receive audio.

## WDSP Mapping

First implementation mapping:

```text
SetRXASSQLThreshold(rx_channel, threshold / 100.0)
SetRXASSQLRun(rx_channel, enabled ? 1 : 0)
```

Recommended first defaults:

```text
enabled = false
threshold = 16.0
tau_mute = WDSP default unless testing shows a pop/click problem
tau_unmute = WDSP default unless testing shows slow opening
```

Do not add AM/FM squelch bindings in the same patch unless their scale and
mode behavior are tested separately.

## Remote-Web State And Settings

Add these fields through the same route used by AGC, NR, NB, ANF, and filter
state:

```text
rxSsqlEnabled: boolean
rxSsqlThreshold: number
```

The parser should update state from `rx_ssql` and `rx_ssql_threshold`. Settings
and profile serialization may store the latest preferred threshold, but the UI
must render the bridge-published state as authoritative after connect.

Bad local settings should normalize to:

```text
rxSsqlEnabled = false
rxSsqlThreshold = 16
```

## UI Contract

The first visible UI should be a compact SQL row in the existing primary audio
strip, not a new large dashboard panel.

Suggested control shape:

- A `SQL` toggle.
- A threshold slider from `0..100`, labeled compactly as `SQL 16`.
- The threshold slider remains visible while SQL is off, but changing the slider
  does not automatically enable squelch. Muting receive audio should require the
  explicit toggle.
- The Operator State drawer may show `Squelch Off` or `SQL 16` after the bridge
  publishes real state.
- The state strip should not add another permanent pill for squelch in the first
  implementation; the existing Audio/Link drawer details are enough.

Accessibility and device behavior:

- Touch drag must work on Android, iPhone, iPad, and desktop pointer devices.
- Keyboard focus should allow normal slider adjustment.
- Safari/iOS unsupported wake-lock behavior must not affect squelch state.
- `Go Live` audio unlock remains the user gesture for receive audio; SQL must
  not start or stop the audio context by itself.

## Audio Profiles

Audio profiles are quick RX-audio/DSP presets, not whole radio memories.

Profile fields:

```text
rxVolumeDb
agcMode
agcGain
nrMode
nrLevel
nbMode
nbThreshold
anfEnabled
filterLow
filterHigh
rxSsqlEnabled
rxSsqlThreshold
```

Do not include:

- Frequency or band.
- VFO tune step.
- TX drive, TX gate, TX EQ, CFC, or RF enable state.
- Client role, connection path, or page-session state.

Suggested default profile names:

- `Clean RX`
- `Weak Signal`
- `Noisy Band`
- `Voice Squelch`

Audio profiles can later live inside the existing remote profile catalog, but
they should be applied as a narrow RX audio/DSP bundle so selecting one cannot
retune the radio or change TX behavior.

## Acceptance Criteria For Implementation

Bridge:

- Unit tests cover command parsing, publish formatting, threshold clamping, and
  default snapshot values.
- `cargo test -j1 --manifest-path update_manager/saturn-bridge/Cargo.toml`
  passes.
- Release build passes before any live binary install.
- With SQL disabled, receive audio behavior is unchanged from the current live
  baseline.
- Enabling SQL on a supported voice mode can mute idle noise above threshold and
  disabling SQL restores audio immediately.

Frontend:

- TCI parser tests cover `rx_ssql` and `rx_ssql_threshold`.
- Settings normalization tests cover missing, invalid, and out-of-range values.
- `npm test -- --run`, `npm run typecheck`, and `npm run build` pass.
- `npm run validate:remote-next-layout` passes for closed and open Operator
  State drawer scenarios after UI is added.
- The control is operator-only; viewer sessions render state but cannot send
  squelch changes.

Live deployment:

- `/remote-next` still returns the expected HTTP 401 auth gate before browser
  auth is supplied.
- Deployed HTML/JS/binary checksums match repo/build outputs for any runtime
  phase that copies assets.
- Local docs record whether this was docs-only, frontend-only, bridge-only, or a
  full runtime deployment.

## Implementation Order

1. Add bridge radio-model fields, TCI parser/publisher support, and role
   filtering for `rx_ssql` and `rx_ssql_threshold`.
2. Add WDSP SSQL bindings and bridge sync behavior.
3. Add remote-web parser/state/settings fields and tests.
4. Add the compact `/remote-next` SQL controls after bridge state exists.
5. Add audio profile save/apply behavior as a separate UI slice.
6. Run real browser validation on Android, desktop, and Apple Safari devices.

## Open Decisions

- Whether to publish a future `rx_squelch_capability` field before AM/FM
  squelch support.
- Whether audio profiles should be global, per-band, or per-mode.
- Whether profile application should prompt before changing RX filter cuts.
- Whether `Voice Squelch` should enable SQL by default or only load the
  threshold and leave the toggle off.
