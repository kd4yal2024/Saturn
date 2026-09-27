# Saturn Remote redesign: control migration inventory

Generated from `remote-web/src/settings/registry.ts` — do not hand-edit the tables.
Refresh with `npm run generate:migration`; CI can fail on drift with `npm run check:migration`.

**How to read a row.** The registry is the machine-readable index of every control,
readout, indicator, overlay, query parameter and storage key. `Settings` is where the
entry lives in the new surface; `Main screen` is its default presence there.

**Status legend.**

- `wired` — the original control and handler remain, and its Settings route is covered
  by the browser registry sweep. The registry test fails if its target id disappears.
- `pending integration` — the feature exists but is only reachable through the legacy
  setup shell until Phase 3 wires the new surface. It is not claimed complete.
- `no home yet` — flagged for the owner or the integration owner rather than guessed.

**Verification boundary.** Chromium checks Settings routes for DOM-backed entries at
phone and desktop widths, and focused probes exercise safe native controls and selected
original actions. Actions that require a live radio, transmit, or disconnection are
reserved for hardware QA; this offline evidence does not claim those actions ran.

**Owner-approved removal.** The analog meter was removed from the visible meter picker
and Settings after the owner requested it. The signal, power, SWR, ALC, compression,
microphone, and L/R audio meter readouts remain available.

## Display (61)

Spectrum, waterfall and 3D view behaviour

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Display view | id `view-3d` | main screen, default | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Colour map | id `terrain-palette` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D history rows | id `terrain-depth` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D surface height | id `terrain-height` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Camera elevation | id `terrain-elevation` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Far-field width | id `terrain-perspective` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D floor level | id `terrain-floor` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D ceiling level | id `terrain-ceiling` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D fit range | id `terrain-fit-range` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D and waterfall split | id `terrain-split` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Span | id `sample-rate-readout` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Display status | id `display-caption` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Display zoom | id `display-zoom` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Pause display | id `display-pause` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Averaging | id `display-spectrum-average` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Tune to peak | id `display-peak-tune-assist` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Stay awake | localStorage `saturn.remote.keepScreenAwake` | main screen, pinnable | Display → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Waterfall history | id `terrain-status` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Band edge markers | id `display-show-band-edges` | main screen, pinnable | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Terrain diagnostics | id `terrain-diagnostics-enabled` | Settings only | Display → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Transceiver clarity | id `display-transceiver-clarity` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Undo appearance change | id `display-appearance-undo` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Spectrum auto range | id `display-spectrum-auto-range` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall auto range | id `display-waterfall-auto-range` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Spectrum floor | id `display-spectrum-floor` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Spectrum ceiling | id `display-spectrum-ceiling` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall floor | id `display-waterfall-floor` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall ceiling | id `display-waterfall-ceiling` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Spectrum trace colour | id `display-spectrum-trace-color` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Trace smoothing | id `display-spectrum-trace-smoothing` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Trace fill | id `display-spectrum-trace-fill` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Peak glow | id `display-spectrum-peak-glow` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Glass sheen | id `display-spectrum-glass-sheen` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall speed | id `display-waterfall-speed` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall contrast | id `display-waterfall-contrast` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall smoothing | id `display-waterfall-smoothing` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall palette | id `display-waterfall-palette` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Spectrum peak hold | id `display-spectrum-peak-hold` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Enhanced colours | id `display-spectrum-enhanced-colors` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Show grid | id `display-show-grid` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Show center line | id `display-show-center-line` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Center display | id `display-center-btn` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Waterfall on or off | id `waterfall-toggle-btn` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Traditional view | id `view-traditional` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D settings | id `terrain-settings-open` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D or traditional surface | id `terrain-mode` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D gamma | id `terrain-gamma` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D noise cleanup | id `terrain-cleanup` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D waterfall cleanup | id `terrain-waterfallCleanup` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Cleanup baseline | id `terrain-cleanupBaseline` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Estimate noise floor | id `terrain-noise-estimate` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D smoothing | id `terrain-smoothing` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D quality | id `terrain-quality` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| 3D grid opacity | id `terrain-gridOpacity` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Reset 3D settings | id `terrain-reset` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Averaging toolbar control | id `spectrum-average-toolbar-btn` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Peak toolbar control | id `spectrum-peak-toolbar-btn` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Tune peak toolbar control | id `spectrum-tune-peak-btn` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Wake lock toggle | id `wake-lock-btn` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Frequency lock toggle | id `freq-lock-btn` | main screen, pinnable | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Traditional waterfall settings | id `terrain-traditional-settings` | Settings only | Display → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Meter (7)

S-meter, TX meters and meter details

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Meter type | id `instrument-meter-mode` | main screen, pinnable | Meter → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Meter averaging | id `instrument-meter-average-btn` | main screen, pinnable | Meter → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Meter readout | id `meter-readout` | main screen, default | Meter → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Left and right dBFS bars | id `instrument-rx-audio-meters` | main screen, pinnable | Meter → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| More meter details and TX tools | id `meter-details-disclosure` | main screen, pinnable | Meter → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX meter mode | id `tx-meter-mode` | main screen, pinnable | Meter → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Meter peak hold | id `instrument-meter-peak-btn` | main screen, pinnable | Meter → legacy display panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Receive (37)

RX audio, DSP and filtering — **live radio state**: no restore-defaults here.

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| DSP panel | id `operations-tab-dsp` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Start or stop audio | id `rx-audio-toggle-btn` | main screen, default | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Receive volume | id `rx-volume` | main screen, default | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Audio buffer | id `audio-buffer-status` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Voice squelch | id `rx-ssql-btn` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Voice squelch level | id `rx-ssql-threshold` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| AGC speed | id `rx-agc-grid` | main screen, default | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| AGC gain | id `agc-gain` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Automatic notch filter | id `rx-anf-btn` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Noise reduction | id `setup-dsp-nr-mode` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Noise blanker | id `mobile-nb-btn` | main screen, pinnable | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Filter low cut | id `filter-low` | main screen, default | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Filter high cut | id `filter-high` | main screen, default | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Passband overlay | id `filter-window` | main screen, default | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| RX audio codec | ?rx_audio_codec | Settings only | Receive → route to the original control | wired | parameter documented; no DOM handler to exercise |
| RX codec diagnostics | id `setup-network-copy-diagnostics-btn` | Settings only | Receive → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Noise reduction level | id `setup-dsp-nr-level` | main screen, pinnable | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| NR2 gain method | id `setup-dsp-nr2-gain-method` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| NR2 NPE method | id `setup-dsp-nr2-npe-method` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| NR2 post filter | id `setup-dsp-nr2-post-filter` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANR taps | id `setup-dsp-anr-taps` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANR delay | id `setup-dsp-anr-delay` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANR gain | id `setup-dsp-anr-gain` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANR leakage | id `setup-dsp-anr-leakage` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANR Thetis defaults | id `setup-dsp-anr-thetis-btn` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANR wide setting | id `setup-dsp-anr-wide-btn` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANF enable | id `setup-dsp-anf-enabled` | main screen, pinnable | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANF taps | id `setup-dsp-anf-taps` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANF delay | id `setup-dsp-anf-delay` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANF gain | id `setup-dsp-anf-gain` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANF leakage | id `setup-dsp-anf-leakage` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANF Thetis defaults | id `setup-dsp-anf-thetis-btn` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| ANF sharp setting | id `setup-dsp-anf-sharp-btn` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| RX equaliser | id `rx-eq-enable-btn` | Settings only | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Noise reduction level (panel) | id `rx-nr-level` | main screen, pinnable | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Noise blanker threshold (panel) | id `rx-nb-threshold` | main screen, pinnable | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| WFM mode | id `wfm-mode-btn` | main screen, default | Receive → legacy dsp panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Transmit (50)

TX audio path, monitor and transmit controls — **live radio state**: no restore-defaults here.

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Arm transmit | id `tx-arm-btn` | main screen, always shown (locked) | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Hold PTT | id `ptt-btn` | main screen, always shown (locked) | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| MOX | id `mox-btn` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Lock transmit | id `tx-lock-btn` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX state indicator | id `tx-zone-state` | main screen, default | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Arm window countdown | id `tx-ready-countdown` | main screen, default | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Microphone level | id `tx-zone-mic-value` | main screen, default | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Forward power | id `tx-zone-power-value` | main screen, default | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| SWR | id `tx-zone-swr-value` | main screen, default | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX audio source | id `tx-audio-source` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Pair native sender | id `satp-pair-btn` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Monitor transmit audio | id `tx-mon-btn` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Monitor level | id `tx-mon-level` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX drive | id `tx-drive` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Microphone gain | id `tx-mic-gain` | main screen, pinnable | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX filter low cut | id `tx-filter-low` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX filter high cut | id `tx-filter-high` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX equaliser | id `tx-eq-enable-btn` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Continuous frequency compression | ?tx_cfc | Settings only | Transmit → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX noise gate | id `tx-noise-gate-enabled` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX downward expander | id `tx-dexp-enabled` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Speech processor | id `tx-speech-processor-enabled` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| CESSB | id `tx-cessb-enabled` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| PureSignal | id `setup-tx-puresignal-enabled` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Phase rotator | id `setup-tx-phase-rotator-enabled` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Two-tone test | id `setup-tx-two-tone-section` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX timeout | id `setup-tx-timeout-enabled` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Two-tone generator | id `two-tone-btn` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX setup disclosure | id `tx-advanced-card` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX state badge | id `app-tx-state-badge` | main screen, always shown (locked) | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| On-air bar | id `tx-on-air-bar` | main screen, always shown (locked) | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Transmit state announcement | id `tx-state-live` | Settings only | Transmit → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| CFC enable | id `cfc-enable-btn` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| CFC pre-compensation | id `cfc-precomp` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX timeout seconds | id `setup-tx-timeout-seconds` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Phase rotator auto | id `setup-tx-phase-rotator-auto` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Phase rotator corner | id `setup-tx-phase-rotator-corner` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| PureSignal auto attenuation | id `setup-tx-puresignal-auto-attenuate` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| PureSignal attenuation | id `setup-tx-puresignal-attenuation` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Reset PureSignal | id `setup-tx-puresignal-reset` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Expander threshold | id `tx-dexp-threshold` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Expander ratio | id `tx-dexp-expansion` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Speech processor gain | id `tx-speech-processor-gain` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX noise gate threshold | id `tx-noise-gate-threshold` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Voodoo 38k | id `apply-voodoo-38k-btn` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| SATP pairing key | id `satp-key` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Copy SATP key | id `satp-copy-key` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Hide SATP key | id `satp-hide-key` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX tool select | id `tx-tool-select` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Open TX tool | id `tx-tool-open-btn` | Settings only | Transmit → legacy tx panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Radio (28)

Radio path, band, mode and memories — **live radio state**: no restore-defaults here.

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Radio panel | id `operations-tab-radio` | main screen, pinnable | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Direct frequency entry | id `freq-entry-overlay` | main screen, default | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| VFO A frequency | id `dds-readout` | main screen, always shown (locked) | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| VFO B | id `vfo-b-select-btn` | main screen, pinnable | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| VFO reference | id `vfo-a-select-btn` | main screen, pinnable | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Split operation | id `split-toggle-btn` | main screen, pinnable | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Band | id `band-grid` | main screen, default | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Demodulation mode | id `mode-grid` | main screen, default | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| WFM de-emphasis | id `wbfm-deemphasis` | Settings only | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Tuning step | localStorage `saturn.remote.vfoTuneStepHz` | main screen, pinnable | Radio → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Frequency lock | localStorage `saturn.remote.freqLock` | main screen, pinnable | Radio → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Band memories | localStorage `saturn.remote.bandMemory` | Settings only | Radio → route to the original control | wired | parameter documented; no DOM handler to exercise |
| RX input | id `path-summary` | main screen, default | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| RX antenna | id `rx-antenna` | main screen, default | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Attenuator | id `rx-attenuation` | main screen, default | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Remote profiles | id `setup-panel-profiles` | Settings only | Radio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Radio preferences | localStorage `saturn.remote.radioPrefs` | Settings only | Radio → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Sample rate | id `sample-rate` | main screen, pinnable | Radio → legacy advanced panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| RX ADC | id `rx-adc` | main screen, pinnable | Radio → legacy advanced panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Stream mode | id `setup-stream-mode` | main screen, pinnable | Radio → legacy network panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Profile select | id `setup-profile-select` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Load profile | id `setup-profile-load-btn` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Save profile | id `setup-profile-save-btn` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Save profile as | id `setup-profile-save-as-btn` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Delete profile | id `setup-profile-delete-btn` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Startup profile | id `setup-startup-profile-select` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Apply startup profile now | id `setup-startup-apply-btn` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Clear startup profile | id `setup-startup-clear-btn` | Settings only | Radio → legacy profiles panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Audio (8)

Devices, codec and audio health

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Audio panel | id `operations-tab-audio` | main screen, pinnable | Audio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Microphone input | localStorage `saturn.remote.audioInputDeviceId` | Settings only | Audio → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Audio output | localStorage `saturn.remote.audioOutputDeviceId` | Settings only | Audio → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Adaptive playback rate | ?rx_audio_adaptive | Settings only | Audio → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Audio scope | id `operations-panel-audio` | main screen, pinnable | Audio → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Microphone device | id `setup-audio-input-select` | Settings only | Audio → legacy audio panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Output device | id `setup-audio-output-select` | Settings only | Audio → legacy audio panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Refresh audio devices | id `setup-audio-refresh-devices-btn` | Settings only | Audio → legacy audio panel | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Network (11)

Bridge connection and transport

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Network panel | id `operations-tab-network` | main screen, pinnable | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Connection status | id `operator-conn-pill` | main screen, default | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Connection state text | id `operator-conn-value` | main screen, default | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Round-trip time | id `operator-latency-value` | main screen, pinnable | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Transport | id `operator-transport-value` | main screen, pinnable | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Bridge address | id `ws-url` | Settings only | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Go offline | id `system-go-offline-btn` | Settings only | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Go live | id `go-live-btn` | main screen, pinnable | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Split websockets | ?transport | Settings only | Network → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Legacy single socket | ?force_legacy_ws | Settings only | Network → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Operator diagnostics | id `setup-network-copy-diagnostics-btn` | Settings only | Network → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Memory (2)

Band memories

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Memory panel | id `operations-tab-memory` | main screen, pinnable | Memory → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Save current memory | id `operations-memory-save-btn` | Settings only | Memory → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Log (2)

Operator log and live client trace

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Operator log | id `operations-panel-log` | main screen, pinnable | Log → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Live client trace | id `operations-tab-log` | Settings only | Log → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## Interface (15)

Theme, layout and keyboard

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Settings | id `header-setup-btn` | main screen, always shown (locked) | Interface → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Operations drawer | id `operations-drawer` | main screen, default | Interface → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Theme | id `theme-btn` | main screen, pinnable | Interface → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Phone view | id `layout-btn` | main screen, pinnable | Interface → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Keyboard shortcuts | id `shortcut-help-btn` | main screen, pinnable | Interface → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Display preferences | localStorage `saturn.remote.displayPrefs` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Meter mode memory | localStorage `saturn.remote.instrumentMeterMode` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Last Operations panel | localStorage `saturn.remote.operationsDrawer` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Last Settings section | localStorage `saturn.remote.setupPanel` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Phone spectrum mode | localStorage `saturn.remote.phoneSpectrumMode` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Phone waterfall | localStorage `saturn.remote.phoneWaterfall` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Active profile | localStorage `saturn.remote.activeProfile` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Display split ratio | localStorage `saturn.remote.spectrumWaterfallRatio` | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Split websocket alias | ?phase42_split | Settings only | Interface → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Phone dock | id `mobile-control-dock` | main screen, default | Interface → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |

## About and system (14)

Device, versions, diagnostics

| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |
| --- | --- | --- | --- | --- | --- |
| Radio | id `static-wsdiag-marker` | Settings only | About and system → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Websocket probe | id `static-wsdiag-btn` | Settings only | About and system → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Copy log | id `static-wsdiag-copy-btn` | Settings only | About and system → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| Saturn Go | id `saturn-manage-home` | Settings only | About and system → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| System monitor | id `saturn-manage-monitor` | Settings only | About and system → route to the original control | wired | Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA |
| TX diagnostic mode | ?tx_diagnostic | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX level override | ?tx_level_db | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| Display profile | ?display_profile | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX source override | ?tx_source | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX Opus codec | ?tx_opus | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX voice equaliser | ?tx_voice_eq | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX CFC pre-compensation | ?tx_cfc_precomp_db | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX noise gate diagnostic | ?tx_noise_gate | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |
| TX noise gate threshold | ?tx_noise_gate_db | Settings only | About and system → route to the original control | wired | parameter documented; no DOM handler to exercise |

## Safety-pinned items that cannot be unpinned

- **VFO A frequency** — Main receive frequency.
- **TX state badge** — Always-visible transmit state badge in the app bar; never collapsed at any width.
- **Arm transmit** — Arm the transmitter; PTT cannot key until armed.
- **Hold PTT** — Momentary push to talk; releasing on pointer up, cancel or blur unkeys.
- **On-air bar** — Danger bar across the top of the viewport while transmitting, visible at every width.
- **Settings** — Open the full settings index; gear button in the app bar.

## Coverage

224 interactive elements carry an id in the template: **169 indexed** and **55 explicitly excluded**, so nothing is unaccounted for.
The registry test fails if a template control is neither indexed nor excluded, and also fails on a stale exclusion, so this total is enforced rather than asserted here.

## Excluded controls, with reasons

### Navigation and modal chrome (37)

| Control | Covered by | Reason it is not a separate setting |
| --- | --- | --- |
| `phone-menu-btn` | — | Trigger for the phone overflow sheet. |
| `phone-menu-back-btn` | — | Returns from the hosted System view. |
| `phone-menu-close-btn` | — | Closes the phone overflow sheet. |
| `phone-menu-status-btn` | Connection status | Opens the existing operator detail overlay. |
| `phone-menu-system-btn` | Radio | Hosts the header diagnostics menu inside the sheet. |
| `phone-menu-help-btn` | Keyboard shortcuts | Routes to the shortcut help. |
| `phone-menu-phone-btn` | Phone view | Routes to the existing Phone view toggle. |
| `phone-menu-theme-btn` | Theme | Routes to the existing theme control. |
| `phone-menu-settings-btn` | Settings | Routes to the existing settings entry point. |
| `phone-menu-radio-path-btn` | RX antenna | Routes to the existing radio-path panel. |
| `phone-menu-ops-memory-btn` | Memory panel | Operations route. |
| `phone-menu-ops-audio-btn` | Audio panel | Operations route. |
| `phone-menu-ops-network-btn` | Network panel | Operations route. |
| `phone-menu-ops-dsp-btn` | DSP panel | Operations route. |
| `phone-menu-ops-radio-btn` | Radio panel | Operations route. |
| `phone-menu-ops-log-btn` | Live client trace | Operations route. |
| `setup-menu-btn` | Settings | Legacy settings trigger. |
| `settings-search` | Settings | Search field for the Settings index. |
| `settings-index-return` | Settings | Returns from an original setup panel to the Settings section index. |
| `setup-close-btn` | — | Closes the legacy settings sheet. |
| `setup-tab-profiles` | Profile select | Section tab in the legacy sheet. |
| `setup-tab-display` | Display view | Section tab in the legacy sheet. |
| `setup-tab-dsp` | Noise reduction | Section tab in the legacy sheet. |
| `setup-tab-tx` | TX audio source | Section tab in the legacy sheet. |
| `setup-tab-network` | Bridge address | Section tab in the legacy sheet. |
| `setup-tab-audio` | Microphone device | Section tab in the legacy sheet. |
| `setup-tab-advanced` | Sample rate | Section tab in the legacy sheet. |
| `setup-dsp-tab-nr` | Noise reduction | Sub-tab inside the DSP section. |
| `setup-dsp-tab-anf` | Automatic notch filter | Sub-tab inside the DSP section. |
| `setup-dsp-tab-eq` | RX equaliser | Sub-tab inside the DSP section. |
| `setup-audio-open-rx-btn` | Start or stop audio | Opens the receive audio panel. |
| `setup-audio-open-tx-btn` | TX audio source | Opens the transmit audio panel. |
| `operations-close-btn` | Operations drawer | Closes the operations drawer. |
| `operations-audio-close-btn` | Audio scope | Collapses the audio scope panel. |
| `operator-detail-close-btn` | Connection status | Closes the operator detail overlay. |
| `shortcut-close-btn` | Keyboard shortcuts | Closes the shortcut help overlay. |
| `freq-entry-cancel-btn` | Direct frequency entry | Cancels direct frequency entry. |

### Duplicates of an indexed control (per-tier or quick action) (11)

| Control | Covered by | Reason it is not a separate setting |
| --- | --- | --- |
| `system-go-live-btn` | Go live | System-menu entry forwards to the original Go Live, Cancel Connect or Cancel Retry handler. |
| `operator-detail-copy-rx-btn` | Operator diagnostics | Copies the same operator log as the diagnostics button. |
| `vfo-mode-tag` | Demodulation mode | VFO mode shortcut for the mode grid. |
| `vfo-band-tag` | Band | VFO band shortcut for the band grid. |
| `vfo-antenna-pill` | RX antenna | VFO antenna shortcut for the radio path select. |
| `rx-att-quick-btn` | Attenuator | VFO-line shortcut for the same attenuator value. |
| `mobile-att-btn` | Attenuator | Phone-view quick action. |
| `mobile-nr-btn` | Noise reduction | Phone-view quick action. |
| `mobile-anf-btn` | Automatic notch filter | Phone-view quick action. |
| `mobile-filter-btn` | Filter low cut | Phone-view quick action for the RX filter. |
| `mobile-wake-lock-btn` | Wake lock toggle | Phone-view quick action for the wake lock. |

### Parameters inside a control surface that is already indexed (7)

| Control | Covered by | Reason it is not a separate setting |
| --- | --- | --- |
| `band-more-btn` | Band | Reveals the remaining band choices on narrow tiers. |
| `mode-more-btn` | Demodulation mode | Reveals the remaining mode choices on narrow tiers. |
| `two-tone-freq1` | Two-tone generator | Two-tone generator parameter. |
| `two-tone-freq2` | Two-tone generator | Two-tone generator parameter. |
| `two-tone-level` | Two-tone generator | Two-tone generator parameter. |
| `two-tone-delay` | Two-tone generator | Two-tone generator parameter. |
| `two-tone-invert-lsb` | Two-tone generator | Two-tone generator parameter. |
