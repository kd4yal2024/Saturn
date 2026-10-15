# PureSignal runtime orchestration and ARM candidate review

The actual shared backend runtime now passes offline startup, reception,
release/re-arm, acquisition-error and shutdown tests. This supersedes the
orchestration gap in the [previous wiring review](PRODUCTION_WIRING_ARM_REVIEW_20261013.md).
The candidate remains default-off and requires review before any G2 execution.

## Runtime and shutdown changes

[shared_backend::run_inner](../../saturn-bridge/src/shared_backend.rs) has one
orchestration loop for production and tests. Its external I/O boundary supplies
hardware endpoints in production, and loopback UDP/ordinary register files in
tests. It does not substitute a test lifecycle or fixture scheduling loop.
The independent test client sends actual TCI commands and tagged PCM. It never
pumps the owner, reads acquisition, changes layouts or issues completion receipts.
The Direct input seam emulates only reader pause acknowledgment and filled DMA
buffers; the real operational consumer performs demultiplexing and restoration.

Once the [production owner](../../saturn-bridge/src/tx_thread/production_owner.rs)
has started, acquisition-attachment and SATP-startup errors enter cooperative
shutdown instead of returning early and dropping a live join handle. Acquisition
continues until the owner reports completion or failure, then the runtime joins
it. Constructor errors and premature Runtime drop also join rather than detach.
Cold native construction can delay a startup-timeout return while that join
finishes; this is not a measured or bounded CM4 response time.

An acquisition failure returns failure even if subsequent restoration and native
cleanup finish safely. A failed restoration returns failure with Phase::Blocked
and no cleanup-complete timestamp. The normal wrapper writes a final failed
review status rather than declaring a production-ready release. No success
receipt is fabricated to dispose of uncertain native state.

## Prekey observations remain inadmissible

The new Direct runtime exercise exposed feedback accumulated after layout
preparation but before key receipt. Its original timestamp genuinely precedes
keying. The [controller](../../saturn-bridge/src/puresignal/controller.rs) now
rejects that block as NotKeyed without latching a false FutureObservation fault.
No sample is retimed or submitted to WDSP. The first completely post-key block
can proceed under the existing admission policy. Future observations still
latch a fault, and settling and stale-age requirements are unchanged.

A deterministic [controller regression](../../saturn-bridge/src/puresignal/tests.rs)
checks both backends, unchanged age, zero queued/native work on rejection,
successful subsequent processing, and a truly future observation fault.

## Executed local evidence

The [runtime tests](../../saturn-bridge/src/shared_backend_tests.rs) use real
matched WDSP on local x86_64. Each failure test runs in a separate process so an
uncertain native channel is never reused to make another test pass.

| Check | Asserted result |
|---|---|
| Normal runtime on P2 and Direct | Actual listener receives both IQ and audio, settings commit, two corrected generations, ordinary release, matching restoration, fresh authorized re-arm, then shutdown while Active. |
| Shutdown restoration delay | External I/O withheld for 80 ms during restoration; repeated runtime turns continue, cleanup-complete remains absent during the hold, and final state is ReadyOff after restoration and owner join. This is an injected delay, not a hardware timing result. |
| Acquisition read error on both backends | Original error returned; restoration completes, native becomes ReadyOff, owner joined. No successful overall runtime result. |
| SATP bind failure on both backends | Occupied loopback UDP endpoint forces actual SATP startup failure after owner spawn; no client admission, owner joined after safe cleanup. |
| Direct attachment refusal | Wrong ordinary-file build identity forces the actual acquisition attach gate to refuse; runtime stops and joins its already-started owner. |
| Failed Direct restoration | Corrupted register-file readback makes the actual acquisition restore fail and invalidate its Link. Runtime returns the readback error, remains Blocked with no native cleanup-complete timestamp, and joins the owner. Best-effort restored registers and a closed output fence do not certify completion. |
| Native constructor refusal | Held legacy native lease prevents shared construction; constructor returns ExternalController after join; a fresh native lease is obtainable afterward. |
| Managed stub regression suite | 551 binary, 49 library and 14 harness tests passed. |
| Default Legacy stub regression suite | 464 binary, 25 library and 14 harness tests passed. |
| Existing real-native combined regressions | Seven passed; historical Opus cases are not managed-Opus approval. |
| Previous matched production paths | Two serial tests rerun against this source. |
| New native runtime tests | Six serial tests, including eight run_inner scenario/backend executions and constructor refusal. |
| Web | 812 tests; typecheck, production build and asset checks. No browser/proxy experiment repeated in this slice. |

Commands, outputs and preserved development failures accompany the source
snapshot. The original control-lane-only test client could not receive RX media;
the final unpaired client explicitly proves IQ and audio delivery. A fixture
that flooded normal RX alongside feedback produced a real SequenceGap refusal;
the final synthetic endpoint respects its bounded delivery rate. Neither failure
was bypassed by relaxing transport fault handling.

## Candidate identity and scope

`review.json` carries the exact source-content pin, ARM binary hash, WDSP archive,
observer source/header pins and web asset identity. The Git base remains
`cc09c61f91030d4c5a38d6c570a20c28b5d97e03`; these uncommitted sources are not
identified by that commit alone. Both checksum indexes and ZIP integrity are
verified after packaging. The prior wiring ZIP remains immutable.

The ARM candidate is cross-built for AArch64/Cortex-A72 against the retained
Trixie sysroot and pinned WDSP version 210 archive. The public ABI transcript
records the command and successful syntax-only status. ARM has not executed.
No G2 connection, install, restart, RF test, commit or push occurred. Firmware,
kernel driver, RXC1 polling, GoldMarsh's branch and the original worktree remain
unchanged. Banked Opus remains stopped; tagged PCM and SATP keep the single owner.

## Controlled hardware review

Use the [G2 procedure](CONTROLLED_G2_PURESIGNAL_PROCEDURE_20261013.md) only after
candidate review and separate approval for its no-RF and RF phases. The measured
feedback-level and delay entries remain prerequisites, not borrowed replay
defaults. Local orchestration evidence establishes neither physical RF-off,
installed WDSP identity nor CM4 deadline compliance. P2 command completion and
Direct control readback remain different host evidence. Existing slow telemetry
is supplemental, not fast overload protection; unclipped capture qualification
remains unavailable without valid interval coverage.
