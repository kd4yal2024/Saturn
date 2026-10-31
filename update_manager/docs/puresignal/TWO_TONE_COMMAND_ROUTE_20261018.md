# Legacy Direct-XDMA two-tone command route

This slice repairs the demonstrated command rejection. It does not establish
physical RF output, fix the earlier tagged-PCM stimulus path, enable PureSignal
fitting/correction, or resolve the LP700 SWR difference.

## Production change

- [Direct command handler](../../saturn-bridge/src/xdma_backend.rs) accepts
  operator two-tone configuration with normal RF permission enabled. Selecting
  the generator does **not** arm or key. The existing authorized PTT, RF gate,
  IQ qualification, owner and watchdog paths are retained.
- [Remote button](../../templates/saturn-remote-next.html) sends generator
  settings/selection before its separate PTT request, allowing the self-sourced
  generator to work without microphone input, including an unpaired SATP source.
  If PTT cannot start, it clears the selection. An action counter prevents an
  old asynchronous completion from changing a newer button action.
- Host result: `tx_two_tone_result:0,<requested>,accepted|rejected,<reason>,<enabled>;`.
  `Configured` means dispatch/enqueue acceptance, **not native application or RF**.
  `ReleaseFirst`, `Stopping`, `NotOperator` and `OwnerUnavailable` are explicit
  rejections. The canonical `tx_two_tone` state remains authoritative.
- A keyed microphone transmission cannot silently switch to generated tones;
  release first. Disabling an active generator uses the existing Disarm path.
  Release/disconnect/watchdog cleanup clears generator intent in the model;
  [TCI cancellation](../../saturn-bridge/src/tci/command_queue.rs) also removes
  queued generator-enable commands belonging to the cancelled session.
- A short stop-pending state prevents a fresh arm from preceding consumption of
  the previous Unkeyed event. Otherwise that old event could clear the new
  request's control-watchdog state. There is no automatic queued re-arm,
  no new power/time limit and no claim that Unkeyed proves physical RF-off.
- Managed PS and tagged-PCM behavior are not changed by this Legacy repair.

## Minimum connected diagnostics

[TX signal capture worker](../../saturn-bridge/src/tx_signal_capture.rs) is
explicitly opt-in through `SATURN_TX_DIAGNOSTIC_DIRECTORY`, an absolute **new**
directory under a writable private parent. Unset means no diagnostic worker.
Existing directories are never overwritten. A failed optional capture setup is
logged and does not change RF permission or normal TX behavior.

Hooks stay in the existing owners:

1. [Existing TX loop](../../saturn-bridge/src/tx_thread.rs) assigns a diagnostic
   arm sequence. This is correlation metadata, **not authorization**. Configuration
   identity remains unavailable (`config=0`, explicitly marked in context.json).
2. [WDSP](../../saturn-bridge/src/wdsp.rs) retains a short output window after a
   successful native exchange. Deliberately discarded startup-settling exchanges
   do not consume that running-output diagnostic window. No DSP setting changes.
3. [Actual H2C writer](../../saturn-bridge/src/xdma_tx_radio.rs) records submission
   time, requested bytes and the actual write return/short/error. Its aligned
   buffer cannot change during the synchronous write: the exact submitted
   payload is copied after return, retaining the original submission timestamp.
   `BeforeOutputIq` identifies those submitted-buffer values, not RF samples.
4. Logical I,Q signed24 **integer** values are retained with packing denominator
   8,388,607; the worker also reconstructs exact physical Q,I signed24-BE `.bin`
   payloads. This is distinct from feedback's signed24/8,388,608 scaling.
5. Successful key readback records `[gpio, tx_config, tx_duc, dac_control, alex]`
   as five exact numeric words in a KeyAsserted timeline record. Subsequent
   high-priority frequency/drive writes record `[frequency_hz, dac_drive_code]`
   in ControlChanged. Shutdown records control-write completion or rejection.
   Cached keyed booleans and completed control writes are **not continuous RF
   readback or proof of physical shutdown**. No extra polling/C2H reader is added.

The producer uses the accepted preallocated/try-lock tap. Eight slots, 8,192 f64
components/record, 256 total records; sample windows are at most 10 Hz per point.
Drops, budget exhaustion and export errors are recorded separately. Formatting,
buffered disk export and raw-byte reconstruction run on the diagnostic-only
worker, not the TX thread. The worker owns no radio/DMA/native resources. It is
asked to stop without joining from TX/unkey; process exit may leave an incomplete
export. Absence of a final capture-summary is not a TX cleanup receipt.

This is the minimum Legacy/Direct output instrumentation. It adds neither a live
PS telemetry stream nor new feedback acquisition. Original sample/RF observation
age and overload coverage remain unknown; timestamps describe host processing.
The existing imported Monitor can display analyzed captures after export.

## Tests and evidence

Tests traverse real WebSocket parsing/operator arbitration, the production
Direct command handler, the actual Legacy TX scheduling loop, real WDSP, actual
packing and actual `write_at` on an **ordinary-file backend fixture**. No G2 RF
is initiated by these tests.

Positive control requires native nonzero IQ, more than 25 completed fixture
writes and key/control evidence without a single microphone frame. The same
pipeline exercises generator disable, a fresh PTT that does not inherit tones,
fresh generator selection, PTT release and actual operator WebSocket disconnect.
Assertions inspect full file-backed hardware shutdown, not merely an early
Unkeyed event. A read-only DMA fixture FD causes a real failed write; it must
never key, and the diagnostic rejection is retained.

The original file fixture did not model keyed FIFO draining. The first extended
replay exposed a fixture pacing timeout; a separate, explicit **test-only**
consumer model now frees space after keying and completed writes. This is not a
production FIFO threshold, firmware change, hardware timing model or fix to a
measured G2 FIFO fault. The initial failure evidence is preserved separately.

Production-handler tests also cover configuration without arm, RF-disabled
operation, closed owner channel, keyed source-switch rejection, pending-stop
rejection, release/disconnect and viewer rejection. Web tests cover the explicit
host result. The actual Chrome Monitor test now verifies that selecting an
oversized file clears both panel and pop-out; a selection counter also discards
late reads of previously selected files.

See the handoff's build identity and preserved logs for exact executed commands,
counts and results. A cross-built ARM candidate still needs installed identity
and receive verification before Jerry keys it.

## Operator bench continuation

Keep normal Legacy TX permission and PS **off**. No firmware, driver, RXC1 arm,
power curve, SWR trip or duration-limit changes belong to this repair.

After the matched candidate/UI is installed and RX verified: refresh Remote,
choose Legacy while disconnected, connect, select 7.200 MHz LSB and start at
5 commanded W on the already confirmed ANT1 dummy load with LP700 monitoring.
Use TX / Two-Tone Test, 700/1900 Hz, 0 dB, 0 ms delay and LSB inversion enabled.
The operator clicks the test button to request TX and clicks again to stop.
No speech or microphone audio is required. The assistant does not key it.

First confirm audibility on the 101D and record LP700 power/SWR. Correlate the
same run with native output, submitted payloads, completed writes and control
events. Then compare Thetis at the same externally measured power, frequency,
mode, antenna/load, LP averaging/PEP setting and correction state. These
measurements—not packet counts—decide RF/SWR and eventual IMD conclusions.
