# Per-band PA calibration

Saturn Remote's **TX Power Target stays variable from 1 to 100 W**, in 1 W
steps. PA calibration converts that target to the hardware's 0–255 drive code.
The code is not watts. A profile does not select a fixed 10 W or 65 W output.
Targets and calculated drive commands are not measurements of G2 or amplifier
output power.

## Operator controls

Open **TX → PA calibration by band**, also searchable from Settings.

1. Select **G2 starting values** to load an editable draft of the Thetis G2
   default PA profile. This button does not transmit or apply anything.
2. Select a band and edit its PA gain in dB if needed. Larger values reduce
   drive. Each band retains its own value; 11 m has a separate entry.
3. While receiving, select **Apply profile**. Managed sessions use the existing
   ordered settings transaction and cleanup boundary. The display distinguishes
   the requested profile from the host-confirmed profile.
4. Verify the target against the external meter, using the same stimulus and
   meter mode. For ideal balanced two-tone RF, average power is half PEP.
5. **Legacy curve → Apply profile** explicitly restores the previous mapping.

The complete equipment profile is saved in the normal Saturn Remote settings,
including the local fallback and Saturn Go settings save. It is restored as an
ordered setting when that operator reconnects to a supporting Bridge. Older
per-band operating memories cannot overwrite it. This is not an unattended
hardware auto-calibration or a new root-owned configuration file. A client
without the saved profile does not inherit another browser's preferences.

The panel's host drive value is a calculated command, not a continuous register
readback. Direct XDMA verifies the matching DAC register at key assertion; the
existing key log includes the drive code. P2 command completion and physical
RF output remain different evidence.

## Conversion and frequency selection

Existing installations default to the original measured curve. No profile is
selected just by upgrading. Its P2 full-scale environment setting remains
effective in Legacy mode. A selected band profile uses the same conversion for
both backends:

```text
target_dBm = 10 log10(target_W × 1000)
volts = sqrt(10^((target_dBm − PA_gain_dB) / 10) × 0.05)
drive = floor(255 × min(1.02 × min(volts / 0.8, 1), 1))
```

Zero drive remains zero; positive targets use bounded 1–255 codes. Targets
above 100 W remain clamped to 100 W. Profiles must contain exactly 12 finite
gains, each 40–80 dB; incomplete or invalid transactions are rejected.

Profile selection uses **the actual TX frequency**, so split operation does not
borrow the receive band's gain. The extended PA regions are:

| Entry | Frequency region, MHz | G2 starting gain, dB |
|---|---|---:|
| 160 m | 1.8–2.75 | 47.9 |
| 80 m | 2.75–5.25 | 50.5 |
| 60 m | 5.25–7.0 | 50.8 |
| 40 m | 7.0–8.7 | 50.8 |
| 30 m | 8.7–12.075 | 50.9 |
| 20 m | 12.075–16.209 | 50.9 |
| 17 m | 16.209–19.584 | 50.5 |
| 15 m | 19.584–23.17 | 47.0 |
| 12 m | 23.17–26.495 | 47.9 |
| 11 m | 26.495–28.0 | 46.5 |
| 10 m | 28.0–29.7 inclusive | 46.5 |
| 6 m | 50.0–54.0 inclusive | 44.6 |

Upper boundaries belong to the next region except where stated. Frequencies
outside these regions use the Legacy curve. These are calibration-selection
regions, not permission to transmit. G2 starting values are an editable
reference profile, not a calibration certificate for an individual radio.

At 40 W target, the starting profile calculates code **41 on 40 m** and **68 on
11 m**. At 10 W on 11 m it calculates **34**. These are examples along the
variable curve, not power limits. The previous global curve calculated code 41
for 40 W on every band.

## Protocol and validation

```text
tx_pa_calibration:0,legacy;
tx_pa_calibration:0,47.9|50.5|50.8|50.8|50.9|50.9|50.5|47|47.9|46.5|46.5|44.6;
tx_pa_calibration_supported:0,true;
tx_pa_drive:0,27426000,40,68,11m,46.5;
```

The full profile is one atomic configuration, in the table's band order.
Managed processing preserves generation checks, terminal transaction results
and stop/cleanup before configuration. Legacy hosts reject profile changes
while TX is active or pending. No setter keys the radio.

Regression coverage includes the saved Thetis calculation examples, all 1–100 W
targets for monotonicity, zero and upper clamps, invalid/partial profiles,
independent 11 m edits, split TX selection, P2 high-priority encoding, direct DAC
readback, ordered ingress and cancellation, browser persistence, and the actual
runtime band-recall loop on both fixtures. `validate-pa-calibration.mjs` checks
the shipped panel in Chrome without any radio socket.

This change does not alter microphone gain, tone amplitude, PureSignal fitting,
feedback attenuation, firmware, XDMA driver or polling. Correct PA mapping does
not establish improved RF IMD; compare PS off/on at matched externally measured
power after the individual radio's calibration is checked.
