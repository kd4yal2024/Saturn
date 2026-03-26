# Saturn vs openHPSDR Protocol 2 v4.4

Date: 2026-03-26

## Purpose

This document records the current byte-level comparison between Saturn
`P2_app`/`P3_app` and the local openHPSDR Protocol 2 v4.4 specification.

It is intended to be the working compatibility baseline for future protocol
changes, especially discovery-reply changes that can affect client behavior.

## Sources Used

Primary spec:

- `../../../OpenHPSDR-Firmware/Protocol 2/Documentation/openHPSDR Ethernet Protocol v4.4.docx`
- `../../../OpenHPSDR-Firmware/Protocol 2/Documentation/openHPSDR Ethernet Protocol v4.4.pdf`

Primary Saturn implementations:

- `sw_projects/P2_app`
- `sw_projects/P3_app`
- `sw_projects/common`

## Scope

This comparison covers:

- discovery reply fields
- general packet handling
- DDC specific packet handling
- DUC specific packet handling
- high-priority packet handling
- stream packet sizes, ports, and framing

This is a source-level wire-format comparison. It is not a packet-capture
comparison.

## Executive Summary

Saturn `P2_app` and `P3_app` are mostly "Protocol 4.3 plus selected 4.4
fields," not full v4.4 implementations.

The largest current divergences are:

- discovery byte `12` still advertises protocol `4.3`
- discovery byte `20` reports `4` DDCs even though Saturn exposes `10`
- discovery byte `23` is used as an app-version field instead of a beta flag
- high-priority data bytes `1396-1397` (`ClientControl`) are not implemented

The largest current matches to v4.4 are:

- discovery board type `10` = `SATURN`
- CAT over TCP at high-priority data bytes `1398-1399`
- Saturn mic selection at TX-specific byte `50` bit `5`
- RX attenuation during TX at TX-specific bytes `58-59`
- FIFO overflow/depth status in high-priority status
- ADC max-magnitude reporting at high-priority status bytes `39-42`

`P2_app` and `P3_app` are materially aligned on the wire. Differences between
them are mainly internal/runtime hardening and not packet-format changes.

## Packet And Field Matrix

### Discovery Reply Packet

| Bytes | v4.4 meaning | Saturn status | Notes |
| --- | --- | --- | --- |
| `11` | Board Type | Match | Both apps advertise `10` = `SATURN`. |
| `12` | Protocol version supported | Mismatch | Both apps advertise `43`, not `44`. |
| `13` | Firmware code version | Match | Set from FPGA firmware version at runtime. |
| `20` | Number of DDCs implemented | Mismatch | Both apps advertise `4`, while Saturn defines `10` DDCs and exposes ports `1035-1044`. |
| `21` | Frequency/phase-word mode | Match by intent | Saturn advertises phase-word mode (`1`) and behaves as a phase-word-only implementation. |
| `22` | Available endian/data modes | Constrained implementation | Saturn advertises `0`, effectively big-endian 3-byte IQ only. |
| `23` | Beta version flag | Mismatch | Saturn uses this byte for `P2APPVERSION` / `P3APPVERSION`. |

Implementation references:

- `sw_projects/P2_app/p2app.c`
- `sw_projects/P3_app/p2app.c`
- `sw_projects/common/saturnregisters.h`

### General Packet To SDR

Saturn currently handles:

- port remapping for DDC/DUC/high-priority/mic/speaker/wideband
- wideband enable and packet settings
- protocol flag byte `37`
- timeout byte `38`
- PA/Apollo byte `58`
- Alex enable byte `59`

Important note:

- v4.4 allows the host to select endian/data format via General byte `39`, but
  Saturn does not consume that field.
- This is consistent with Saturn discovery byte `22 = 0`, which advertises no
  alternate endian/data-format choices.
- `SetFreqPhaseWord(...)` is currently a no-op in the common Saturn register
  layer, so Saturn should be treated as phase-word-only in practice.

Implementation reference:

- `sw_projects/P2_app/generalpacket.c`
- `sw_projects/P3_app/generalpacket.c`

### DDC Specific Packet

Saturn implements:

- ADC count at byte `4`
- ADC dither/random at bytes `5-6`
- DDC enable bitmap at bytes `7-8`
- per-DDC ADC/rate/sample-size starting at byte `17`

Important note:

- Saturn's DDC synchronisation handling is explicitly marked in the code as
  not matching the original spec intent.

Implementation reference:

- `sw_projects/P2_app/IncomingDDCSpecific.c`
- `sw_projects/P3_app/IncomingDDCSpecific.c`

### DUC Specific Packet

Saturn implements the documented v4.x fields used by Thetis/P2 clients,
including:

- CW/keyer data
- CW ramp period at byte `17`
- mic/line options at byte `50`
- Saturn `3.5mm` vs `XLR` mic selection at byte `50` bit `5`
- line gain at byte `51`
- RX attenuation during TX at bytes `58-59`

Implementation reference:

- `sw_projects/P2_app/IncomingDUCSpecific.c`
- `sw_projects/P3_app/IncomingDUCSpecific.c`

### High-Priority Data To SDR

Saturn implements:

- run/TX bits
- DDC frequencies
- DUC frequency
- TX drive
- CAT over TCP port at bytes `1398-1399`
- transverter/audio/user-output control
- Alex words
- RX attenuator bytes `1442-1443`
- CWX bits

Saturn does not currently implement:

- `ClientControl` at bytes `1396-1397`

Implementation reference:

- `sw_projects/P2_app/InHighPriority.c`
- `sw_projects/P3_app/InHighPriority.c`

### High-Priority Status From SDR

Saturn implements:

- PTT bits
- ADC overflow bits
- power/voltage readings
- FIFO overflow/depth fields introduced in v4.3
- ADC max-magnitude fields at bytes `39-42`
- user analog/user input fields

ADC max-magnitude note:

- Saturn code labels bytes `39-40` and `41-42` as `ADC1` and `ADC2` peak hold.
- The v4.4 spec labels those same positions as `ADC0` and `ADC1`.
- The byte positions match the new v4.4 fields even though the ADC numbering
  terminology differs.

Implementation reference:

- `sw_projects/P2_app/OutHighPriority.c`
- `sw_projects/P3_app/OutHighPriority.c`

## Default Ports

Saturn `P2_app` and `P3_app` use the same default port map:

- `1024` command/discovery
- `1025` DDC specific in / high-priority out
- `1026` DUC specific in / mic out
- `1027` high-priority in / wideband 0 out
- `1028` speaker in / wideband 1 out
- `1029` DUC IQ in
- `1035-1044` DDC IQ out

This matches the current Saturn P2/P3 compatibility assumptions and the
existing Thetis regression checklist.

Implementation reference:

- `sw_projects/P2_app/p2app.c`
- `sw_projects/P3_app/p2app.c`
- `sw_projects/P3_app/Thetis_P2_Compatibility_Regression_Checklist.md`

## Packet Sizes

Current Saturn packet payload sizes:

- discovery reply: `60`
- DDC specific: `1444`
- DUC specific: `60`
- high-priority to SDR: `1444`
- high-priority from SDR: `60`
- microphone: `132`
- speaker audio: `260`
- DUC IQ: `1444`
- DDC IQ: `1444`
- wideband: variable, typically based on General packet settings

Implementation reference:

- `sw_projects/P2_app/*.h`
- `sw_projects/P3_app/*.h`

## Stream Framing Notes

### DUC IQ

Saturn expects:

- packet size `1444`
- bytes `0-3`: sequence
- bytes `4-1443`: payload
- `240` IQ samples per frame

Implementation reference:

- `sw_projects/P2_app/InDUCIQ.c`
- `sw_projects/P3_app/InDUCIQ.c`

### DDC IQ

Saturn sends:

- packet size `1444`
- bytes `0-3`: sequence
- bytes `4-11`: timestamp (currently zeroed)
- bytes `12-13`: bits per sample = `24`
- bytes `14-15`: sample count field
- bytes `16-1443`: sample payload
- `238` IQ samples per packet

Implementation reference:

- `sw_projects/P2_app/OutDDCIQ.c`
- `sw_projects/P3_app/OutDDCIQ.c`

### Microphone

Saturn sends:

- packet size `132`
- bytes `0-3`: sequence
- bytes `4-131`: `128` bytes microphone payload

Implementation reference:

- `sw_projects/P2_app/OutMicAudio.c`
- `sw_projects/P3_app/OutMicAudio.c`

### Speaker Audio

Saturn expects:

- packet size `260`
- bytes `0-3`: sequence
- bytes `4-259`: `256` bytes speaker payload

Implementation reference:

- `sw_projects/P2_app/InSpkrAudio.c`
- `sw_projects/P3_app/InSpkrAudio.c`

## P2_app vs P3_app

For Protocol 2 compatibility purposes:

- default port numbers are the same
- packet sizes are the same
- discovery reply field layout is the same
- DUC-specific field usage is the same
- high-priority field usage is the same
- ADC max-magnitude status fields are present in both

The main protocol-visible difference found in this comparison is:

- discovery byte `23` carries `P2APPVERSION` in `P2_app`
- discovery byte `23` carries `P3APPVERSION` in `P3_app`

## Change Risk Guidance

These fields should not be changed casually.

Low-risk changes:

- implement `ClientControl` at bytes `1396-1397`, provided zero remains the
  default and existing clients continue to work unchanged
- stop using discovery byte `23` as an app-version field

Medium-risk changes:

- change discovery byte `20` from `4` to `10`

High-risk changes:

- change discovery byte `12` from `43` to `44`

Reason:

- clients may branch on discovery fields during device enumeration, UI setup,
  and connect/startup logic
- discovery changes are more likely to affect compatibility than adding a
  previously-unused field with a zero default

## Current Recommendation

Keep discovery-reply behavior stable unless there is a concrete client-driven
reason to change it.

The safest near-term alignment step is:

- implement `ClientControl` at bytes `1396-1397`

The changes most likely to affect client connection behavior are:

- discovery byte `12` (`43` -> `44`)
- discovery byte `20` (`4` -> `10`)

## Recommended Order For Future Alignment

1. Implement high-priority data `ClientControl` at bytes `1396-1397`.
2. Decide whether discovery byte `23` should become spec-correct beta flag
   behavior or remain a Saturn-private app-version signal.
3. Add packet-capture validation for discovery reply fields before changing
   byte `20`.
4. Treat any change to discovery byte `12` as a compatibility event requiring
   client smoke testing against:
   - Thetis
   - piHPSDR
   - deskHPSDR

## Guardrails

Before changing discovery bytes `12`, `20`, or `23`:

1. Capture baseline discovery and startup traffic.
2. Test connect/start/stop against at least one known-good Thetis build.
3. Document the change in this file and in the relevant app changelog.
4. Keep a rollback path ready if client enumeration changes unexpectedly.
