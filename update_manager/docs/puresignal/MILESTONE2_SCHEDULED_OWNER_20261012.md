# PureSignal milestone 2 — offline scheduled owner migration

Date: 2026-10-12. Isolated SilverForge worktree; base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome and boundaries

The shared lifecycle now runs with mic ingress, native-generated IQ, configuration
transactions and watchdogs through the **same scheduling driver used by the
existing TX thread**. A combined replay connects the actual P2/direct feedback
decoders, queued consumer, real pinned WDSP and actual output adapters. Both
backends establish correction and complete release/reconfigure/fresh-arm/
disconnect/cleanup sequences offline.

This advances migration; it does **not finish the installed production chain**.
TCI fixed-rate mic buffering is migrated in the gated consumer. SATP selection
is a fenced configuration transaction but SATP arming is explicitly refused
until its rate matcher and sample-age provenance are migrated. The real native
scheduler fixture supports a USB mode/filter/mic-gain subset, not the complete
installed speech/EQ/CFC/de-expander/model wrapper. Those remain the next bounded
offline work, not a new firmware investigation.

Production selection, live socket producers, G2, firmware 1.31.002 / 0x53460004,
FPGA, kernel driver, polling, services and deployment are unchanged. The default
legacy TX loop uses the extracted driver with its existing stop/idle-wait policy;
it still refuses gated owner commands and never constructs a second native owner.
No commit/push; GoldMarsh's worktree is untouched. No RF test was run.

## Source-linked pipeline

```text
operator ingress -> sealed credential -> existing TxCommand queue
audio ingress -> original timestamp + generation stamp -> bounded audio queue
P2 / XDMA decoder -> original PairBlock metadata -> bounded feedback queue
                             |
                             v
                shared TX scheduling driver
                lifecycle/release/deadlines FIRST
                             |
                gated owner consumer, one native N
                pscc admission -> fixed-clock mic render
                -> same-exchange IQ -> generation-fenced output
                             |
       stop receipt -> cooperative native cleanup -> ReadyOff
       fresh action (and config if canceled); cleanup is never an arm
```

Relevant sources:

- [Shared driver and wait policy](../../saturn-bridge/src/tx_thread/scheduler.rs),
  used by [existing TX run](../../saturn-bridge/src/tx_thread.rs) and
  [gated owner consumer](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs).
- [Existing mic frame type](../../saturn-bridge/src/tx_audio.rs) and
  `mic_samples_to_mono` in the existing TX thread: source, 48 kHz rate, channel
  count and enqueue time are retained, not reconstructed at dequeue.
- [Lifecycle](../../saturn-bridge/src/puresignal/tx_lifecycle.rs): stop receipts,
  release handling, cooperative native cleanup, deferred configuration.
- [Actual owner adapter](../../saturn-bridge/src/tx_owner_stop.rs) and
  [re-arm/model/output handoff](../../saturn-bridge/src/tx_owner_stop/rearm.rs).
- [Scheduled combined tests](../../saturn-bridge/src/tx_owner_stop/scheduled.rs),
  [native fixture](../../saturn-bridge/tests/support/native_port.rs) and
  [decoder replay](../../saturn-bridge/tests/support/puresignal-controller-native-replay.rs).

## Cancellation and generation rules

`Cancel::release()` closes the atomic output fence and posts the release mailbox
without taking the DMA writer lock or relying on channel capacity. Existing
in-flight permits still drain; none is retroactively described as canceled.
The lifecycle runs before each bounded queue drain, even with no arrivals.

Audio envelopes and configuration snapshots are stamped **before enqueue**.
They include the acquisition fence, output revision and handled release ticket;
audio additionally carries the active output generation. Old envelopes are
rejected after cancellation, cleanup or epoch changes. Generated IQ retains the
mic block's oldest ingress observation and its generation through native render
and final enqueue. Partial packet tails retain their original age as before.

A source/model command carries its complete model snapshot and PS configuration;
it never means “read whatever shared model exists later.” It closes output and
waits for the matching backend stop and native cleanup before installation.
P2 and direct re-arm now use that installed snapshot, rather than P2's old
borrowed model. The explicit offline drive cap cannot be raised by the snapshot.
Two-tone and WFM are refused by this migrated model path.

Release or fault also cancels the lifecycle's staged PS configuration. Release
is rechecked after each bounded native cleanup step, before pending configuration
can be installed. A canceled transaction leaves runtime readiness invalid;
fresh configuration must be submitted before a fresh arm. Neither configuration,
cleanup completion nor fault acknowledgment authorizes TX.

Release during native render is handled before IQ enqueue. A normal cancellation
does not get relabeled as a new `CleanupPending` fault requiring acknowledgment.
An external stop flag requests shutdown and continues receipt/cleanup service;
it does not abruptly destroy the native channel.

## Scheduling and native ownership

Each migrated turn services lifecycle/release first, then at most one command,
four audio messages and one feedback block. Mic input is bounded to four
512-sample blocks. Owner IQ remains bounded to one 4,096-float output plus its
partial packet tail. Production-sized channel construction is not selected;
the test harness uses bounded channels of 16 / 16 / 2 entries.

Mic DSP retains the 512/48 kHz cadence; output uses 240/192 kHz packet cadence.
The migrated consumer sends at most one packet per turn, with bounded cadence
catch-up; this is not a G2 throughput or real-time latency qualification.
Source pauses produce zeros without renewing the source-arrival watchdog or
granting mic qualification. Deadline policy uses original ingress time, not
“freshness” assigned while emptying a backlog.

`ScheduledNative::process` admits `pscc` with **zero DSP exchanges**. `render`
performs the one exchange and returns its IQ. The same native instance performs
stop progress and cooperative cleanup; no `WdspTxEngine` is opened beside it.
A turn which progressed cleanup cannot also render, including a fresh arm
arriving in the cleanup-completion turn. Both fake and real traces assert at
most one DSP exchange per scheduling turn. This is a count bound, not a
wall-clock duration guarantee for a native call, DMA write or driver operation.

## Capability status

| Path | This slice | Remaining boundary |
| --- | --- | --- |
| Sealed operator arm / release / disconnect | Integrated through existing command type and periodic lifecycle | Live ingress/production selection remain off |
| TCI mic / native-generated IQ | Bounded queue, original age, normal mic/IQ key gates, paced output | Installed ingress wrapper/full native TX chain not selected |
| Source and model selection | Snapshot transaction; stop/cleanup before installation; canceled snapshots rejected | SATP rate matcher provenance and complete model wrapper |
| SATP audio | Selection supported; arming safely refused | Do not route it through the fixed-rate TCI implementation |
| Source, duration, feedback watchdogs | Periodic service on empty/busy queues; faults latched | No G2 timing or real overload detection-delay claim |
| P2 output / stop | Real UDP loopback owner; complete unkey command burst | Command completion, not hardware acknowledgment/RF-off |
| Direct XDMA output / stop | Actual owner with ordinary-file control/DMA fixture | Synthetic FIFO/readback, not hardware/RF-off |
| Correction and measurement confidence | Real WDSP reports correction with missing counter evidence | Capture remains `ClipStatusUnknown`; no measured RF IMD |

## Execution evidence

Four new scheduled-owner tests cover multiple scenarios: source/duration deadlines
with empty or busy command queues; feedback loss after correction without another
feedback packet; old audio/IQ/config/arm rejection and disconnected authority;
source selection, cancellation during native render, cancellation during staged
configuration and successful fresh transactions. The lifecycle adds a separate
regression proving release prevents staged PS configuration installation.
The existing standalone reconfiguration replay now explicitly expects its
release-canceled transaction to remain at config 40, then submits a fresh
transaction and verifies config 41. This intentional cancellation-contract
change does not bypass either stop or native cleanup.

The new real-WDSP scheduled test serially exercises both actual backend adapters:
decode -> queue -> lifecycle -> `pscc` -> mic DSP -> output, establishes correction,
cancels old queued configuration/audio, changes the filter via a fresh snapshot,
re-arms only with a new credential, and then disconnects. Original decoder/owner/
native handoff replay also remains. Runtime blocking-reset count is zero; final
IQC flags are zero before the harness's defensive destructor reset.

| Check | Result |
| --- | --- |
| Owner-feature Cargo, native stubs | 46 library + 396 binary passed; 2 existing ignored |
| Default Cargo, native stubs | 24 library + 377 binary passed; 2 existing ignored |
| Combined actual-owner real-WDSP runner | 2 serial tests passed, each covers P2 and direct XDMA |
| Standalone strict / proposal / lifecycle | 24 / 32 / 46 passed; real native replays and numerical canaries passed |
| Formatting / whitespace / runner ShellCheck | Passed |

Current transcripts are `offline-native/scheduled-*`; prior transcripts are
historical. These are SilverForge's executed results, not the external reviewer's
independent evidence. Reproduce from the extracted archive root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
bash update_manager/saturn-bridge/tests/run-puresignal-owner-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
```

Rust/cached Cargo dependencies, compiler/binutils and FFTW development files are
required. Pinned WDSP archive SHA-256 remains
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
It is x86-64 offline material: **never install it on G2**. The synthetic replay
timing/input, loopback command completion and regular-file readback establish
none of physical RF-off, live thresholds, installed-library identity, RXC1
feedback qualification or bounded G2 scheduling/overload response time.

Next offline slice: migrate the SATP rate matcher with trustworthy sample-age
provenance, then wrap the complete installed TX model/DSP processing in this
sole native owner and repeat the combined scheduler tests. Keep live admission
and production selection unchanged pending review. Analyzer/panel work may
continue offline; no FPGA, driver or polling change is indicated.
