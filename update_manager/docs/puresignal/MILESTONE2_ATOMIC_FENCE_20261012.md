# PureSignal milestone 2 — atomic cancellation fence and owner command handoff

Date: 2026-10-12. Isolated SilverForge base: `861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome and scope

The reported source-level race was valid. Closed state and invalidation revision
were published separately, so a paused close could be overwritten by reopen.
That implementation and its concurrent-close comment are superseded by the
[atomic fence](../../saturn-bridge/src/tx_owner_stop.rs). This is a concurrency
correction, not evidence of a radio incident.

Continued the next bounded **offline** production-owner integration step: a
sealed authorized-arm envelope now travels through the existing `TxCommand`
type/channel and its shared-lifecycle consumer in the combined real-WDSP replay.
The installed TX scheduler, audio/rmatch/source-selection paths and live socket
handlers have **not** been switched. This is not a completed production migration.

No G2, RF test, firmware (1.31.002 / 0x53460004), FPGA, kernel driver, polling,
services or deployment change. No commit/push. GoldMarsh's worktree is untouched.

## Atomic boundary and ordering argument

`OutputFence::state` is one `AtomicU64`: the high bit is closed, the remaining
63 bits are the invalidation revision. `close()` atomically sets closed **and**
advances the revision in one `fetch_update` transition. It never takes the DMA
writer mutex. Revision exhaustion saturates and permanently refuses reopening;
it never wraps or reuses a prior revision.

Conditional reopen holds the writer guard, stores the newer output generation,
then compares/exchanges that same state word from the expected **closed revision**
to its open form. The generation store alone never publishes open output.

| Ordering at the state word | Result |
| --- | --- |
| Close linearizes before reopen CAS | Revision differs; CAS fails and cannot open. |
| Reopen CAS linearizes before close | Close publishes a closed, incremented revision. It cannot be overwritten by a later reopen store because there is no such store. |
| A permitted writer is still holding its guard | Close completes without waiting. Reopen/stop receipt waits for drain; that write remains explicitly in flight. |

`enter_generation()` snapshots and rechecks the complete open state, including
revision, plus the requested output generation around writer acquisition. Its
successful final state check is the permit linearization point. A cancellation
after that point does not retroactively cancel the already-permitted operation;
the retained guard prevents a complete stop receipt or reopen until it drains.

A genuinely fresh, authorized generation can reopen after cancellation, complete
backend stop and native cleanup. That is intentional new authority, not reuse of
the canceled revision. Legacy untagged writers remain permanently inhibited in
managed mode, including after such a fresh reopen.

## Deterministic regression evidence

[Owner tests](../../saturn-bridge/src/tx_owner_stop/tests.rs) include five new cases:

1. Pause close immediately after its atomic state transition, before it returns;
   attempt stale reopen and output entry, then resume close. Both closed state and
   incremented revision are already visible; the fence remains closed and entry
   stays denied. The former flag/increment pause point no longer exists.
2. Pause the competing reopen between generation setup and its state CAS; execute
   close, then continue reopen. CAS refuses the canceled revision.
3. Keep a valid writer permit held on one thread while another completes close.
   New permits are denied; drain and reopen remain pending until the old permit
   is released. Test coordination uses channels, not scheduling sleeps.
4. Exercise revision exhaustion and verify permanent fail-closed behavior.
5. Repeat the paused-close race through the actual direct owner with a sealed
   operator credential. Reject that old arm; complete a new stop and native-cleanup
   notification, issue a new credential, and successfully open generation 2.
   Opening a generation alone does not key RF; the direct fixture stays off.

The original normal-key positive controls and full combined replay remain in
place. The test hooks are empty closures in ordinary calls; they expose the
actual state transitions without implementing a separate test algorithm.

## Continued existing-owner integration, still offline

[Existing TX command enum](../../saturn-bridge/src/tx_thread.rs) now has a
non-default `PureSignalAuthorizedArm(AuthorizedArm)` variant. Its credential was
issued at operator ingress before queuing, remains sealed/non-cloneable, and
cannot have its old revision refreshed at dequeue. A legacy `Arm { rf_enabled }`
cannot be converted into it.

[Shared-lifecycle consumer](../../saturn-bridge/src/tx_thread/ps_owner_queue.rs)
services one periodic lifecycle step before at most one command. Thus release,
revocation and feedback deadlines take precedence and still run on an empty
queue. A disconnected command channel closes output, posts release and notifies
a fault without a second native exchange in that same turn. Further native
cleanup progresses on subsequent turns. Native cleanup completion alone remains
insufficient to authorize TX.

The legacy TX run loop refuses this new envelope and stops rather than treating
it as a legacy boolean arm or constructing a second native owner. No live
producer emits it. In the combined test, all fresh authorized arms now traverse
the real `TxCommand` channel; canceled queued credentials are tested both during
release and after completed cleanup. Legacy boolean arm commands are refused.
Closing the command channel while real native cleanup advances also verifies
the one-exchange-per-turn bound. Both backends establish correction through
the same lifecycle, native port and actual owner adapters used in the previous
slice.

This migrates the reviewed **arm envelope/consumer boundary**, not the full
production event loop. Mic/rmatch buffering, SATP/TCI source policy, model-change
envelopes, source/watchdog service, real acquisition feedback and installed native
owner integration remain required before live selection. Actual socket handlers
must issue credentials at ingress; relabeling old queued booleans is forbidden.

## Execution evidence and reproduction

| Check | Current result |
| --- | --- |
| Focused actual-owner cases | 15 passed, including all five new fence cases. |
| Owner-feature Cargo (native stubbed) | 45 library + 392 binary passed; 2 existing ignored. |
| Default Cargo (native stubbed) | 24 library + 377 binary passed; 2 existing ignored. |
| Combined real WDSP + actual P2/direct owners | One serial combined test passed on both backends, including the existing-command envelope/consumer. |
| Standalone native strict/proposal/lifecycle | 24 / 32 / 45 passed; real-WDSP fitting, lifecycle and watchdog replay passed. |
| Formatting, whitespace and ShellCheck runners | Passed. |

Current transcripts in the new archive are `offline-native/fence-*`; earlier
reports/transcripts are historical. From its extracted root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
bash update_manager/saturn-bridge/tests/run-puresignal-owner-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
```

Requires cached Cargo dependencies, Rust, compiler/binutils and FFTW development
files. Bundled WDSP remains pinned x86-64 offline material, archive SHA-256
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
**Never install that archive on G2.** Numerical replay is not measured RF IMD.

P2 evidence remains local UDP command completion; direct evidence remains
ordinary-file control readback with synthetic FIFO occupancy. Neither proves
physical RF-off, G2 latency or live correction. Installed-library identity,
RXC1 feedback topology and path-specific thresholds are still qualification work.
No additional FPGA, driver or polling assignment follows from this correction.
