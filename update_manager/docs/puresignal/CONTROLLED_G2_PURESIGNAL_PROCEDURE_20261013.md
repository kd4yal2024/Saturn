# Controlled G2 PureSignal host candidate procedure

This staged procedure prepares a separate hardware review of the default-off
host candidate on the CM4/Trixie G2 at 192.168.0.139. It does not authorize
deployment or transmission. Keep firmware 1.31.002 / 0x53460004, the kernel
driver and RXC1 polling unchanged. Review one backend at a time, with one
acquisition reader and one TX/native owner. Preserve the known Legacy release
and its configuration as the host-only rollback target.

## Prerequisites and recorded identities

Before scheduling execution, record the reviewed candidate's source pin and
binary SHA256 from review.json, installed release path/hash, service/environment
backup, native WDSP version/archive/header identities, configured backend,
kernel/XDMA identity, and firmware/build readback. Native build-info proves the
linked candidate identity, not which executable an installed service selects.
Verify the selected executable separately. No automatic updater activation is
part of this procedure.

No-RF review starts with managed RF permission false, no operator connected,
known RF inhibit and a verified rollback plan. Candidate launch must not create
a second Legacy TX engine or C2H reader. On any constructor, topology, layout,
native identity or scheduling refusal, stop the candidate and retain the failed
evidence. Do not substitute Legacy underneath a managed session.

RF review requires a separate approved window, verified ANT1 50-ohm dummy-load
path, power-rated connections/coupler, and the operator's inline LP700. Disable
unattended/repeating keying. Keep an independent operator stop available.
LP700 power and SWR are not RF IMD measurements; no spectrum qualification can
be inferred from them alone.

## Feedback levels and delays requiring approval

Fill this worksheet separately for P2 and Direct. Empty entries prohibit
operational adaptation. Existing fixture values 0.6121, 0.1, 1e-6, 9 dB and zero
delay are synthetic replay inputs, not hardware qualification.

| Parameter | Unit and exact meaning | Required entry or initial proposed condition |
|---|---|---|
| Feedback route | Physical source and layout | ADC1 RF feedback plus returned TX reference, paired DDC0/1 at 192000 complex pairs/s, signed 24-bit components divided by 8388608. P2 source selects 0/2; Direct register selects 0/3. Direct retains DDC6 at 384 kHz. Verify synchronization on installed RXC1 rather than assume it. |
| Attenuation | ADC1 TX attenuation, dB | Proposed initial 31 dB, maximum supported attenuation; verify readback/control mapping. Change only while clean/off, in one approved step at a time. Record actual ADC1/RX/ADC2 state before and after each step. Do not lower attenuation merely to force a fit. |
| Hardware peak | Normalized returned-reference amplitude | Measured/calibrated per backend and native scale. Required value for SATURN_PS_HARDWARE_PEAK; no measured G2 value exists in this package. |
| Active reference fraction | Reference complex RMS / verified hardware peak | Required approved SATURN_PS_ACTIVE_REFERENCE_FRACTION. For N pairs RMS=sqrt(sum(I^2+Q^2)/N). Establish from speech pauses, tone envelope minima, missing, mistuned and unrelated feedback cases. |
| Feedback RMS floor | Normalized complex RMS, not watts | Required approved SATURN_PS_FEEDBACK_RMS_FLOOR above measured noise-only feedback and below the lowest suitable active feedback. Noise-only/mistuned replay must fail qualification; near-zero detection alone is insufficient. |
| Component rail limit | max(abs(I),abs(Q)), normalized | Required approved SATURN_PS_COMPONENT_LIMIT with headroom from measured clipping onset. Equality trips SevereOverdrive. This is not a complex-envelope or calibrated overload counter threshold. |
| Requested delay | Signed seconds | Required measured SATURN_PS_DELAY_SECONDS, using synchronized reference/feedback alignment at 192 kHz (one sample=5.208333 us). Record method, sign and residual error, and confirm the native sign convention offline. Do not infer it from browser/audio latency. |
| Loop delay | Nonnegative seconds | Required approved SATURN_PS_LOOP_DELAY_SECONDS for the pinned native loop, separately from sample-path alignment. |
| Settling and freshness | Nanoseconds | Required SATURN_PS_SETTLING_NS and SATURN_PS_FEEDBACK_MAX_AGE_NS after measuring keyed layout stability and worst observed acquisition/queue delay. Observe intervals, not just publication age. Source enforces freshness <=1 s; that validation range is not an approved safe operating limit. |
| Source and lifecycle limits | Nanoseconds | Required mic age, source-stall, TX-duration, authority, cleanup and startup limits. Measure on ARM under empty/busy queues, release/disconnect and native fitting. No x86 fixture deadline proves CM4 compliance. |

The existing operational admission policy does not by itself prove arbitrary
feedback suitability. If these measurements cannot establish usable limits,
keep strict/default admission and RF permission inhibited. Do not fabricate
ADC/DDC clear brackets, change polling cadence or increase deadlines silently.

## Proposed execution sequence after separate approval

1. Preserve logs, identities and rollback state. Run the matched ARM executable
   first without RF, verify dynamic linking/native identity and explicit shared
   selection, then observe receive IQ/audio and startup cleanup. Repeat stop and
   process shutdown with no operator. Restore Legacy if any identity, startup,
   receive or cleanup check fails.
2. Review the feedback worksheet and proposed protection limits. A low-power
   window may gather feedback evidence with adaptation disabled only if a
   separately reviewed acquisition diagnostic is available. This package does
   not provide a magic mode that overrides the controller's disabled state to
   arm RF. If the current entry points cannot gather required evidence safely,
   stop for a bounded diagnostic review; do not bypass the lifecycle gate.
3. Proposed first RF condition: 7.200 MHz USB into ANT1 dummy load, commanded
   power setting 1, ADC1 TX attenuation initially 31 dB, one operator-controlled
   short window, and independent LP700 observation. Stop if LP700 PEP reaches
   5 W or SWR reaches 1.5:1, or the instrument/operator reports uncertainty.
   These are proposed review limits, not a granted RF approval. The host's
   SATURN_PS_MAX_WATTS=1 limits the tx_drive setting in this candidate; it is
   **not a calibrated 1 W physical power ceiling**. The earlier FPGA/LP700
   discrepancy must remain visible. Never increase drive to chase native fit.
4. With approved backend-specific limits and suitable feedback, test tagged PCM
   first, then SATP separately. Record reference/RF RMS and peaks, sample counts,
   oldest observations, sequence continuity, attenuation, delay, native status,
   requested drive, FPGA meter values and LP700 power/SWR. Record positive
   overload indications and whether unclipped qualification is unavailable.
5. Exercise ordinary release during active correction and fitting, fresh re-arm,
   keyed client disconnect, feedback loss and process shutdown, each as a
   separately controlled trial. Host release must close output immediately,
   drain permitted writes, complete backend stop, restore acquisition and then
   complete native cleanup. Verify RF falls on the external monitor; host
   completion alone is insufficient. Measure notification-to-inhibition,
   backend stop, layout restoration and native cleanup independently.
6. Verify saved layout/ADC controls and normal receive after each trial. On a
   fault, require completed cleanup plus a fresh authorized action before any
   new generation. Acknowledge is not PTT. Retain failed evidence and exit the
   hardware window on an unresolved discrepancy.

## Stop criteria and evidence limits

Immediately request release and independently remove RF permission on known
ADC overrange, component guard trip, excessive power/SWR, wrong/unrelated
feedback, missing active RF feedback, stale or future observations, sequence or
framing loss, queue/FIFO faults, wrong generation/source/layout, disconnect,
configuration change, or unexpected process/native error. Release handling
takes priority over cleanup. A stuck/failed authority receipt or restoration
means failed shutdown, not successful unkey. Use the independent operator
stop if external RF persists; do not wait for a native reset to terminate RF.

Record backend detection delay separately from response after notification.
P2 stop means a complete local command burst, not FPGA acknowledgment; Direct
means shutdown/control readback, not physical RF proof. Existing periodic
telemetry may detect a hazard later and is not fast overload protection.
Bridge-ingress timestamps are not exact RF-output/capture timestamps, and P2
packets do not carry firmware-generation tags.

No external RF spectrum measurement is available in the supplied evidence.
Consequently this procedure can establish bounded host behavior, feedback
integrity and independently observed power/stop behavior, but cannot claim RF
IMD improvement or unclipped spectral qualification. Those require a separate
appropriate measurement review.

## Host only rollback

Unkey and verify RF-off independently; stop the shared candidate and wait for
its explicit cleanup result. Restore the preserved Legacy executable, release
selection and environment through the existing release mechanism only after
that state is verified. Do not restore over a running managed owner or start a
parallel native engine. If cleanup is Blocked, keep RF inhibited and investigate
the failure before resuming any transmitter. Firmware, driver and poller stay
unchanged throughout rollback.
