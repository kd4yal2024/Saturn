# P2/P3 ADC Peak Status Message

Date: 2026-03-16

## Summary

`P2_app` and `P3_app` version `45` extend the 60-byte outgoing high-priority
status packet to report peak ADC amplitudes for the current message period.

This change was applied in both:

- `sw_projects/P2_app`
- `sw_projects/P3_app`

## Wire Format

The existing 60-byte high-priority packet layout is retained.

New fields:

- bytes `39..40`: ADC1 peak amplitude, big-endian `uint16_t`
- bytes `41..42`: ADC2 peak amplitude, big-endian `uint16_t`

These values are peak-hold values accumulated between successive
high-priority status messages. After each message is sent, the local
peak-hold state is reset and accumulation starts again for the next message.

## FPGA Compatibility

Peak amplitude reporting depends on FPGA support added at firmware version `27`
or newer.

Behavior by FPGA version:

- FPGA `< 27`: peak amplitude fields are reported as `0`
- FPGA `>= 27`: peak amplitude fields are read from the ADC overflow register
  block and encoded into the status packet

The existing ADC overflow bits in byte `5` are unchanged.

## Implementation Notes

- `GetADCOverflow(...)` in `sw_projects/common/saturnregisters.*` now returns:
  - overflow bits as the function result
  - optional ADC1/ADC2 peak amplitudes via output parameters
- `P2_app` and `P3_app` both accumulate per-message peak holds while polling
  for immediate overflow-triggered updates
- peak-hold state is explicitly initialized to avoid uninitialized first-packet
  data

## Optional Runtime Telemetry Export

The packet fields above are always produced on the wire, but exporting them to
local runtime telemetry is optional.

When enabled:

- control file: `/dev/shm/saturn_p23_adc_peak_telemetry.enabled`
- snapshot file: `/dev/shm/saturn_p23_adc_peak_telemetry.json`

Behavior:

- telemetry export is off by default
- `P2_app` and `P3_app` check for the control file at runtime
- when enabled, they overwrite a single latest-snapshot JSON file in `/dev/shm`
  at most once per second
- disabling telemetry removes the control file and stops future snapshot updates
- the last snapshot file is retained in `/dev/shm` until it is overwritten by a
  later enabled run or removed manually

This keeps the feature out of persistent storage and avoids continuous disk
writes while still making the latest ADC peak values available to local tools
such as `/saturn/p23test`

## Versioning

- `P2_app`: `V45`
- `P3_app`: `V45`
