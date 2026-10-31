# MeasurementOnly offline implementation review

## 2026-10-17 measurement microphone-gain isolation (not yet deployed)

The diagnostic action temporarily selects **0 dB (unity)** microphone-panel
gain before arm/key, with build-matched native readback checked before and
during MeasurementOnly. The committed normal microphone gain is not changed.
Rejected quiescent admission restores it immediately; release, fault and
disconnect restore it through the existing cleanup/channel-recreation path
before re-arm. Other microphone DSP settings remain unchanged by this override.

The host advertises `saturn_ps_measurement_mic_gain_db:0,0;`; the PS panel must
not claim this policy for older hosts that do not advertise it. Normal
`tx_mic_gain:0,<dB>;` readbacks now update the browser slider, including zero.
Previously a cached slider value could disagree with the committed host gain.
Zero dB means unity, not zero signal. This correction alone does not establish
physical RF output or RF power calibration.

## 2026-10-15 host diagnostic follow-up

The original review below is historical. The current source adds a live-capable,
explicitly selected diagnostic entry; this does not change Legacy defaults or
authorize adaptation. A new matched ARM build and an attended hardware window
are required. Do not use the earlier no-RF ARM kit for this diagnostic.

The selected shared owner additionally requires `SATURN_PS_MEASUREMENT_ONLY=1`,
`SATURN_PS_FEEDBACK_INPUT=rx-bypass`, an existing owner-private absolute
`SATURN_PS_MEASUREMENT_DIR`, and one approved
`SATURN_PS_MEASUREMENT_FREQUENCY_HZ`. Strict admission, a maximum **commanded**
drive of 5 W and an owner duration no greater than 1,000 ms are enforced.
All existing explicit timing, native identity, topology and RF-permission
requirements still apply. Start with both RF permissions inhibited. The mode
rejects ordinary PTT and settings that enable fitting/correction; it is not a
general TX unlock or a silently selected fallback.

The authenticated current operator sends one standalone
`saturn_ps_measurement:0,start,<duration-ms>;` request. Authorization and its
timestamp are captured before queuing. The owner publishes `queued`, then
`admitted` or `rejected,<reason>`; admitted does not mean physical RF. Completed
stop, acquisition restoration and native cleanup precede `complete,<count>,<fault>`.
Only then may `saturn_ps_measurement:0,export,<capture-id>;` write raw evidence.
`status` permits reconnect inspection and `stop` requests immediate cancellation.
Malformed mixed batches cannot swallow ordinary release. A viewer cannot key
or cancel the operator by exhausting its own reply queue.

Export uses the host-selected directory, exclusive owner-only files, durable
file/directory synchronization and no-replace rename. Failed/partial files and
records survive an export failure for review/retry. Serialization and file I/O
run on the control thread, not the native/TX scheduler. Completed but unexported
records block another capture. Uncertain cleanup remains Blocked and cannot be
exported/re-armed as a successful measurement. Records in memory are not durable
until export succeeds; process loss can lose them.

[ps-measurement-client.mjs](../../saturn-bridge/scripts/ps-measurement-client.mjs)
defaults to description-only and opens no connection. Its explicit `--run`
requires the exact source-content pin, approved frequency, loopback TCI (normally
an SSH tunnel), operator role, diagnostic capability and an attended ANT1
dummy-load/LP700 confirmation. It requests **1 commanded W for 500 ms** by
default. An explicit `--commanded-watts=5` selects 5 commanded W; values outside
1..5 are rejected before opening a socket. Commanded power is not a calibrated
LP700 reading, and no automatic increase chases a meter target. It emits bounded
700/1900 Hz tagged PCM through the normal native chain,
and waits for cleanup and export. It never enables PureSignal or sends ordinary
PTT. No automatic retry follows a fault. This stimulus establishes neither
broadband delay nor speech/Opus qualification. A two-tone alone has periodic
delay ambiguity.

For the confirmed external sampler connection, both acquisition paths select
RX BYPASS (RX1 Alex word `0x1800`: PS input plus filter bypass). Direct XDMA
verifies the route with the layout, preserves RX2 and restores the exact saved
register after stopping. P2 retains its distinct command-completion guarantee.
The reported Model 7999 sampler is specified for 1.5–35 MHz; 6 m is outside this
diagnostic's frequency range. External coupling remains uncalibrated/null and
is separate from the commanded ADC attenuation. Unknown overload coverage
remains Unknown, even when usable paired data are retained.

The attended initial bench limits are LP700 **5 W**, **SWR 2:1**, and at most
**one second** per authorized action, starting at **7.200 MHz**. LP700 readings
are not ingested by this software: these are independent operator stop limits,
not calibrated software trips. The host duration is a request/response deadline,
not a measured physical RF-off bound. Keep a hardware stop available and stop
on unexpected readings; do not raise drive to obtain a fit. No all-band batch
or escalation follows automatically. LP700 power/SWR is not an RF spectrum/IMD
measurement. Initial captures leave `rfImdQualified=false` and actual external
instrument observations null until separately recorded with provenance.

Offline tests exercise the real control handler, the actual `run_inner` owner,
the Node client, matched native DSP, and both UDP/file acquisition adapters.
They use synthetic feedback and are not radio tests. Preserve and recheck the
actual installed release/configuration before any hardware window, exclude
maintenance explicitly in addition to cooperating locks, and retain the bounded
fresh-process Legacy return check described below. Firmware, driver and RXC1
polling remain unchanged.

## Subsequent queued-audio correction (offline, 2026-10-15)

The first attended Direct-XDMA diagnostic attempt terminated with
`QueueOverrun`, retaining 15 pre-key/NativeUnverified paired blocks and completed
cleanup evidence. It did not qualify feedback levels or TX performance. Legacy
was restored with RF inhibited; no automatic retry or power escalation followed.

An offline regression reproduces a staging overrun while acquisition preparation
is pending: the owner drains valid frames from its bounded 16-message ingress
FIFO into a smaller 2,048-sample mic staging ring before rendering can free room.
The correction defers dequeue when the largest permitted frame will not fit.
It enlarges neither buffer, drops no queued frame, and preserves original ages.
Ingress-full faults, stale/source deadlines and out-of-band release remain active.
This reproduces a plausible fault mechanism, not definitive attribution of the
hardware event, whose log did not include per-queue diagnostics.

The positive control also checks audio across key receipt. That receipt advances
the feedback generation and TX epoch, but retains the same authorized output
permit. Bounded mic history may survive only that exact adjacent feedback
boundary, without restamping. Boot, configuration, release, revision or output
permit changes remain invalid; nonadjacent or overflowing epochs are rejected.
A fresh current-authority check precedes output, so retained stamps cannot hide
a genuine authority loss or misclassify a raced ordinary release.

Regression cases require all eight queued blocks to render, reject stale audio
before keying, and complete release/cleanup without further output. Existing
release, re-arm, watchdog and missing-authority cases remain required controls.
This source change is not a new live qualification. The MeasurementOnly ceiling
remains 5 commanded W; a requested 10 W instrument reading needs a separately
bounded candidate/procedure with explicit PEP-versus-average interpretation.
The operator subsequently chose to remain at 5 W. The next attended retry uses
5 commanded W for 500 ms, the unchanged independent LP700 5 W / SWR 2:1 stop
limits, and no fitting or correction. This is a proposed retry, not evidence of
its execution or of an achieved external power reading.

The broader runtime replay also exposed an observation race in its test fixture:
an acquisition stop request can become visible before the owner's end-of-turn
lifecycle publication. The fixture now records the pre-stop ID, distinguishes
that older publication from the pending stop, and requires repeated checks of
the current stop with neither authority nor native cleanup completed while
restoration is withheld. The original failed replay is retained; no production
stop/cleanup condition is relaxed by this test-only correction.

## Managed DUC prefill correction (offline, 2026-10-16)

The attended 5-commanded-W retry stopped with `Stale`. It retained 22 pre-key
`CleanupPending`/`NativeUnverified` blocks, no valid TX-reference qualification,
and completed cleanup. The operator reported relay clicks but no LP700 RF
output. Legacy reception and web health were restored, the 53 preserved regular
files matched, and the original runtime RF inhibit remained in place.

Source review found an independent, reproducible startup defect: the managed
Direct-XDMA adapter wrote an 11-frame zero seed (1,980 FIFO words), then waited
for the 19-frame target (3,420 words) without writing any deficit refill. The
earlier file fixture always advertised the target occupancy and hid the defect.
The original regression fails at 11 versus 19 frames; this is evidence of the
host defect, not proof that every observed scheduling delay has that cause.

The cooperative path now closes the measured deficit using the established
Legacy geometry and eight-attempt bound. Each turn performs at most one H2C
write and never waits in a polling loop. A 500-microsecond monotonic visibility
deadline follows each DMA write; RF stays inhibited through zero prefill and
the first live frame. A final occupancy/underflow check precedes keying.
Cancellation, fresh microphone/IQ gates, source ages, finite action duration,
generation fencing and shutdown readback remain required. Failed prefill does
not enable RF or grant a fresh action.

Unit fixtures explicitly select a controlled backend clock where needed;
production and actual-runtime/native replay retain the real monotonic clock.
New controls exercise the 499,999/500,000 ns boundary, partial occupancy,
nonzero IQ at the file-backed H2C sink, cancellation, attempt exhaustion,
overflow/threshold faults, late underrun and insufficient room. The actual
runtime MeasurementOnly fixture now derives pre-key occupancy from frames
actually written rather than injecting an already-full FIFO. Its positive
control must key and write continued IQ. None of this establishes G2 RF output,
physical RF-off latency or IMD performance. Firmware, driver, polling, power
ceiling and fitting/correction settings are unchanged.

## Original offline review

SilverForge's next review slice adds a finite, explicitly authorized measurement
purpose to the existing shared TX owner. It records paired feedback without
fitting or applying correction. The deliverable is a source delta and local
tests, not an installed release or authorization for RF operation.

## Lifecycle and authority

[MeasurementOnly](../../saturn-bridge/src/puresignal/tx_lifecycle.rs) is a distinct
run mode. Its action requires completed cleanup, disabled PS configuration,
fresh generation-specific operator authority and a nonzero finite duration no
greater than 60 seconds. This ceiling is an implementation bound, not an
approved bench duration. Ordinary `trx` or a previously queued boolean arm does
not select measurement mode. The [typed request](../../saturn-bridge/src/tx_thread/ps_owner_queue.rs)
retains its ingress time; time spent queued reduces its remaining duration.

The [owner scheduling loop](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs)
retains the existing output fence, source-age checks, release mailbox and
one-exchange control steps. The controller admits diagnostic continuity and
age checks but does not call PSCC. Actual native fitting controls, MOX and IQC
run/busy state are checked through the
[build-matched observer](../../saturn-bridge/src/wdsp/ps_observer.c). Its CALCC
layout comes from the exact pinned `calcc.c`, rather than guessed offsets or
configured intent. That host observer changes neither WDSP algorithms nor the
FPGA or kernel driver.

Release, disconnect, stale or missing feedback, queue faults, native-state
inconsistency and configuration changes close output and enter the existing
stop/cleanup sequence. Re-arm requires completed backend stop, acquisition
restoration and native cleanup, then a fresh authorized action. P2 local command
completion and Direct XDMA control readback retain their different meanings;
neither is an external measurement of RF-off time.

## Raw evidence and measurement confidence

The [recorder](../../saturn-bridge/src/puresignal/measurement.rs) retains original
reference and RF components before admission or fitting. Rejected samples stay
in the evidence with both the original admission reason and any recording
defect. A bounded extra slot retains the first overflow-triggering block; later
losses are counted, and the recording is marked incomplete instead of evicting
older evidence. Export belongs after cleanup, outside the active TX scheduling
turn. Terminal fault and cleanup time accompany the records, including decoder
failures that occur before a paired block exists. Storage retention completeness
is separate from completed fault-free capture; unfinished or failed cleanup
cannot be exported as a complete measurement. Command labels must match the
same sealed ADC/drive/attenuation snapshot used to authorize the request.

| Evidence | Meaning |
| --- | --- |
| Component representation | Interleaved I/Q encoded as exact IEEE754 f64 bit strings. Both adapters decode signed 24-bit components divided by 8,388,608; no calibration, clipping or normalization is applied by the recorder. |
| Source identity | Source pin, native identity, backend, route and generation/configuration/epoch fence. Fixture identities explicitly identify synthetic replay. |
| Timing | Original per-pair conservative host arrival, block's oldest arrival and recording time. These are bridge monotonic ingress times, not FPGA capture time or physical RF-output time. |
| Direct partial frames | Oldest original frame time survives completion and 1,024-pair assembly; later arrivals cannot refresh earlier retained samples. |
| Attenuation and drive | Command snapshot fields, nullable when unavailable. They do not certify actual calibrated attenuation, power or LP700 readings. |
| Continuity | Generation-relative interval ordinals, original P2 sequence where supplied and admission fault. A rejected discontinuous interval cannot yield a qualified delay estimate. |
| Overload | Original source, counter generation, interval, observation time, width, saturation, overflow, completeness and validity. Missing or insufficient coverage is Unknown; a known qualifying hit is Clipped. |
| Native state | Disabled fitting, automatic control and correction are only verified when the matched guard passes. A failed/unavailable guard means Unknown even if disabled intent was configured. |
| Levels | Complex RMS, component/envelope peaks and envelope minimum in normalized converter units, not watts or dBm. |

The integer delay analyzer uses mean-subtracted complex correlation. Positive
lag means **RF[n + lag] matches reference[n]**, so RF arrives later than the
returned reference. Negative lag means the opposite. This convention is not a
WDSP delay-setting recommendation. Boundary peaks and competing periodic peaks
are marked unresolved. Half a sample is the reported integer quantization
term; total uncertainty remains unavailable until repeatability, clock and
path errors are measured. At 192 kHz, one sample is 5.208333 microseconds.

Synthetic signals test arithmetic, provenance and cancellation. The export
always reports `rfImdQualified: false`. No test result here establishes RF IMD,
absolute RF calibration or suitability for adaptation. Unknown overload does
not become clear because a delay or level can be computed.

## Legacy return wait

[wait-legacy-return.py](../../release/puresignal-no-rf/wait-legacy-return.py)
addresses the status-file startup race observed during the no-RF ARM window.
It is read-only: it does not start a service, remove a maintenance guard or
change RF permission. Capture the old MainPID and the wall-clock boundary before
the separately authorized Legacy start. Supply the new actual MainPID, preserved
executable hash and boundary to the helper.

The bounded wait retries absent, partially written, old-PID and predating
telemetry. It requires active Legacy service identity, stable process-start
ticks, matching executable SHA256, both runtime RF permissions disabled,
fresh `ready` telemetry from the expected PID and explicit `tx_keyed=false` /
`tx_stream=false`. Process replacement or a reported failed state fails the
return check. Successful telemetry still requires a separate bounded IQ/audio
delivery observation before releasing maintenance exclusion.

## Review boundary

The internal diagnostic ingress is not wired to a browser command or live
endpoint. Default startup remains Legacy and managed selection remains opt-in.
No new ARM image, G2 installation, live keying or adaptation is included. The
accepted no-RF ARM candidate and its earlier evidence remain immutable.

Use the [proposed RF measurement procedure](PROPOSED_RF_MEASUREMENT_20261014.md)
for review before implementing any live entry. It retains firmware 1.31.002 /
0x53460004, the kernel driver and RXC1 polling unchanged. Installed-release
preservation must use the actual current release, not the diagnostic source's
older Git base. The installer exclusion defect remains a separate open issue.

## 2026-10-16 bring-up follow-up: bounded listening stimulus

The later attended FIFO-corrected G2 attempt reached actual managed keying and
nonzero IQ writes. The operator heard TX, but reported that the pulse was too
short. It stopped with `Stale`; this was not a passed RF measurement. Legacy
3f6c0d3c was restored with RF inhibited and 53 preserved files matched.

The shared acquisition loop imposed a 1 ms sleep after every DMA completion,
even with completed buffers waiting. Small reads can arrive faster than 1 kHz;
the exported observation ages rose from 9 to 143 ms. The correction consumes
one buffer per turn without the idle sleep while backlog remains, retaining
the control, shutdown and restoration services on every turn. It does not
retime samples, enlarge a queue or relax the freshness/polling checks. An
actual-runtime three-buffer regression fails with the old wait and checks
both immediate draining and return to idle waiting after the queue empties.
P2 likewise skips the additional idle sleep after consuming a datagram; its
192 kHz / 119-pair stream also exceeds 1,000 packets/s. The existing socket
timeout remains. The longer replay also exposed a fixture-only 4 ms throttle
mistakenly applied to already-paced P2 UDP input; it is now Direct-only. The
original sequence-gap failures are retained rather than treating them as a
radio observation or weakening continuity checks.

The long Direct replay also exposed duplicate pacing: a nominal per-packet
timer was applied in addition to the direct writer's hardware FIFO-space
checks. The direct owner now attempts its single cooperative write each turn,
with the existing FIFO check providing backpressure. P2 retains its UDP packet
timer. No output timestamp is refreshed and old IQ still trips the age limit.

For the separately requested listening/power check, the host may explicitly
select `SATURN_PS_MEASUREMENT_CAPTURE=summary-only`. This remains MeasurementOnly:
fitting/correction readback, fresh operator authorization, source/feedback age,
queue faults, cancellation, stop receipts and native cleanup are unchanged.
The maximum is 45 seconds and 5 commanded W, not unlimited TX. The existing raw
profile remains the default and retains its 1,000 ms host limit.

The summary profile retains bounded aggregate admission/observation counts, not
raw reference/RF arrays. Its distinct `saturn-two-tone-summary-v1` export reports
`rawFeedbackCaptured: false` and `rfImdQualified: false`. It cannot qualify IMD,
delay, unclipped capture coverage or correction performance. Missing overload
evidence stays unknown. This avoids retaining hundreds of MB of raw samples
and constructing an enormous JSON export on the installed 1 GB CM4.

Capability negotiation includes the capture mode and mode-specific duration.
The Node client requires explicit `--summary-only` and a matching host profile
before sending a long stimulus; a raw client cannot silently fall back to
summary. The long-run tests use the actual orchestration, both file/loopback
backend fixtures, tagged PCM and real WDSP with correction disabled. These are
offline tests, not hardware throughput or physical shutdown measurements.

The G2 remains on the preserved Legacy release. No further RF test is authorized
to start unattended: the operator deferred live testing until the evening.
Runtime RF inhibition does not survive reboot. Firmware, driver and RXC1
polling/arming are unchanged; GoldMarsh/main and the installer remain untouched.

## 2026-10-18 Timing Measurement UI (offline implementation)

The PS tab's Measurement stimulus selector now offers **Timing Measurement**
alongside the existing two-tone / silent-carrier test. It requires an explicit
`saturn_ps_stimulus_caps:0,saturn-timing-multisine-v1;` advertisement, USB/LSB,
the existing MeasurementOnly host, clean ReadyOff state, matching frequency,
1–5 commanded W, ARM TX and fresh attended confirmation. Selecting it does not
key RF. Its separately labelled Start button requests
`saturn_ps_measurement:0,start,DURATION_MS,saturn-timing-multisine-v1;`.
Older hosts leave this action unavailable; there is no microphone/two-tone
fallback. Unknown waveform versions and timing requests in AM/SAM/FM are rejected.

The browser prepares one deterministic 48 kHz, 32-component 400–2600 Hz multisine
record before authorization (seed `0x53465031`, 5 ms tapered edges, peak below
0.4), then repeats that 1,000 ms record using the existing tagged-PCM pacing
and cancellation path. This is not a continuous two-tone waveform and its RMS
is lower than the two-tone at the same peak ceiling. Commanded watts are not a
guarantee of measured output power. Release, disconnect, scheduling stalls and
host failures retain the existing stop/cleanup gates. Fitting and correction
stay disabled; this action does not set WDSP delay or hardware peak.

Requested waveform identity follows the sealed request into the generation's
acquisition scope. Held spectra, pop-outs and raw paired-window archives carry
that identity; a later model/UI change cannot relabel retained samples. The
Monitor uses a dBFS spectrum and suppresses two-tone IMD markers/numbers for
timing or silent-carrier captures. Timing captures cannot become an IMD baseline.
These labels describe the requested stimulus, not independent proof of RF content.

Cleanup-gated measurement envelopes retain version, seed, rate and repeat period.
Timing summary exports use `saturn-timing-summary-v1`, explicitly without raw
feedback. Summary-only counts cannot establish delay: review separately retained
raw paired windows when the host archive is enabled. The offline timing reviewer
preserves the waveform metadata, reports signed lag/ambiguity and keeps total
uncertainty and RF qualification unresolved. Neither a successful capture nor a
delay candidate authorizes fitting, correction or another TX action.

Reproducible offline checks: `bash tests/run-puresignal-timing-ui.sh`, from
`update_manager/saturn-bridge`, with an explicit local reviewed WDSP build,
offline observer link and new `PS_TIMING_UI_EVIDENCE_DIR`. The runner pins all
source inputs, serializes native tests, generates the probe, exercises real
Chrome / native WDSP / `run_inner` through P2 and file-backed Direct XDMA,
checks native disabled state, and checks timing suppression/copy in both Monitor
documents. Loopback/file results do not qualify G2 RF operation, measured delay,
overload coverage or physical shutdown timing. This UI slice does not deploy to
the G2 or alter its firmware, driver, polling or calibration profile.
