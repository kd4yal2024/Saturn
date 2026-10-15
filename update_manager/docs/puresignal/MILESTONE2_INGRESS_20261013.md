# PureSignal milestone 2 — actual ingress / exclusive host-owner review

Date: 2026-10-13. Isolated SilverForge base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome and scope

Connected the actual TCI WebSocket handler and authenticated SATP UDP receiver
to the shared lifecycle's bounded queues. A real-WDSP replay selects that host
path on both P2 and ordinary-file direct-XDMA fixtures, establishes correction
across three generations, then faults an authenticated sample-counter jump and
completes stop receipt/native cleanup. No commit/push, deployment, G2, RF,
firmware, FPGA, driver or polling change. GoldMarsh is untouched; firmware
1.31.002 / 0x53460004 remains unchanged.

**Limited offline review, not complete production TCI migration or hardware-test
authorization.** Managed operator-socket PCM and SATP are exercised. Managed
Opus and split-media TCI remain inhibited pending their history/generation
contracts. Default legacy transports and normal TX remain unchanged.
Installed startup still selects Legacy. SharedReview is an explicit internal
review selection: no environment flag, URL switch, hot switch or fallback enables
it on G2. The pre-client frontend attachment/shared-owner factory are available
for reviewed host integration, not called by installed startup.

## Source-linked path

```text
TCI operator WebSocket complete-message observation
 -> capture age/generation before role parser, PCM decoder or queue
 -> sealed arm / bounded configuration transaction / stamped audio
 -> shared OwnerLoop -> one full native TX engine -> one output adapter

SATP UDP receive
 -> capture age/generation before HMAC, timeline and jitter work
 -> existing HMAC/lease/replay checks -> jitter ring retains observation
 -> original-age playout -> bounded audio queue -> FIR/ring provenance
 -> same OwnerLoop/native engine

release / disconnect / detected discontinuity
 -> atomic output fence closes independently of queue capacity
 -> periodic lifecycle -> matching stop receipt -> native cleanup
 -> fresh authorization required; fault remains latched when applicable
```

[TCI socket routing](../../saturn-bridge/src/tci/client.rs) captures before the
existing parser/decoder. [SharedIngress](../../saturn-bridge/src/tx_thread/ps_ingress.rs)
consumes TX controls/audio rather than teeing them into the legacy mailbox.
RX/client-only requests retain their ordinary mailbox. Up to 64 TX settings in
one text message form one model/PS/source transaction. A release in that message
cancels the batch. Neither config nor arm is restamped at dequeue.

[SATP](../../saturn-bridge/src/satp.rs) puts the observation in each packet before
jitter insertion. [TxAudioIngress](../../saturn-bridge/src/tx_audio.rs) offers an
exclusive shared producer route with no legacy receiver. Managed delivery
refuses packets without original arrival evidence and fabricated gap audio.
Counter discontinuity, ring overrun, resynchronization, session/lease changes and
unresolved playout gaps retire an active generation. Source faults carry a
host-observation time and reason in the trace.

The normal pre-arm jitter clear is not an operator release: native/source cleanup
already preceded that new authorization. It discards old jitter without canceling
the new generation. Old buffered observations retain their binding and cannot be
relabeled. SATP FIR/ring/packet-tail age tracking from the accepted slice remains.

## Exclusive ownership and authority

[Selection](../../saturn-bridge/src/tx_thread/owner_selection.rs) invokes exactly
one startup branch; SharedReview failure does not fall back to Legacy.
[OwnerLoop::with_shared_ingress](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs)
constructs the bounded channels and one owner together. Frontend attachment
refuses existing clients or an already-installed route.

Real legacy and shared engines also use one
[process-local channel-1 lease](../../saturn-bridge/src/wdsp/native_owner.rs) in
review-feature builds. A second constructor is refused before OpenChannel.
Recreation transfers the same lease without a release/reacquire gap. Retained
unproven native lifetime also retains that lease. The real replay attempts a
legacy constructor during shared ownership and requires ExternalController.
Stubs have no native channel, so independent stub fixtures remain parallel.
This does not replace existing cross-process XDMA/radio ownership.

[Credentials](../../saturn-bridge/src/tci/ps_authorization.rs) now carry an atomic
session-liveness token revoked on unregister, together with current operator
identity. Registry contention is not treated as disconnect. Socket disconnect
closes the shared fence before unregister and does not enqueue a legacy boolean
TX release. Source-active publication is atomic; snapshot-lock availability is
not a source transition. Snapshot locking contains no native/device work and
cancellation does not acquire it. TX commits preserve unrelated RX state.

## Startup, status and explicit limits

The [SATP adapter](../../saturn-bridge/src/tx_thread/satp_provenance.rs) waits for
512 admitted real input samples before starting the destination sample clock.
Primed silence/incomplete tails do not meet readiness. While waiting, the single
native engine advances with discarded zeros, source/key deadlines continue and
release stays first. One-exchange-per-turn remains intact. Underflow after
startup still faults; it is not hidden by fresh packets or native slew.

| Path / behavior | Evidence or gate |
| --- | --- |
| Operator-socket TCI PCM | Actual framing/parser/decoder reaches shared owner on both backend fixtures |
| Configuration/release/fresh arm | Batch, cancellation and pre-queue authority exercised |
| Authenticated SATP | Actual UDP/HMAC/lease/replay/jitter reaches real matcher/full engine on both fixtures |
| Feedback | Existing P2/direct decoders feed synthetic wire fixtures; actual acquisition-owner hookup/identity remains unqualified |
| Legacy/shared native coexistence | Second engine refused before channel creation |
| Managed Opus | Advertises PCM only and rejects Opus before decoding; retained-state age/generation contract unqualified |
| Managed split media | Rejected; existing separate TCP lanes lack an arm/media generation barrier |
| Shared pipeline measurements | Legacy counters are not shared measurements: explicit shared-* state and tx_pipeline_metrics_valid=false |
| Physical RF-off / G2 latency | Not proven by loopback completion/file readback or exchange-count accounting |

TCI native replay uses intermittent 512-sample PCM messages at 20 ms, including
pauses/zero fill; it is **not sustained TCI throughput evidence**. SATP's independent
fixture sender uses accumulated 48 kHz deadlines/four 128-sample packets per burst,
independent of DSP pauses. The WebSocket fixture prequeues its complete handshake;
production handshake/reconnect timing is not qualified. The existing offline FFTW
planning limit is applied explicitly, avoiding dependence on test order.

Timestamps establish complete-message/UDP bridge-ingress age, not sender capture,
network transit or exact RF-output age. Group delays are not promoted to measured
RF timestamps. Native recreation/backend stop operations still lack measured G2
wall-clock bounds. Slow telemetry is not fast overload protection or proof of an
unclipped capture. Installed native identity, real RXC1 feedback topology, live
level thresholds, status integration and physical RF qualification remain open.

## Executed checks

| Check | Result |
| --- | --- |
| Owner Cargo, native stubs | 46 library + 418 binary passed; 2 existing ignored |
| Default Cargo, native stubs | 24 library + 379 binary passed; 2 existing ignored |
| Real WDSP owner runner | 5 serial combined tests passed, including actual ingress on both backend fixtures |
| Standalone strict/proposal/lifecycle | 24 / 32 / 46 and real/numerical replays passed |
| Formatting, whitespace, runner ShellCheck | Passed |

New deterministic cases cover full-queue release canceling old arms, delayed PCM
retaining original age, viewer refusal, batched settings, release canceling a
batch/old arm, preservation of RX state, Opus/split refusal, registry contention
versus revocation, source priming and native-owner exclusivity. Actual native replay
requires real rendering/output, three corrected generations, the injected fault
latch, empty owner queues, zero IQC flags and completed cleanup. It asserts at
most one native exchange per turn throughout active operation and cleanup.

Current transcripts: offline-native/ingress-*. Older reports/transcripts are
historical. These are SilverForge's execution results, not the external reviewer's
independent execution. Reproduce from extracted root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
bash update_manager/saturn-bridge/tests/run-puresignal-owner-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
```

Requires Rust/cached dependencies, compiler/binutils, FFTW development files.
Runner validates 22 pinned native inputs. Bundled x86-64 WDSP archive SHA-256:
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
**Never install that offline archive on G2.**

Review this limited ingress/exclusive-owner slice. A normal managed remote TCI
trial still needs a split-media generation barrier and an Opus history contract,
or an explicit reduced operator-socket PCM/SATP trial choice. No FPGA, driver or
polling change is indicated by these host tests.
