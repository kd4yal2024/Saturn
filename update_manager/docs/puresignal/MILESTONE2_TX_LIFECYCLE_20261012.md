# PureSignal milestone 2 — TX-authority lifecycle, offline review cut

Date: 2026-10-12. SilverForge isolated source base: `861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome and boundary

Implemented the **production-facing TX-authority interface and cooperative lifecycle**, and exercised it offline with both paired-feedback adapters, the shared controller and real pinned WDSP. The TX authority in these tests is **simulated**. There is no production authority adapter, live admission, RF test or physical RF-off qualification in this cut.

The host now separates these operations explicitly:

```text
hazard notification or priority PTT release
  -> stop feedback admission / invalidate captures / latch primary fault
  -> existing TX authority fences output and cancels pending key actions
  -> drain in-flight writes, discard queued output, complete controlled unkey
  -> accept current stop receipt (backend-specific evidence, not an RF measurement)
  -> native MOX off; drain fit and reset in yielding single-exchange steps
  -> verify native correction off and IQC run/busy clear; retire generation
  -> off, fault still latched; acknowledge while released
  -> only a fresh authenticated TX action may request a new key
```

The producer barrier is essential: no newly uncorrected IQ may enter an old corrected-drive output generation. An unkey command alone, a UI event alone, or a logical `correction_applied=false` alone cannot establish that barrier. Pending native fitting may still progress while output is fenced; this implementation does **not** claim to freeze an asynchronous fitter. Its output is discarded, and host-requested cleanup cannot start before the stop receipt.

Existing production wiring is unchanged. `Controller::new` remains strict even when the review features are compiled. The alternative admission policy still requires its explicit constructor. The lifecycle requires an additional explicit non-default `offline-puresignal-tx-lifecycle` feature, which depends on `offline-puresignal-policy-proposal`. No production binary constructs it.

G2, installed firmware **1.31.002 / 0x53460004**, installed WDSP, driver, TX qualification, services, RF settings and polling were not touched. GoldMarsh's dirty worktree was inspected read-only and preserved. No commit, push or deployment was performed.

## Source-linked interfaces

Implementation: [tx_lifecycle.rs](../../saturn-bridge/src/puresignal/tx_lifecycle.rs). Shared owner changes: [controller.rs](../../saturn-bridge/src/puresignal/controller.rs). Features: [Cargo.toml](../../saturn-bridge/Cargo.toml).

| Contract | Definition and obligations |
| --- | --- |
| `TxAuthority` | Owned by the existing RF/TX authority, never the poller or a second WDSP owner. Progress methods are `release_received`, `begin_stop`, `poll_stop`, `begin_arm`, `poll_key`. They must not sleep, block on a DSP reset, or wait for a native worker. |
| `StopRequest` | Monotonic stop ID, backend, acquisition/config/TX fence, stop cause and controller notification time in monotonic ns. This is **not** hardware fault-onset time. |
| `StopReceipt` | Current ID plus affirmative local output-barrier closure, in-flight-write drain, queued-output discard and backend unkey-operation completion. All are required. Wrong ID is ignored, wrong backend evidence or incomplete completion blocks cleanup. |
| `StopEvidence` | `P2CommandBurstOnly` means local fencing plus command-burst completion, not UDP delivery/RF-off acknowledgment. `DirectControlReadback` requires future source-qualified control readback, not detector watts or an external RF measurement. The fixture simulates both labels; it does not supply hardware evidence. |
| `FreshTxAction` | New authority-authenticated operator action sequence, handled release ticket and current expected fence. Sequence/fence/ticket checks do not authenticate an operator themselves: authentication, physical interlocks, normal mic/IQ gates and the approved fresh-drive policy remain the existing authority's responsibility. |
| `KeyReceipt` | Matching fresh action and output generation, with non-future monotonic key time. A stale receipt cannot arm native correction. Key error/timeout requests a stop. |
| `ReleaseHandle` | Atomic, coalescing, saturating sequence mailbox. Posting release never waits for native DSP, fitting/reset, queue space or a native lock. The owner services it before each feedback/control quantum, not behind the feedback command queue. Saturation fails closed. |
| `CooperativeNative` | Single serialized WDSP owner extends `NativePs` with `advance_dsp_discard`, `begin_cleanup`, `cleanup_step`. Each cleanup step contains at most one DSP exchange, no sleep/reset loop/join/channel destruction. `Complete` promises asynchronous quiescence and IQC run/busy clear, not just an applied-flag change. |

Only the enclosing lifecycle can reach its controller's native owner. An attached controller's fault latch clears admission/captures **without calling blocking reset or native MOX-off**. Its configuration/key-generation retirement cannot proceed until the cooperative owner certifies cleanup. An attached `pump` consumes at most one queued block per call, hence at most one native window; a queued backlog cannot consume the entire queue before the release mailbox is serviced again.

Configuration is a bounded single pending slot. It fences output and waits for cleanup before applying native settings. Reconfiguration while PTT is held requires a subsequent release and fresh action; no config completion automatically keys. Fault acknowledgment clears only the acknowledged primary latch while safely off; it does not key. Shutdown is terminal for that owner.

## Native cleanup: real WDSP, not a fake reset

Actual offline port: [native_port.rs](../../saturn-bridge/tests/support/native_port.rs). Read-only internal IQC observer: [puresignal-native-observer.c](../../saturn-bridge/tests/support/puresignal-native-observer.c). Combined path: [puresignal-controller-native-replay.rs](../../saturn-bridge/tests/support/puresignal-controller-native-replay.rs).

The port first sets PS MOX false **after stop receipt**, then keeps TX DSP advancing while it drains the current fit. Stable source states WAIT/STAYON/RESET indicate completion of LCALC under this sole-owner fixture; `GetPSInfo` reports the pre-`pscc` state, so two stable observations are required. Only then does it request `SetPSControl(reset=1)`. It continues zero-input `pscc` plus one `fexchange0` per owner step, with output discarded, until two stable observations show state RESET, applied flag zero and internal IQC run/busy flags zero. The outer owner enforces an explicit cleanup deadline.

The runtime lifecycle does not invoke the port's old multi-second `NativePs::reset` loop. That loop remains solely for historical strict/proposal replay and final already-quiescent standalone-port destruction; the latter happens after output is inhibited and shutdown/cleanup is complete. No live integration should reuse the historical blocking port.

Posting PTT release is independent of native work. Handling is checked before each cooperative step, including while the native reset is pending. This eliminates waiting for the entire fitting/reset loop. It is **not a hard real-time bound** on one native C call, WDSP critical-section acquisition, scheduler dispatch or physical unkey. Those limits must be verified on the actual production owner/platform; no G2 latency was measured here.

### Failure handling

Primary hazard, TX-authority failure and native-cleanup failure have separate latches. A later report or cleanup failure does not erase the initiating primary fault. Authority/cleanup timeout prevents re-arm and does not claim successful bypass or RF-off. PTT release remains serviceable in the blocked state, and required DSP progress can continue with output discarded.

If the offline lifecycle is dropped before verified quiescence, it requests output inhibition again and deliberately retains its native owner instead of freeing memory that a native worker may still use. This fail-closed diagnostic retention is tested; it is **not** a complete production recovery/resource-lifetime policy. An authority failure means requested inhibition, not proven hardware-off. A production supervisor's recovery/shutdown policy remains to be wired and reviewed.

## Backend mapping: existing code is not yet the new receipt

| Backend | Existing source and required integration |
| --- | --- |
| Shared TX owner | [tx_thread.rs](../../saturn-bridge/src/tx_thread.rs), `do_unkey`, currently sets PS MOX false and deactivates native DSP **before** sending RF-unkey commands. This cannot be reused unchanged to prove the new cleanup sequence. Its `Unkeyed` event is emitted after the first packet attempt, including a failed send; it is not stop completion evidence. No production code was changed in this slice. |
| P2 | [p2/session.rs](../../saturn-bridge/src/p2/session.rs) contains the existing command building/transmit boundary; zero drive is already representable. The future authority adapter must fence all IQ writers/periodic command producers, discard queued output, finish in-flight writes, send unkey with an approved drive-disable policy and complete the burst before returning a receipt. Existing 12-packet / 5 ms spacing nominally adds about 55 ms of sleeps, **not** a delivery or RF-off guarantee. It must become yielding progress, not a blocking reset path. UDP has no physical RF-off readback here. |
| Direct XDMA | [xdma_tx_radio.rs](../../saturn-bridge/src/xdma_tx_radio.rs), `shutdown`, already attempts drive zero, TX amplitude/output/stream gate disable, MOX/TX GPIO clear, relay release and FIFO reset. It aggregates errors, but currently does **not** perform post-shutdown off-state readback. Its key-state readback helper is not an off-state receipt. The future adapter must first fence/drain the sole DMA producer and cancel queued writes, then complete existing shutdown controls and validate an off-state readback before using `DirectControlReadback`. This needs a host integration review, not a new FPGA or driver in this milestone. |

The local producer/barrier guarantee and the confidence in physical RF-off are different. P2 command-burst completion remains weaker than direct control readback; neither is an external RF power measurement. There is no basis here to advertise equivalent physical stop guarantees or to enable live PureSignal.

## Detection delay versus response after notification

These are deliberately separate timelines. Preserve oldest host arrival and producer/source validity; do not turn a decision fence into a hardware sample timestamp.

| Source | Detection/notification limitations — unchanged |
| --- | --- |
| P2 paired packets and high-priority ADC status | Format/sequence/arrival checks detect known delivered-path faults. Positive selected-ADC status is usable conservatively, but the status packet has no exact paired-sample/TX-epoch observation timestamp. Scheduling, UDP delivery/loss and queue delays are not bounded by this replay. Zero status does not certify a clear capture. |
| Direct paired acquisition | Existing sole-owner framing/FIFO/read/handoff notifications can stop after delivery. The frame lacks hardware sequence/time, so silent whole-frame loss remains unproven. No timing qualification of the frozen RXC1 feedback path was added. |
| Direct ADC telemetry | [xdma_backend.rs](../../saturn-bridge/src/xdma_backend.rs) retains 1 s performance and 5 s basic-status cadence. Read/snapshot/scheduling/queue delays are additional. These are not fast overload protection. |
| RXC1 snapshots | [rx_counter_v31.rs](../../saturn-bridge/src/rx_counter_v31.rs) retains its opt-in, supervised arm and internal **5 s** period. A positive coherent delta can supplement hazard handling; a periodic equal-count snapshot cannot certify a new capture unclipped. The poller was not enabled or accelerated. |
| Missing RF / stale feedback | The offline candidate examines complete 1,024-pair windows; 192 kHz corresponds to about 5.33 ms plus assembly/delivery/scheduling. Fixture age limit is 250 ms. The near-zero RF threshold is not qualified to detect noisy, mistuned or unrelated feedback. These values are **not production defaults**. |

After a valid notification reaches this owner, admission stops and `begin_stop` is requested synchronously in that handling operation; release gets precedence before the next native exchange. Native cleanup start is recorded only after a matching stop receipt. This response cannot recover time lost before notification or retrospectively certify earlier captures.

`LifecycleStatus` exposes phase, stop ID, primary/authority/cleanup fault, release tickets, required release, controller notification time, authority completion time, cleanup start/completion and step count. Timestamps use the owner's monotonic ns clock. `correction_applied` is nullable: unknown while stopping/cleaning/blocked, false after verified native cleanup under the accepted local stop receipt, and current native-controller attribution while active. This false value does not certify physical RF-off. `last_known_applied_before_stop` is historical, not a current health claim. Unclipped-capture qualification remains independently nullable with its precise reason.

The simulated authority in the replay waits for three 1 ms model control ticks before acknowledging a stop. The recorded delayed-overload example completed its stop receipt at **3 ms** and cleanup at **12 ms** after notification on both modeled backends. Those are **fixture-clock sequence results**, not measured P2/XDMA/RF latency or a production deadline. Native worker scheduling changes the number of exchanges between runs. The positive report in that example is deliberately delivered **300 ms** after its host observation; that delay is not included in the 3/12 ms response figures.

## Offline evidence

Runner: [run-puresignal-offline-native.sh](../../saturn-bridge/tests/run-puresignal-offline-native.sh). Unit cases: [tx_lifecycle/tests.rs](../../saturn-bridge/src/puresignal/tx_lifecycle/tests.rs). Simulated authority: [tx_authority_fixture.rs](../../saturn-bridge/tests/support/tx_authority_fixture.rs). Packaged full transcript: `offline-native/tx-lifecycle-replay-result.txt`.

| Check | Result |
| --- | --- |
| Default strict standalone tests | **24 passed**; strict policy unchanged. |
| Explicit admission proposal | **32 passed**, including the previous contract regression cases. |
| Explicit lifecycle feature | **42 passed**: the prior 32 plus 10 lifecycle tests. |
| Cargo stub-native full suite, lifecycle feature | **42 library + 377 binary passed, 2 existing ignored**. Stub tests are not native replay. |
| Cargo stub-native full suite, default features | **24 library + 377 binary passed, 2 existing ignored**. |
| Library Clippy, lifecycle feature | Passed with `-D warnings`. Whole-test Clippy was also attempted and reports test-style lints in the earlier offline `puresignal/tests.rs`; no whole-repository Clippy pass is claimed. |
| Browser typecheck and regression tests | Passed; **699 tests / 82 files**, unchanged UI/panels. |
| ShellCheck runner / diff whitespace | Passed. |
| Pinned header/source ABI verifier | Passed; incompatible old `GetPSDisp` declaration still rejected. |
| Real native combined pipeline | Both P2 and Direct XDMA passed strict, admission-proposal and new lifecycle replay. Both new lifecycle runs interrupt fitting, handle release in native RESET, keep DSP advancing, exercise delayed overload, missing RF and reconfiguration, preserve latches, require fresh authorization, and finish with IQC flags **0**, channel closed. **Zero blocking resets during lifecycle runtime**, checked before final already-quiescent destructor. |
| Standalone numerical fitting | Passed; 4,096 measured / 512 correction capacities and canaries retained. Modeled RMS approximately 0.102006762 to 0.00147866877. This is not measured RF IMD/ACLR. |

The unit tests additionally withhold/forge/stale a stop receipt, stall authority/native cleanup, fault the key operation, regress the clock, submit a feedback backlog, preserve a primary fault across a delayed secondary hazard, ensure shutdown cannot re-arm, and demonstrate `correction_applied=true` with `ClipStatusUnknown` measurement qualification. Their native fake panics if any blocking reset is attempted.

The actual native fixture uses WDSP version **210 / PS3**, upstream `b02d5bac675dd2f33ec2bab2b339f79a597c47dd`, Linux-port `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`, unchanged x86-64 archive SHA-256:

```text
8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248
```

Full built C/header sources and GPL license accompany the archive. The runner checks public headers, implementation and internal observer inputs before linking. This is **not** an identity check of the library installed on G2. Never install this x86-64 replay archive on G2.

## Remaining bounded work

1. Review this interface/order and choose backend stop-evidence/recovery obligations before wiring the existing TX authority. Do not reuse early `Unkeyed` events or native deactivation as stop receipts.
2. Qualify feedback suitability/limits with path-specific noisy, mistuned and unrelated RF, normal speech pauses and two-tone envelope-minimum cases. Preserve absence of production defaults until reviewed.
3. Establish installed native identity, real feedback topology and owner integration separately. Direct XDMA still needs its actual feedback acquisition configuration, not controls alone.
4. Continue the offline analyzer and read-only PS panels without implying live correction, fast overload protection, external RF measurement or unclipped qualification that is unavailable.

No new FPGA, kernel-driver change or faster polling is requested. The intended result remains correction on both backends, trustworthy measurements and three dockable/pop-out panels; this slice proves lifecycle sequencing in the combined offline pipeline, not that complete live result.
