# Proposed RF feedback measurement procedure

This procedure is for review. The current assignment authorizes offline
implementation and replay only. No live keying or adaptation is authorized.
MeasurementOnly is implemented as a typed internal request, not a live browser
control. A reviewed invocation, matched host candidate and separate hardware
authorization are prerequisites to running any keyed step below.

## Bench approval and preservation

Confirm the actual antenna route, coupler/reference connections, approved
attenuation, independent stop arrangement and inline instrument before the
window. The reported bench has an LP700 and a 1500 W dummy load; that report
does not substitute for checking the current 50-ohm load, ANT1 routing, cable
condition, meter mode, coupler limits or G2 input limits. No higher-power test
is implied by the load's rating.

Before any keying, approve specific frequency, drive ceiling, forward-power
ceiling, SWR trip, duration, settling interval and stale/missing-feedback
deadline. Leave these blank until reviewed bench values exist. The 60-second
software ceiling is not a recommended RF duration. Start with the separately
approved low-power point and feedback attenuation; do not increase drive to
force a fit. Keep adaptation disabled.

Preserve the actual installed release, configuration, effective environment,
backend selection, native/firmware identities, executable hashes and logs in a
protected backup. Latest verified return was release `42ff0919`; recheck the
actual current pointer before every future window. Stop GoldMarsh updates,
installers and other maintenance explicitly. The radio lock alone does not
cover every installer path; the exclusion defect remains open. Do not change
firmware, driver, RXC1 arming/polling, display transport or TCP settings.

## Diagnostic source and acquisition

Use one shared native/TX owner and each backend's existing acquisition owner.
P2 retains its receive socket. Direct XDMA retains its sole C2H reader, performs
generation-specific layout preparation and restores the saved layout and ADC
controls on stopping. Do not start a second reader to obtain samples.

The synchronized reference is the returned DUC/DDC reference, not browser PCM.
Record the route, physical ADC, reference/RF source selections and feedback
sample rate with the generation. Verify the source layout through the same
preparation completion path before accepting a capture. Use identical recorded
PCM stimulus through the normal native TX chain on each backend. Keep source,
model, drive and attenuation constant within a capture.

## Measurement sequence after separate authorization

1. Check native identity and clean owner state with RF permission disabled.
   Confirm no pending fitter or applied IQC curve. The measurement action must
   require a fresh authenticated operator request, explicit duration and fresh
   generation. Ordinary PTT or reconnect must not select it.
2. If separately approved, collect receive-only baseline pairs using the
   reviewed diagnostic layout. Record noise and zero-reference behavior at
   each approved attenuation. Do not synthesize a paired return reference when
   the backend does not supply one.
3. Request one short MeasurementOnly capture at the approved low-power point.
   Native fitting, automatic calibration and correction application remain
   disabled before key receipt, on periodic ticks, before and after rendering,
   and during feedback admission. A state inconsistency stops the action.
4. Feed broadband deterministic, band-limited PCM to estimate alignment. Repeat
   separately with recorded speech/pauses and a known two-tone envelope to
   characterize RMS, peaks and minima. A two-tone alone cannot resolve periodic
   delay ambiguity. Every stimulus change belongs after unkey and completed
   cleanup, in a new authorized generation.
5. For each window, retain the raw reference/RF bit-preserving samples, original
   arrival times, interval/sequence, native state evidence, command snapshots,
   continuity faults and all overload brackets. Retain unsuitable samples and
   their reasons. Mark Unknown when interval overload coverage is unavailable;
   do not increase polling to manufacture clear evidence.
6. Record the LP700's displayed power and SWR, meter mode, calibration identity,
   instrument timestamp and observation interval alongside the capture. Mark
   whether power is average, peak or PEP. Store instrument values separately
   from FPGA readings; investigate disagreement rather than adjusting samples
   to agree. The recorder's LP700 field stays null until genuine external
   observations are attached with their provenance.
7. Release immediately at the duration bound or any stop condition below.
   Wait for the matching backend stop receipt, acquisition restoration and
   native cleanup before exporting evidence or admitting another action.

## Delay analysis and uncertainty

For contiguous broadband captures, use the supplied integer correlation
estimator with an explicit search range. A positive result means RF arrives
later than the returned reference: RF[n + lag] matches reference[n]. Reject
boundary peaks and ambiguous competing peaks. Reject captures with stale,
discontinuous, nonfinite or known-clipped samples. Missing overload coverage
remains Unknown even if a lag can be estimated.

Report lag in signed samples and seconds, correlation/competing peaks and
quantization separately. One 192 kHz sample is 5.208333 microseconds; the
half-sample integer term is not total uncertainty. Repeat independent windows
and establish spread, clock accuracy, path stability and occupied-band
fractional-delay residual before reporting a total uncertainty. The current
analyzer intentionally leaves total uncertainty null. Apply any proposed
fractional delay and its WDSP sign convention first in the pinned native offline
replay, never infer SetPSTXDelay's sign from this analyzer alone. Measure loop
delay separately from browser/transport round-trip latency.

## Stop criteria and return

Release, disconnect, stale or absent feedback, source stall, queue overflow,
sequence/framing/FIFO faults, configuration changes, known overload, unexpected
native PS/IQC state and approved LP700 power/SWR limits terminate output. The
software does not ingest or enforce LP700 limits automatically; an independent
operator stop must enforce those limits unless a separately reviewed instrument
integration exists. Slow telemetry supplements evidence; it is not fast
overload protection.

Record detection delay separately from the response after notification. P2's
completed local command burst is not RF-off acknowledgment. Direct control
readback is not measured physical RF power. Observe physical cessation
independently; retain faults and Blocked restoration without claiming cleanup.

Return to the preserved installed release, leave the runtime RF inhibit in
place, and use the bounded fresh-process telemetry wait followed by receive
delivery observation. Release maintenance exclusion only after that return
passes. Runtime inhibition does not survive reboot. A failure preserves inhibit
and evidence; it does not justify automatically killing an uncertain owner,
starting a second backend or enabling adaptation.

## Outcomes requiring review

Supply raw capture links and quantitative level/noise/attenuation distributions,
delay sign and uncertainty, backend-specific detection delays, measured stop
behavior and proposed numeric feedback limits. No defaults become production
limits from synthetic replay. With only LP700 power/SWR and no external RF
spectrum measurement, this procedure does not qualify RF IMD improvement.
