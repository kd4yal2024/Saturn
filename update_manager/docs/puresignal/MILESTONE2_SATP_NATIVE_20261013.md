# PureSignal milestone 2 — SATP provenance and full native TX wrapper

Date: 2026-10-13. Isolated SilverForge base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome and scope

Implemented SATP provenance first, then a shared-lifecycle wrapper around the
existing full `WdspTxEngine`. The gated scheduled replay now runs SATP through
the actual WDSP variable-rate matcher and full TX engine, establishes correction
through the P2/direct decoder paths, and exercises source changes, fresh re-arm
and a source discontinuity while real native fitting is in progress.

This is an **offline integration deliverable**, not production activation or RF
qualification. Live selectors, socket/UDP ingress producers and the default TX
loop remain unchanged. G2, installed firmware 1.31.002 / 0x53460004, FPGA, kernel
driver and polling are untouched. No commit/push or deployment; GoldMarsh's
worktree is untouched. All radio-output tests use loopback UDP/ordinary files.

## Source-linked age path

```text
original ingress stamp / observation time
    -> partial 64-input packet tail
    -> conservative union of retained 140-input FIR history and new chunk
    -> shadow labels for the actual native output-ring occupancy
    -> 512-sample native mic block, retaining its oldest source observation
    -> full native TX exchange -> generation-tagged IQ / partial DUC tails
```

The [SATP provenance adapter](../../saturn-bridge/src/tx_thread/satp_provenance.rs)
owns the existing [MicRateMatcher](../../saturn-bridge/src/wdsp.rs), not a separate
DSP thread. The pinned `rmatch.c`/`varsamp.c` sources establish this profile:
64-sample input calls, 512-sample output calls, 48 kHz nominal input/output,
140 retained raw inputs in the resampler FIR, and a 4,096-sample output ring
initially half-filled with zeros. The native runner now checks those source/header
hashes; a C probe verifies the constructed native profile's actual retention.

| Retained state | Provenance treatment |
| --- | --- |
| Partial incoming chunk, up to 63 samples | Keep each original timestamp; a later packet cannot replace it |
| FIR input history | Track the last 140 raw-input timestamps; each output chunk conservatively covers previous history plus all new inputs |
| Native output ring | Infer the exact number of newly written outputs from serialized occupancy before/after each 64-input call; retain a label per output sample |
| Initial primed zeros | `oldest=None` means known constructor-primed silence, not missing provenance; actual native zeros are tested |
| Underflow/slew or overflow/blend | Invalidate the generation instead of inventing provenance for repeated/blended samples |
| Native-generated IQ and DUC packet tails | Preserve the mic block's oldest source observation and original output generation through the existing owner span accounting |

Labels intentionally overapproximate the FIR support of each chunk. They may be
older than an exact coefficient-wise minimum; they must never be newer. The
age floor advances only after the FIR and output ring have both retired the old
inputs. Fresh arrivals after a stall cannot refresh the FIR, ring or packet tail.

This timestamp is the **supplied bridge ingress observation**, not a reconstructed
SATP sender capture time or measured RF emission time. The live SATP jitter-buffer
and sequence/discontinuity producer still need production wiring to preserve that
origin before its own buffering. The full TX engine's internal filter/group delays
and remote delivery delay are not qualified here as exact end-to-end sample ages.
No capture is certified unclipped merely from these source-age labels.

## Generation and discontinuity rules

[OwnerLoop](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs) enables SATP only
when its native port explicitly reports the verified history profile. Ports
without that capability preserve the inhibited SATP default. The production
selector does not choose the new port.

A clean matcher is prepared only in ReadyOff, after completed stop receipt and
native cleanup. A fresh authorized arm binds it to the new output generation.
Release, source reconfiguration, fault or flush invalidates the old binding;
retained native state cannot be rebound to a newer generation. The next clean
matcher begins with new, verified zero state.

Feedback `tx_epoch` changes at the key receipt but the TX output generation does
not. Already admitted source history may cross that feedback-only boundary
without resetting its age. Audio validation still requires matching boot,
configuration/acquisition generation, output generation, revision and release
ticket. Old audio across an actual cancellation/reconfiguration generation remains
rejected. Control/configuration commands retain the stricter full fence check.

Unexpected occupancy/counter/ratio changes and reorder/stale observations stop
the generation. Underflow is detected before calling the native slew path;
overflow is detected immediately after the bounded input call and before output
delivery. Flush is an out-of-band release, not an automatic re-arm. The accepted
post-pump eligibility and missing-authority correction remains in place.

## Full native DSP wrapper

[FullNative](../../saturn-bridge/src/wdsp/ps_native.rs) owns one actual
`WdspTxEngine`, uses the same [model synchronization and DSP implementation](../../saturn-bridge/src/wdsp.rs)
as the existing transmitter, and implements `NativePs`, `CooperativeNative` and
`ScheduledNative` on that one instance. It is not Send/Sync; a second FullNative
claim is rejected before opening another channel. This is not installed-process
owner discovery/arbitration; no live legacy owner is selected alongside it.

Model/filter/mic gain, EQ, CFC/leveler, phase rotation, speech compressor/overshoot
control, noise gate, DEXP and FFT/latency settings use the existing engine code.
The managed constructor skips the legacy engine's independent PureSignal
initialization/enable/reset loops, including on channel recreation; the default
legacy constructor retains its original behavior. In the managed port,
the shared controller owns PS feedback rate, peak, delay, MOX/control, `pscc`,
faults and cleanup. The wrapper does not call legacy blocking stop/reset loops.

The new exact-block exchange takes 512 mono samples and produces 4,096 IQ floats
with exactly one native exchange, propagating native/nonfinite errors. It does
not enter the legacy unbounded queue loop. `pscc` admission performs no exchange;
rendering, stop progress and cleanup advance the same native channel. Read-only
audit/diagnostic access grants no mutable native port or DSP budget.

After the matching stop receipt, cleanup continues zero-input DSP while any
fitter/IQC transition completes. It requests PS reset only after the current fit
is settled and verifies clear native IQC flags. Then it closes/recreates the same
engine through its existing recreation path, eliminating old TX filter, meter,
ALC/EQ/CFC/DEXP and input/output sample state before re-arm. Recreation completes
before native-clean notification. There is never a second engine running beside
the first; unsafe/unproven native lifetime cleanup retains the engine fail-closed.
The captured TX model is synchronized again on the new engine, restoring EQ/CFC
and other feature settings initialized disabled by channel creation. The
retained shared-controller PS configuration is reapplied to the recreated
channel before completion, including feedback rate, hardware peak, loop delay
and quantized TX delay. This covers ordinary release/re-arm with no new config
command. A changed/nonfinite actual delay fails cleanup rather than silently
changing the controller's contract.

Channel creation/recreation and matcher allocation/destruction are native
operations with **no demonstrated wall-clock bound**. The one-exchange-per-turn
limit remains, but it is not a promise of G2 response latency. Release fencing
does not wait on these operations. Live integration still needs its timing and
installed-library identity qualification.

Two-tone, WFM and unknown-mode admission remain inhibited in this wrapper. USB
and the exercised model-feature/source combinations are the real replay evidence;
other normal modes inherit the existing engine implementation but are not declared
qualified by this slice. The previous fixture-only USB subset is now supplemented
by this full-engine test, not falsely relabeled as broad RF qualification.

## Executed tests

Seven provenance unit cases cover partial-packet/FIR/ring age retention, a stale
partial tail, fresh arrivals behind old ring data, retirement of the old age
floor, overflow and refusal to rebind old history across generations. The actual
native matcher test verifies primed silence, sustained fresh production retaining
older observations, FIR staleness after a stall and new-generation zero state.

The new combined full-engine replay runs both actual owner adapters. It checks
actual native EQ/CFC/phase/compressor/overshoot/gate flags and mic gain, confirms
DEXP exchanges, establishes correction from decoded feedback, releases, switches
SATP→TCI→SATP, rejects old-generation IQ, and flushes SATP while native PS state
is in LCALC. Subsequent scheduled turns obtain stop evidence, continue native
cleanup/recreation and finish ReadyOff with zero IQC flags. Native audit asserts
at most one DSP exchange per turn throughout cleanup and re-arm, not just active TX.
An additional ordinary release/re-arm in that replay sends no configuration
command: the native getter must retain a nondefault hardware peak, the controller
retains its actual delay, actual native feature flags and mic gain remain intact,
and the fresh generation must establish correction again. The synthetic waveform
uses its matched zero TX delay; nonzero end-to-end delay is not qualified by this
full-engine replay.

| Check | Result |
| --- | --- |
| Owner-feature Cargo, native stubs | 46 library + 408 binary passed; 2 existing ignored |
| Default Cargo, native stubs | 24 library + 377 binary passed; 2 existing ignored |
| Real WDSP owner runner | 4 serial tests passed: prior owner handoff, scheduled subset, full engine/SATP integration, native matcher provenance |
| Standalone strict/proposal/lifecycle | 24 / 32 / 46 passed; real replay and numerical canaries passed |
| Formatting, whitespace, runner ShellCheck | Passed |

Current transcripts are `offline-native/satp-full-*`; prior reports/results are
historical. These are SilverForge's execution evidence, not the external reviewer's
independent execution. Reproduce from extracted root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
bash update_manager/saturn-bridge/tests/run-puresignal-owner-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
```

Requires Rust/cached Cargo dependencies, compiler/binutils and FFTW development
files. The pinned x86-64 offline WDSP archive remains SHA-256
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
The owner runner validates 22 archive/source/header inputs before building native
probes. **Never install this archive on G2.**

Review the SATP provenance and full native wrapper as the next bounded integration
slice. Production ingress/selection, installed library/RXC1 feedback topology,
path-specific thresholds and live RF qualification remain open. P2 evidence is
UDP command completion; direct evidence is ordinary-file readback with synthetic
FIFO occupancy. Neither proves physical RF-off. No additional FPGA, driver or
polling work follows from these offline tests.
