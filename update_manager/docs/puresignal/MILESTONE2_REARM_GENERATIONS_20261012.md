# PureSignal milestone 2 — authorized re-arm and output generations

Date: 2026-10-12. SilverForge isolated base: `861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome

Implemented and exercised the next **offline** handoff: one shared lifecycle/controller, real pinned WDSP, and the existing P2/direct-XDMA output adapters now run together. Release, fault, disconnect and reconfiguration cancel pending arms and queued mic/IQ. Native cleanup completes before a new role-authorized action opens a new output generation. Old capabilities, commands, samples and stop receipts are rejected across that boundary.

This supersedes the previous watchdog/owner report's stop-only limitation **for explicitly enabled offline adapters**. Default adapters remain stop-only. Production socket handlers, command queues and TX scheduling have not been switched to this handoff. No live PureSignal approval, physical RF-off measurement or G2 timing bound is claimed.

G2, firmware **1.31.002 / 0x53460004**, FPGA, driver, polling, services and deployments are unchanged. No commit/push/merge occurred. GoldMarsh's original worktree was not edited.

## Source-linked boundary

```text
TCI operator ingress: validate current connected operator; parse NEW TX request
     |
     v  sealed, single-use capability stamped BEFORE queuing
        [action sequence, release ticket, config fence, invalidation revision]
shared lifecycle: complete backend stop receipt -> cooperative WDSP cleanup
     |
     v  fresh authorized action only
offline owner: open new output generation -> normal mic / IQ qualification
     |
     v  generation-stamped native output, bounded queue with retained tails
P2 packet builders/socket OR direct stream/DUC packing/register key/readback
     |
release/fault/disconnect/config change: close immediately, erase capability/queues
     v  drain permitted writes -> complete stop -> WDSP cleanup (DSP advancing)
```

- [Operator credential issuance](../../saturn-bridge/src/tci/ps_authorization.rs) uses the existing connected-operator registry and role-aware TCI parser. Viewers, disconnected operators, release commands and arbitrary `FreshTxAction` structs cannot supply an arm capability. Capabilities are not cloneable; the owner consumes them once and rechecks the role before key/output. Role checks on the periodic path use a nonblocking lock attempt and fail closed on contention.
- **Authorization is precisely scoped:** this uses existing TCI operator-role authorization. It is not a new cryptographic login, SATP/HMAC implementation or proof that an installed browser connection is authenticated. The test-only registry establishes the operator fixture without a live socket. Future production socket integration must issue credentials at actual ingress, before command queuing; stamping a legacy queued boolean when delivered would be incorrect.
- [OutputFence / owner stop boundary](../../saturn-bridge/src/tx_owner_stop.rs) tracks monotonically advancing invalidation revisions and output generations. After explicit handoff activation, all untagged legacy writers remain denied permanently, including after re-arm. Closing never waits for the writer mutex. Reopening requires a closed, drained fence, matching revision and newer generation. Already permitted writes may finish; receipts wait for them, rather than falsely calling them canceled.
- [Re-arm and queue handoff](../../saturn-bridge/src/tx_owner_stop/rearm.rs) requires the matching completed stop receipt **and** lifecycle notification of native cleanup/config retirement. That notification does not authorize TX. Fault acknowledgment invalidates pre-acknowledgment grants and never keys. A new action must pass the lifecycle's release/fault/fence checks and the owner's sealed-capability checks.
- Mic and IQ use the existing owner's `VecDeque<f32>` queue types, bounded here to one mic block and one native output plus a partial packet tail. Native 4,096-float output is not truncated to a packet multiple: tails remain ordered. Timestamp spans preserve the oldest remaining observation, so a fresh append cannot hide stale queued IQ. Cancellation discards those tails too. Mic qualification is recorded at input dequeue; native exchange errors invoke the stop path.
- [Lifecycle integration](../../saturn-bridge/src/puresignal/tx_lifecycle.rs) checks release before and after an offline native-output exchange. Native cleanup still waits for a complete matching backend stop receipt, advances at most one DSP exchange per lifecycle step, and does not delay release behind a blocking reset. No early legacy `Unkeyed` event substitutes for completion.

## Backend guarantees remain different

| Backend | Offline path exercised | Evidence / limitation |
| --- | --- | --- |
| P2 | Existing HP and DUC IQ builders, sequence counter and UDP socket; distinct loopback control/IQ destinations. Independent HP sender retains a stale TX-enabled model during re-arm but cannot emit untagged output. | Matching complete 12-command zero-drive/unkey burst, actual 5-ms stop spacing in the combined replay. **Local UDP command completion only**, not acknowledgment, RF-off or delivery proof. Packets already handed to UDP cannot be recalled. |
| Direct XDMA | Actual stream configuration, mux/FIFO reset, zero prefill packing, first live frame, `key_with_frame`, keyed control verification, steady IQ packing/writes and shutdown/readback, using ordinary register/DMA files. | **File-backed control readback exercise**, not hardware DMA/FIFO timing or physical RF-off. Key progress is split into bounded stages with try-locks; it does not call the existing blocking FIFO-prefill/pacing loop. Fixture occupancy is explicitly synthetic. |

The offline key gate reuses [the existing `can_key_rf` and `KeyQualification`](../../saturn-bridge/src/tx_thread.rs), backend packet requirements and microphone windows. Silent microphone input cannot qualify nonzero IQ. The fixed offline model must be running, TX-enabled, non-WFM and within an explicitly supplied drive cap. No new production drive defaults or feedback thresholds were established. Direct's test path does not silently bypass the microphone requirement with two-tone mode.

## Combined execution evidence

[Combined native/actual-owner test](../../saturn-bridge/src/tx_owner_stop/combined_native.rs), run serially on both backends:

1. Complete actual owner stop and real WDSP cleanup; deny a viewer action.
2. Arm, queue real native output, release before key; erase the queued mic/IQ and capability.
3. Reject the prior queued ingress credential, action, mic/IQ stamp and stop receipt; require a newly issued action.
4. Qualify native microphone-derived IQ and key through the actual adapter. Keep untagged legacy output inhibited.
5. Decode synthetic P2/XDMA feedback through the shared controller; real WDSP establishes correction. Deliver the output of that **same** native processing exchange through the actual owner, retaining packet tails. Missing overload brackets leave unclipped qualification unavailable, not fabricated clear.
6. Fault with correction applied and output queued; stop-before-cleanup and retain the fault latch. Release, finish native cleanup and explicitly acknowledge.
7. Cancel another pending arm/output on reconfiguration. Require another fresh action, then key again.
8. Disconnect during Active TX with native output queued; periodic `tick()` detects revoked operator identity without another packet or release notification, closes/discards output and latches a fault. Release then completes stop and native cleanup. No automatic re-arm.
9. Verify zero runtime blocking resets, final IQC flags zero and channel closure. The isolated final destructor does one already-quiescent defensive reset; it is not counted as runtime cleanup.

The P2 capture asserts exactly two owner key commands while the independent stale-model HP sender runs, plus completed zero-drive commands and nonzero native IQ packets. Direct exercises two actual file-backed key/readback sequences. Neither is measured RF.

| Verification | Result |
| --- | --- |
| Default Cargo, native stubbed | 24 library + 377 binary passed; 2 existing ignored. |
| Owner-feature Cargo, native stubbed | 45 library + 387 binary passed; 2 existing ignored. Ten actual-owner regression cases. |
| Combined real-native/actual-owner replay | One serial test passed, exercising both P2 and direct-XDMA, including generation cancellation, correction, reconfiguration and disconnect. No native stubs. |
| Standalone strict/proposal/lifecycle | 24 / 32 / 45 passed; existing real-WDSP fitting, lifecycle and periodic-watchdog replays passed. |
| Library Clippy, formatting, ShellCheck, whitespace | Passed. No full-repository Clippy claim. |

Native identity remains WDSP 210 / PS3: upstream `b02d5bac675dd2f33ec2bab2b339f79a597c47dd`, Linux port `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`; archive SHA-256 `8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`. This is **x86-64 offline evidence, not the installed G2 library**; never install this archive on G2. Synthetic fit/RMS improvement is not externally measured IMD.

## Reproduction and remaining scope

From extracted review-package root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-owner-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
```

Cargo runners require cached dependencies (`--offline`); native replay also requires Rust, a C compiler/binutils and FFTW development files. Complete bridge source/build inputs and the pinned native archive/source/license are included. Transcripts named `offline-native/rearm-*` are current; older reports/transcripts are historical.

The bounded offline connection is now exercised together, rather than inferred from separate native and adapter tests. **It does not run the installed production `tx_thread` scheduler or upgrade its existing untagged command queues.** Production wiring must carry the reviewed envelopes from true operator ingress through all producers, use the single native owner, preserve normal source/startup/drive protections and schedule lifecycle service. Default constructors still attach no fence; the new features have no live callers. The offline adapter's fixed model and synthetic FIFO are not substitutes for that wiring.

Installed-library identity, exact RXC1 feedback topology/continuity, suitable path-specific feedback thresholds, G2 scheduling/syscall bounds, physical RF-off and live correction remain unqualified. Slow telemetry remains supplementary rather than fast protection. Analyzer and read-only PS-panel work may continue offline; no new FPGA, driver or polling change is indicated.
