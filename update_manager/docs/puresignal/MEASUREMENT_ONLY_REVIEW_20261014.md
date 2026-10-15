# MeasurementOnly offline implementation review

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
default, emits bounded 700/1900 Hz tagged PCM through the normal native chain,
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
