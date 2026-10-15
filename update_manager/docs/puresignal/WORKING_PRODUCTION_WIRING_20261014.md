# Historical working checkpoint — superseded

This checkpoint predates the completed offline host-wiring slice. For the current
state and remaining limits, read [the production wiring review](PRODUCTION_WIRING_ARM_REVIEW_20261013.md).

The accepted 20261013 production-base ARM bundle stays unchanged. This work is
in the separate `saturn-puresignal-production-review` worktree on `cc09c61…`.
There has been no deployment, RF operation, G2 change, commit/push, FPGA change,
driver change or RXC1 polling change.

## Evidence cleanup

The earlier empty ABI-check log was insufficient evidence. The new runner
`saturn-bridge/scripts/verify-puresignal-arm-abi.sh` records compiler version,
header/declaration hashes, actual command and exit status. It completed with
`exit_status=0`; transcript:
`/tmp/puresignal-production-arm-native-abi-command-status.txt`.
This remains syntax-only checking, not ARM execution.

## Work implemented so far

- New opt-in Cargo feature `managed-puresignal`; default builds remain unchanged.
- Build-matched native observer (`wdsp/ps_observer.c`). Both archive and every
  prepared private/public header are pinned in `build.rs` before compilation.
  The observer uses atomic **loads**, not modifying `InterlockedAnd` calls;
  ported IQC run/busy fields are C `long` (64-bit on these targets), not i32.
  Public ABI comparison uses a separate translation unit: private `comm.h` and
  public `wdsp.h` must not be included together because their port declarations
  and macros conflict. `FullNative::matched` consumes the matched observer.
- Acquisition Link with generation-specific prepare, retirement and matching
  completion. Restoration is forbidden until backend stop evidence permits it.
  Backend stop completion is latched to avoid repeating TX FIFO register writes
  while the RX owner restores the layout.
- Fixed feedback storage pool and stream framing. Direct samples accumulate to
  1,024-pair deliveries, preserving oldest observation time; P2 retains packet
  sequence semantics. No missing counter coverage is fabricated.
- Operational XDMA reader pause acknowledgments use unique serials; a stale
  pause acknowledgment cannot authorize another register transaction. Filled
  buffers carry scope and observation captured **before** DMA read. The same
  OperationalRxSession saves/applies/readbacks/restores paired layout plus DDC6,
  retires queued buffers and partial history, and demultiplexes feedback/RX.
- P2Session's existing receive socket services paired-DDC layout requests and
  routes raw feedback through the same bounded framing path. No extra socket
  or reader exists. Managed feedback does not reach the legacy TX queue.
- Data faults close the matching output generation, discard decoder history
  and preserve acquisition restoration service. Retired-generation data faults
  cannot close a freshly reopened generation. Layout I/O failure stays latched;
  best-effort restoration is not reported as successful completion.
- Reserved TCI startup gates client admission until shared-owner installation
  and clean committed configuration. Frontend drop terminates its accept loop.
- `production_owner.rs` constructs the sole full native owner in its own thread
  and attaches the same lifecycle/actual output adapters. It has explicit
  limits, native/source pin requirements, independent RF permission and topology
  attestation. Strict admission is the default; operational admission requires
  explicit supplied limits rather than promoting fixture thresholds.

## Not finished — continue before handing off

1. **Main/backend entry is not connected yet.** `Selection::parse` and Shared
   construction exist, but main still calls the original Legacy branch. Do not
   claim production wiring complete or build/publish a new candidate as ready.
2. Finish actual runtime RX/control/stop orchestration for both backends. Keep
   RX servicing restoration until TX stop/native cleanup completes; do not stop
   the reader first or let shutdown join wait indefinitely on an absent owner.
3. Preserve P2 heartbeat ownership with generation-tagged HP writes while keyed
   and receive-safe-only writes while off. Do not restart its untagged legacy
   producer or create a parallel TX engine.
4. Finish P2 queued-datagram retirement/source checks. Kernel/wire arrival age is
   not yet bounded; bridge-observation timestamps do not prove exact RF age or a
   physical firmware generation. Do not overstate this guarantee.
5. Add actual production-path tests (not only Link/decoder tests): pause/read
   races, layout rollback/readback failure, buffered partial frames, stale
   completions, release/fault during apply, restore-after-unkey ordering,
   matched observer cleanup, disconnect/re-arm and no parallel legacy owner.
6. Re-run native suites and source-integrated fixtures, then pin new source,
   build matched ARM executable with `managed-puresignal`, and package honest
   evidence with a nonempty ABI transcript. No ARM execution/RF qualification.

## Current execution evidence

Before the last restoration-order refinement, the managed-feature stub suite
passed 48 library tests, 540 binary tests and 14 replay-harness tests; two binary
benchmarks were ignored. Log:
`/tmp/puresignal-working-production-all-stub.txt`.
This is not proof of production startup or native matched-observer execution.
Later checks/results must supersede it where source changed. Native check is
being run against the recovered verified x86 archive; the seven combined native
tests still need a fresh rerun after these changes.

## Local build inputs / preservation

- Recovered verified source/native tree:
  `/tmp/saturn-production-review.XFOJ8g/offline-native/wdsp-build`.
- Matched ARM WDSP archive and headers:
  `/tmp/saturn-production-review.XFOJ8g/arm-native-reviewed`; archive
  `f5f21c7cb76df1add17647c15a700f564a7f045e930e84be870333fa6e74973e`.
- Read-only-copied ABI sysroot:
  `/tmp/saturn-production-review.XFOJ8g/arm-sysroot`.
- Clang `/usr/bin/clang`; Rust 1.98.1; ARM linker wrapper is
  `saturn-bridge/scripts/link-reviewed-arm.sh`, with explicit `SATURN_ARM_LLD`
  and `SATURN_ARM_SYSROOT`. ARM Rust flags: `-C target-cpu=cortex-a72`.
- Original review worktree, GoldMarsh branch and Downloads archives preserved.
  Do not overwrite the previously accepted ARM review bundle/source snapshot.

No live protection latency, feedback thresholds, RF shutdown or exact installed
RXC1 pair topology is qualified by this checkpoint. Existing slow telemetry is
supplemental, never advertised as fast protection.
