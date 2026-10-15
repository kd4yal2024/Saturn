# PureSignal milestone 2 — review fixes and combined native replay

SilverForge, 2026-10-12. Follow-up to the [first offline cut](MILESTONE2_OFFLINE_20261012.md).
Target firmware stays **1.31.002 / 0x53460004**; no installed identity was read this turn.

## Outcome

All three reviewed implementation defects are fixed with regression tests.
Both wire replay adapters now feed the **same shared Rust controller and a real
WDSP NativePs port** in an isolated offline process. Unkey during native fitting,
configuration reset after correction, fresh-epoch recalibration, stale feedback,
and destruction pass while native TX DSP continues advancing.

**Live integration remains blocked by missing acquisition evidence.** The existing
owners cannot supply the controller's three complete overload brackets within
its deadline. The policy was not relaxed, polling was not accelerated, and no
periodic snapshot was relabeled as sample-aligned evidence. This is an integration
decision, not proof that unchanged RXC1 firmware cannot support PureSignal.

No G2 connection, RF test, register/DMA access, service action, deployment, FPGA
change, driver change, or polling-policy change occurred. Production P2 behavior,
Direct XDMA's PS prohibition, and normal TX approval are unchanged. New native
port code is under `tests/support` and is not imported by the production library
or executable. No UI tab or pop-out panel was added.

Work remains uncommitted in the isolated `silverforge/puresignal-milestone1`
worktree, based on `861f0fbc79e0e36c68a5c8ddc157e010b8be20df`. GoldMarsh's
original worktree and changes were untouched; nothing was pushed.

## Three bounded corrections

| Review finding | Correction | Regression evidence |
| --- | --- | --- |
| Partial DMA frame rejuvenated on completion | [DirectXdmaAdapter](../../saturn-bridge/src/puresignal/adapters.rs) stores the minimum arrival timestamp across every held word, including the header conservatively; clears it on completion/loss. This remains host arrival time, not an FPGA timestamp. | Both 9-word and 17-word layouts held across a 300 ms pause fail the 250 ms gate despite fresh final-word/counter evidence. The next frame gets its own timestamp. |
| Settling checked consumption time | [Controller::check_meta](../../saturn-bridge/src/puresignal/controller.rs) requires the oldest observation to reach the key-down plus settling boundary. Late submission cannot change that observation. | Key-down at 10, settling 100: observation 109 submitted at 200 is rejected; observation exactly 110 is accepted. No native call or fault latch for the rejected settling block. |
| Equal saturated counters certified clear | [CounterBracket](../../saturn-bridge/src/puresignal/mod.rs) requires a source-qualified width 1..64 plus explicitly false saturation and overflow covering the entire bracket. Counts at or beyond the representable maximum cannot certify clear, even if flags claim false. Unknown width/status also fails closed with `CounterInexact`. | 1/8/32/64-bit maxima, missing/invalid width, true/unknown saturation, true/unknown overflow all fail. An inexact DDC bracket latches the controller fault without feeding native DSP. |

[Wire contracts](contracts.ts) now carry end-counter generation, width, saturation,
overflow, and the `counter-inexact` reason. These are required evidence fields,
not promises that the current owner produces them. [Rust tests](../../saturn-bridge/src/puresignal/tests.rs)
exercise all three fixes.

## Acquisition coverage decision

Audit scope is the pinned Bridge source above, **not a new inspection of the G2**.

| Existing source | What it actually provides | Why it cannot satisfy current per-block admission |
| --- | --- | --- |
| [RXC1 owner](../../saturn-bridge/src/rx_counter_v31.rs) | Opt-in, durably armed snapshots of five u64 counters for each of ten DDCs; reset/session/config/token validation; overflow and `exact` reporting. Internal `PERIOD` is **5 seconds**. `accepted_clamped_iq_components` is the relevant clamp count. | Needs snapshots before and after the measured interval. Waiting for the next unchanged-cadence snapshot exceeds 250 ms, and even exceeds the configuration's maximum 1 s freshness allowance. Snapshots are acquired per DDC, not atomically with the paired sample stream; JSON has no matching first/end sample ordinals. Counter accepted-pre-FIR ordinals cannot simply be treated as post-FIR DMA ordinals. |
| [XDMA worker](../../saturn-bridge/src/xdma_backend.rs) | Calls extended/RXC1 sampling in its 1 s performance section; RXC1 still enforces its own 5 s period. Basic ADC status is read in the 5 s status section. | Neither schedule supplies qualified per-feedback-block coverage. Changing the caller to poll more often is not authorized and would not establish sample alignment. |
| [ADC extended telemetry](../../saturn-bridge/src/xdma_rx.rs) | Coherence-checked snapshots of u32 episode counts/high-clock totals, episode activity/validity, snapshot generation and ADC clock rate. Basic status exposes two overflow bits and peaks. | More useful than an instantaneous bit alone, but no current producer binds the before/after evidence to paired-capture ordinals/config/TX epoch, with full reset/wrap/saturation validity and the required latency. Snapshot coherence is not capture coverage. |
| [P2 high-priority status](../../saturn-bridge/src/p2/packets.rs) | ADC overflow byte and power/peak status; P2 feedback packets have wrapping outbound sequence and a zero timestamp at the reviewed packetizer pin. | No complete ADC-overrange interval bracket or both DDC-clamp brackets arrive through the reviewed P2 interface. Outbound sequence is not FPGA acquisition time or clamp coverage. |

**Selected policy:** keep the strict controller gate; with current real evidence,
live feedback admission is unavailable. Neither backend may populate complete
clear brackets from cached status or periodic snapshots. The offline generator's
clear brackets are explicitly synthetic: it knows each modeled sample and does
not claim to emulate the hardware telemetry producer.

A future owner contract must demonstrate three qualified covering intervals,
their source/reset/config identity, counter width and inexactness handling, and
a bounded delivery latency. If the existing interface cannot do that unchanged,
a separate reviewed admission policy/interface decision is needed. No firmware
or kernel change is justified by this offline slice, and increasing polling is
not an accepted shortcut. Exact frozen RXC1 paired-path verification remains open.

## Combined real-native pipeline

```text
deterministic nonlinear fixture, signed 24-bit wire encoding
       | P2 DDC0 packet                     | paired XDMA mux + DDC6
       v                                   v
  P2Adapter                         DirectXdmaAdapter
       +--------------+--------------------+
                      v
              shared Controller
              bounded/fenced queue
              exact 1024 paired samples
                      v
          OfflineNative implements NativePs
               pscc -> real WDSP fitter
               fexchange0 -> native TXA DSP -> memory discard
```

[Combined replay](../../saturn-bridge/tests/support/puresignal-controller-native-replay.rs)
uses the actual adapter and controller source, not a duplicate controller or fake
native engine. [OfflineNative](../../saturn-bridge/tests/support/native_port.rs)
owns one channel on one host thread (`!Send`/`!Sync`), preallocates its buffers,
applies feedback rate before delay, returns WDSP's quantized delay, and advances
TX DSP with zero-mic exchanges. PS MOX here is a native fitting flag; the process
has no radio-keying or DUC/DMA/socket output path. The fixture's observation clock
is deterministic test time, not a claim of wall-clock transport timing.

Reset ordering matters. The port clears PS MOX, drains an outstanding `LCALC`
without stopping TX DSP, then requests reset and continues exchanging until
native reset/applied status and IQC run/busy flags are clear. A tiny
[read-only observer](../../saturn-bridge/tests/support/puresignal-native-observer.c)
reads the existing atomic IQC flags using the **same verified internal headers**;
it does not change the native archive or algorithms. Stable native-state checks
are justified only under this exclusive port, with no external controls or
correction save/restore. They are not a general public-WDSP cancellation API.

Reset has a 3 s offline wall-clock bound. Failure returns `NativeFailure`; an
unquiesced destructor refuses to free native objects and aborts this diagnostic
process. That is **not** an approved production real-time or RF fault policy.
Production ownership integration and its bounded drain/unkey envelope remain open.

The test verifies unkey during `LCALC`, continued DSP advancement, cleared raw
capture/queues, rejection of old-generation callbacks, and a new calibration
after rekey (the drained old fit cannot be attributed to the new epoch). It also
resets an applied solution during configuration, recalibrates, induces staleness,
and verifies the fault survives unkey/rekey and final CloseChannel is quiescent.
Native display arrays are not assigned invented capture timestamps after reset.

Two completed combined runs observed:

| Backend | Real feedback blocks | DSP exchanges | Reset-only DSP exchanges | Resets / fit-time unkeys | Final IQC flags / closed |
| --- | --- | --- | --- | --- | --- |
| P2, initial combined build | 419 | 730 | 311 | 12 / 1 | 0 / true |
| Direct XDMA, initial combined build | 321 | 632 | 311 | 12 / 1 | 0 / true |
| P2, final source build | 439 | 756 | 317 | 12 / 1 | 0 / true |
| Direct XDMA, final source build | 323 | 634 | 311 | 12 / 1 | 0 / true |

Counts vary with asynchronous native scheduling; they are diagnostic observations,
not timing qualification or exact expected counts. The standalone native fitting
positive control also passed again: **4096 measured / 512 correction points per
array, canaries clear, delay 150 ns requested / 140.224359 ns returned**, modeled
amplitude RMS **0.102006762 -> 0.00147866877**. That modeled inverse-curve check
is separate from a full closed-loop TXA/RF or measured IMD result.

## Verification and independent replay

- Shared-core regression tests: **24 passed**, also runnable with plain rustc.
- Existing Bridge stub-native binary tests: **377 passed, two ignored**.
- Browser tests: **699 passed in 82 files**; TypeScript check passed.
- Rust library Clippy with warnings denied, runner ShellCheck, source/header ABI
  checker including older-signature negative test, and diff whitespace check passed.
- Real combined native replay: both backends passed; standalone fitter passed.

Native identity remains the previously built **x86-64 Linux offline** WDSP 2.10
archive, upstream `b02d5bac675dd2f33ec2bab2b339f79a597c47dd`, Linux port
`974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`, GCC 11.4.0 / FFTW 3.3.8:

- Archive: `8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
- Final combined executable: `eac1a322469df3c9797acc7ed2360dda2065092e1cbb976d6eecaf9bd5a24a77`.
- Standalone executable: `5576813ab72bde01d0309572b28105c6a6720c410b902dcd19b1acec2863ddcb`.
- Standalone replay source remains `5000f3f3ff1c268f59952fe5577d59fb162b71f76f6193de60c14aa9f457f6ab`.

The [runner](../../saturn-bridge/tests/run-puresignal-offline-native.sh) checks
archive/public/internal-header identities before linking, runs the 24 shared-core
tests, builds the combined and standalone replay, applies process timeouts, and
prints binary hashes. It writes only a fresh local `/tmp` directory, retained
for inspection; it has no deployment command. Public channel/exchange signatures
are checked alongside the PureSignal declarations in [native-abi.h](native-abi.h).

From the extracted follow-up package root, on compatible x86-64 Linux with
`rustc`, C compiler/binutils, FFTW development files, and coreutils installed:

```sh
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh \
  offline-native/wdsp-build
```

The follow-up package includes the exact offline archive, its corresponding
ported C/header sources and license, build helper sources, and a SHA-256 manifest.
It is **not a G2/ARM library or installation payload**. Rebuilding with another
toolchain requires a separately reviewed identity; do not bypass the hash gate
or substitute an unknown installed library. Original M1 and first M2 ZIPs remain
unchanged. Milestone 2 is still in progress; installed library identity, exact
RXC1 path, acquisition coverage, production owner binding, display association,
measurement analyzer, and panels are not qualified by these offline passes.
