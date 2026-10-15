# PureSignal milestone 2 — post-pump stop progression correction

Date: 2026-10-13. SilverForge isolated base:
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome

The source-review finding was valid: the phase checked before `pump()` was no
longer sufficient after it serviced release or latched a feedback/native fault.
Falling through into due rendering could find retired output authority and
panic at `stamp.output.unwrap()`. Closing the output fence did not establish a
completed stop receipt; the panic could prevent further scheduled stop/cleanup.

The [owner loop](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs) now:

- Rechecks lifecycle eligibility immediately after `pump()`. If the phase is
  no longer AwaitingKey/Active, it clears local buffers and finishes the turn
  without rendering or sending output.
- Handles missing output authority with an explicit `Option` branch instead of
  `unwrap()`. The existing zero-exchange cancellation service gets first refusal;
  an existing fault or ordinary release is not replaced by a new artificial
  error. Missing authority while still active requests a controlled stop.
- Returns to the scheduling driver so the **next** turns obtain the matching
  stop receipt and progress cooperative native cleanup. It does not run a second
  lifecycle tick/cleanup exchange in the faulting turn or assume RF is already off.

The changed consumer remains offline-gated. Default production selection, live
producers, G2, firmware 1.31.002 / 0x53460004, FPGA, driver and polling are
unchanged. No RF testing, service/deployment change, commit or push. GoldMarsh's
worktree remains untouched.

## Deterministic regressions

[Scheduled tests](../../saturn-bridge/src/tx_owner_stop/scheduled.rs) use the real
controller/lifecycle and actual direct owner with ordinary register/DMA files.
The native failure and status indications are injected through the test native
port; they are not observations of the installed radio/native library.

Each case reaches Active, drains setup IQ, queues feedback, and explicitly makes
both rendering and sending due. A callback at the actual tick/pump boundary
allows a release to arrive **after** the initial tick and **before** pump without
sleep-based races. Normal turns pass an empty callback through the same code.

| Case | Required result |
| --- | --- |
| `process()` returns native failure during pump | Original `NativeFailure` latched; no render/output |
| `GetPSInfo` severe-overdrive bit during pump | Original `SevereOverdrive` latched; no render/output |
| Release after initial tick, before pump | Pump services release; no artificial fault, render or output |
| Authority retired without a local mic stamp | Missing snapshot authority is handled without unwrap; controlled authority stop |
| Healthy feedback/rendering control | One native rendering exchange and one actual owner output write; then orderly shutdown |

The stopping cases assert zero further rendering, packet dispatch and actual
backend DMA-write-counter increments, both in the observed turn and throughout
subsequent cleanup. Immediately after the faulting turn, the lifecycle must be
AwaitingAuthority with **no completed receipt and no cleanup yet**. Subsequent
scheduled turns must produce matching direct control-readback evidence and finish
native cleanup in ReadyOff. The ordinary-file control state is then checked off.
This is not physical RF-off measurement. Each turn retains the one-exchange limit.

## Execution evidence

| Check | Executed result |
| --- | --- |
| Focused pump-boundary regressions | 5 passed |
| Owner-feature Cargo, native stubs | 46 library + 401 binary passed; 2 existing ignored |
| Default Cargo, native stubs | 24 library + 377 binary passed; 2 existing ignored |
| Combined actual-owner / real-WDSP replay | 2 serial tests passed; each exercises P2 and direct fixtures |
| Standalone strict / proposal / lifecycle | 24 / 32 / 46 passed; native replays and numerical canaries passed |
| Formatting, whitespace and runner ShellCheck | Passed |

These are SilverForge's executed results, not the external reviewer's independent
execution. Current archive transcripts are `offline-native/pump-stop-*`; earlier
scheduled-owner and other reports/results are historical. From extracted root:

```sh
sha256sum --check SHA256SUMS
cd update_manager/saturn-bridge
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --offline --features offline-puresignal-owner-adapters scheduled_pump -- --nocapture --test-threads=1
```

The existing three runner scripts reproduce the full owner, real native and
standalone suites from the prior report. Requires cached Cargo dependencies,
Rust, compiler/binutils and FFTW development files. Pinned x86-64 offline WDSP
archive remains SHA-256
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
**Never install that archive on G2.** No new installed-library, feedback topology,
physical RF-off, live threshold or G2 latency qualification is claimed.

## Remaining migration order

The correction does not enable SATP or finish the installed native DSP wrapper.
Continue SATP sample provenance first, then the full wrapper, retaining the shared
lifecycle, native owner, fresh-authority and output-fence boundaries. The pinned
rate-matcher implementation retains resampler history, ring samples and slew
state; queue arrival time alone cannot certify its output age. Provenance must
cover that retained state and invalidate discontinuities rather than treating
new arrivals as fresh timestamps for old output. This requires offline native
adapter work, not new FPGA counters, faster polling or G2 tests.
