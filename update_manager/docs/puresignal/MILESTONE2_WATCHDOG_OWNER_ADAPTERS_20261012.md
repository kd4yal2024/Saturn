# PureSignal milestone 2 — periodic watchdog and offline TX-owner stop adapters

Date: 2026-10-12. Isolated SilverForge base: `861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome and limits

Closed the packet-independent watchdog gap. Added stop-only adapters against the existing P2 output methods and direct-XDMA writer/shutdown methods, with offline tests of local output fencing, pending-output cancellation and complete receipt generation. **Production wiring remains unchanged. No live PureSignal qualification is claimed.**

The earlier TX-lifecycle report is historical. This report supersedes its missing periodic Active check and begins its owner-adapter assignment. G2, installed firmware **1.31.002 / 0x53460004**, FPGA, kernel driver, polling, services and RF were untouched. No commit, push or deployment occurred. GoldMarsh's original dirty worktree was inspected read-only and preserved.

## Watchdog correction

[Controller::check_deadline](../../saturn-bridge/src/puresignal/controller.rs) supplies the same deadline checks to `pump()` and the Active branch of [TxLifecycle::tick](../../saturn-bridge/src/puresignal/tx_lifecycle.rs). It does not consume feedback, process native data or perform cleanup. An expired deadline requests the normal stop sequence and latches `Stale` even if no further packet arrives.

The no-first-packet deadline starts at key time plus settling. Subsequent freshness uses the last accepted observation time, not processing time. The oldest partial and oldest queued observation are checked too: recent arrivals cannot hide an expired backlog. Equality at the freshness limit remains admissible; exceeding it faults.

Release is serviced before the Active watchdog. Tests explicitly release on an expired tick and confirm release handling wins without inventing a stale-feedback fault. Watchdog ticks perform zero DSP exchanges; existing cleanup steps retain their one-exchange maximum. The native curve is retained until the matching complete stop receipt.

Regression cases in [tx_lifecycle/tests.rs](../../saturn-bridge/src/puresignal/tx_lifecycle/tests.rs), each exercised on both backends:

- Successful key, no first feedback packet, only periodic ticks: stop and latched fault.
- Established correction, feedback ends, only periodic ticks: stop, receipt, cleanup, still-latched fault.
- PTT release on an expired tick: priority release handling, no watchdog misclassification.

The [real-WDSP combined replay](../../saturn-bridge/tests/support/puresignal-controller-native-replay.rs) repeats both missing-stream cases, including actual established native correction. It retains zero blocking resets during lifecycle runtime, final IQC run/busy flags zero, and channel closure. Its authority remains simulated; this is separate evidence from the owner tests below.

## Existing output boundaries, now exercised offline

New non-default feature: `offline-puresignal-owner-adapters`, depending on the lifecycle feature. [OwnerStopAdapter and OutputFence](../../saturn-bridge/src/tx_owner_stop.rs) have **no production constructor/caller**. Only test-only loopback/file constructors attach a fence. Normal P2 binding and direct-XDMA opening still set no fence, including when the feature is compiled. `Controller::new` still selects the strict policy.

```text
P2 foreground packets ----+
P2 independent HP sender -+--> shared final-output permit --> local UDP send
Direct owner operations --+--> shared final-output permit --> state lock / DMA
                              |
stop/release: close atomically + discard owner queues
                              |
periodic progress: await permits drained, complete backend stop, issue receipt
                              |
                     lifecycle native cleanup may begin
```

The fence closes without acquiring the writer/state mutex. Ordinary producers check it before and after acquiring the final output permit. A write already holding a permit may finish; it is **in flight**, not falsely described as canceled. Stop completion waits for that permit. The closed fence cannot reopen in this review cut. Poisoned locks fail closed.

The adapter exclusively borrows the same `VecDeque<f32>` pending-IQ and pending-mic queue types used by [the existing TX owner](../../saturn-bridge/src/tx_thread.rs). Stop/release clears those queues, and they cannot be refilled during this exclusive offline handoff. This is not yet an integration of the running `tx_thread` command/WDSP owner: the future production handoff must close admission for commands, mic, synthesized IQ and every producer before reopening a fresh authorized generation. A borrowed queue does not prove that unintegrated production handoff.

| Backend | Tested local completion | What it does not prove |
| --- | --- | --- |
| P2 | [P2Session](../../saturn-bridge/src/p2/session.rs) sends actual existing-format packets to loopback UDP. Both foreground sends and the independent high-priority thread share the final fence. Stop progress sends one MOX-off, drive-zero, TX-relay-clear packet per due step; **all 12 sends** must complete. Existing 5 ms spacing is implemented as yielding progress, not sleeps within the adapter. | UDP delivery to a real radio, radio acceptance or physical RF-off. Packets already sent before the barrier cannot be recalled. Evidence remains `P2CommandBurstOnly`. |
| Direct XDMA | [DirectXdmaTxRadio](../../saturn-bridge/src/xdma_tx_radio.rs) uses its actual state mutex, sample packing, `FileExt` DMA write and existing shutdown. Stop uses `try_lock`, then reads back DAC zero-drive word, TX amplitude/output/stream/mux disable, MOX/TX GPIO clear and relay inhibit, ALEX relay clear and FIFO-reset release. Any I/O/readback failure prevents a receipt. | FPGA/device behavior, actual DMA completion timing, detector accuracy or external RF power. Tests use ordinary files, not `/dev/xdma*`; evidence is a **file-backed exercise** of `DirectControlReadback`, not measured G2 readback. |

The [direct fixture](../../saturn-bridge/src/xdma_tx_radio/offline_fixture.rs) never calls hardware `open`, environment device discovery, the codec worker or memory locking. The monitor's test-only disabled constructor prevents unintended codec access. A paused actual file write holds the real output/state permits: stop returns pending, new key/stage/write requests are rejected, then the permitted write completes and shutdown/readback produces the receipt. Output is not flushed from canceled queues.

The six [owner regression tests](../../saturn-bridge/src/tx_owner_stop/tests.rs) cover the full P2 burst and packet fields, independent HP sender, actual direct in-flight write/drain/readback, direct register I/O failure, malformed backend stop, and actual UDP send failure. Errors latch the adapter failed and never mint a receipt.

**No early `Unkeyed` event is consumed.** Existing production `do_unkey` is unchanged and still cannot substitute for this receipt: it deactivates native processing before RF commands and emits the event after the first packet attempt. This cut does not silently route production unkey through that legacy path.

## Evidence and reproduction

| Check | Result |
| --- | --- |
| Standalone strict / proposal / lifecycle suites | **24 / 32 / 45 passed**. |
| Full Cargo stub-native owner-feature suite | **45 library + 383 binary passed, 2 existing ignored**; includes six new owner cases. |
| Default Cargo stub-native suite | **24 library + 377 binary passed, 2 existing ignored**. |
| Real pinned WDSP replay | Both feedback backends passed strict, proposal, lifecycle and periodic-watchdog cases; native cleanup advances DSP, fault latches persist, runtime blocking resets zero, final IQC flags zero, channel closed. |
| Standalone numerical fitting | Passed; 4,096 measured / 512 correction capacities and canaries; modeled RMS 0.102006762 to 0.00147866877. Not measured RF IMD. |
| Library Clippy / formatting / ShellCheck / whitespace | Passed. No full-repository Clippy claim. |

Native identity unchanged: WDSP 210 / PS3, upstream `b02d5bac675dd2f33ec2bab2b339f79a597c47dd`, Linux port `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`; archive SHA-256 `8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`. This is the bundled **x86-64 offline** library, not the installed G2 library. Never install it on G2.

From the extracted package root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
```

Native replay needs rustc, C compiler/binutils and FFTW development files. Owner tests additionally require Cargo and cached dependencies (explicit `--offline`); the package includes the complete bridge source/build script/manifest/lockfile, not a full Saturn checkout. FFI is stubbed **only** for Cargo owner tests. Transcripts are in `offline-native/watchdog-owner-native-result.txt`, `owner-cargo-result.txt` and `default-cargo-result.txt`. Prior transcripts remain historical evidence.

## Remaining qualification and next boundary

The stop-only adapter implements the stop side of `TxAuthority` but **refuses `begin_arm` and `poll_key`**. Operator authentication, fresh-drive approval, normal mic/IQ qualification, pending-arm/command cancellation and production queue-generation handoff must be integrated with the existing owner offline before any live wiring. Real WDSP with the simulated lifecycle and real output-owner methods with stubbed native are separate tests; this package does not claim their complete production combination.

Freshness enforcement is now packet-independent, but it still relies on the lifecycle service actually being scheduled. There is no measured G2 scheduling, syscall, native-call or RF-off bound. Backend/source notification delay remains separate from controller response after notification. Slow status/RXC1 polling is unchanged and is not fast overload protection.

Installed native identity, exact RXC1 feedback configuration/continuity, path-specific feedback-level suitability, physical RF-off and live timing remain open. Firmware 1.31.002 stays unchanged. No FPGA, driver or faster polling work is requested by these findings.
