# Proposed collection of feedback measurements without adaptation

This is a concrete design for the missing diagnostic, not permission to key
the reviewed candidate. No diagnostic is added to the accepted source or ARM
binary in this preparation. Implement it only after the no-RF ARM review and
design approval; then qualify it separately under an authorized dummy-load
window. Confirm the current inline LP700, ANT1 routing, load impedance/rating,
coupler attenuation and operator stop arrangements first.

## One explicit measurement purpose in the existing owner

Add a reviewed MeasurementOnly purpose to the existing TX authority/lifecycle,
not a second native engine or DMA process. Current fresh_tx_action requires
PureSignal enabled; do not enable adaptation to work around it. A dedicated
measurement purpose must permit ordinary uncorrected RF under a fresh, finite,
authenticated action while native PS enable, automatic fitting and correction
application stay false. Export their actual native state as evidence. Never
call PSCC or accept measured samples into a correction fit in this purpose.

Retain output generation fencing, authenticated release/disconnect, bounded
input queues, age checks, output drain, backend stop, acquisition restoration
and native cleanup. Measurement source changes and configuration changes unkey
and retire the generation. Do not add an RF-capable shortcut to this no-RF kit.

## Acquisition and retained evidence

Use the current acquisition owners and existing paired-DDC adapters. Direct
XDMA pauses the sole reader before the same verified layout preparation and
captures scope/time before each read; P2 retains its single receive socket and
existing packet retirement. Preserve the normal receiver and restore saved
ADC controls/layout on every stop. No firmware, driver or polling change.

Deliver pairs to a separate bounded observation sink after decoding and before
any adaptation admission. The sink records, never fits or keys. Never suppress
low-level or unsuitable samples merely because they fail future adaptation
thresholds: mark them for measurement. Store original packed components or
normalized I/Q with exact signed-24-bit/8388608 scaling and clipping policy.

Each capture records backend, reviewed binary/native identities, route, source
generation, RF/reference source selections, sample rate/count, interval ordinals,
oldest observation and completion/publication times, sequence/framing/queue loss,
ADC attenuation, commanded drive, measured LP700 power/SWR and instrument time,
native PS/automatic/correction state, and overload evidence with its generation,
interval/coverage/validity. Missing overload coverage remains unavailable, not
clear. Any loss invalidates that measurement interval. Do not retime queued
samples or call a host timestamp an RF acquisition timestamp.

## Specific measurements

1. Collect unkeyed noise-only RF feedback and zero returned reference after
   approved receive-only layout preparation. Record complex RMS and component
   peaks, distributions and noise variability at each approved attenuation.
2. In an authorized short uncorrected RF window, feed bounded deterministic
   band-limited PCM through the normal native TX chain. Use the returned DUC
   reference, not browser PCM, as the synchronized TX-reference measurement.
   Hold drive and attenuation constant. Begin with the separately approved
   low-power setting and maximum attenuation; LP700 observes actual power.
   Log any disagreement with FPGA meters. No drive increase to force a fit.
3. Repeat a known two-tone envelope and recorded speech/pauses using identical
   uncorrected settings. Measure complex RMS=sqrt(sum(I^2+Q^2)/N), maximum
   component magnitude, complex envelope peaks and minima, returned-reference
   scale, and feedback/reference level ratio. Establish a noise margin and
   minimum usable active feedback from these captures, not from a fixture's
   near-zero constant. Test noise-only, mistuned and unrelated feedback offline
   against the retained captures before approving production limits.
4. Estimate delay from simultaneous complex reference/RF using cross-correlation
   of the broadband deterministic stimulus, then fractional-delay phase slope
   over its occupied band. Record lag sign, integer and fractional samples,
   fit residual and repeated-window uncertainty. At 192 kHz, a sample is
   5.208333 us. A pure two-tone alone has periodic ambiguity and is insufficient.
   Apply the known estimate in pinned WDSP offline replay and verify its delay
   sign convention before configuring hardware. Measure loop delay separately;
   do not substitute browser/audio round-trip latency.
5. Repeat across approved attenuation/drive points only after unkey and complete
   cleanup. Choose hardware peak, active-reference fraction, feedback RMS floor,
   component guard, settling, freshness and response limits from measured
   distributions and path-specific rejection cases. Supply numeric values,
   units, rationale, raw capture links and uncertainty for review. If unsuitable
   feedback cannot be distinguished, do not approve adaptation.

## Independent stop and qualification limits

Each keyed capture is one bounded action with an independent operator stop;
power, SWR and RF duration limits must be approved for the actual bench.
Known overload, native-state inconsistency, missing feedback, sequence/queue
loss, stale source, disconnect or reconfiguration immediately close output and
request unkey. Restoration or cleanup failure stays Blocked. Measure detection
delay separately from response after notification. Slow telemetry is not fast
overload protection, and P2 local command completion is not physical RF-off.

With no external RF spectrum measurement, these captures qualify feedback
levels, alignment and host behavior only. LP700 power/SWR do not prove IMD
improvement. Capture unclipped confidence remains unavailable unless interval
overload coverage supports it.
