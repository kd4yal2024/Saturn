# PureSignal milestone 2 — first offline implementation and replay

SilverForge, 2026-10-12. Target firmware remains **1.31.002 / 0x53460004**.

## Outcome and limits

The three review corrections are incorporated, and a shared host controller,
strict P2 paired decoder, and bounded Direct XDMA paired DMA decoder now exist
as offline-testable code. A freshly built, pinned WDSP archive also passed a
real numerical fitting/display-buffer positive control. **Milestone 2 is not
yet complete: this does not establish working PureSignal on either live backend.**

No G2 connection, deployment, service action, register/DMA access, RF test,
firmware change, kernel change, or RXC1 polling-policy change occurred. Normal
TX approval is unchanged. The new controller/decoders and browser consumer are
not selected by production code, and the three panels are not wired. Existing
P2 PS behavior and Direct XDMA's production PS prohibition remain unchanged.

All work is in `/home/jd/worktrees/saturn-puresignal-m1`, branch
`silverforge/puresignal-milestone1`, base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`. Nothing was committed, pushed,
cherry-picked from GoldMarsh, or edited in the original dirty Saturn worktree.
The isolated binary's five-line library-dependency import is solely needed to
retain Cargo's native-link metadata after adding a library test target; it
does not instantiate PureSignal.

## Three corrected contracts

1. **Independent measurement modes.** [contracts.ts](contracts.ts) separates
   `SpectrumCapture`, `TwoToneMeasurement`, and `BroadbandMeasurement`.
   Capture, two-tone, and broadband products carry independent validity.
   Speech/noise regrowth integrates in-band and adjacent/regrowth power without
   fundamentals, tone balance, IMD orders, or a two-tone PEP conversion.
   `Product<T>` is now a discriminated union: nonvalid products require null
   current data. Runtime and negative compilation fixtures cover these cases
   in [measurement-contract tests][measurement-tests]. This is a corrected
   contract, not a completed FFT/IMD analyzer.

2. **Delivery delay counts toward age.** [freshness.ts][freshness] implements
   a nonce-matched, boot/connection-scoped clock bound. For server reply time S
   and client send/receive times C0/C1, the server-minus-client offset lies in
   `[S-C1, S-C0]`, widened conservatively for clock-rate error. Maximum current
   age uses the upper bound and the original server observation time, not a
   receipt-age shortcut. The initial explicit policy uses a 200ms maximum
   probe round trip, 2s bound lifetime, 500ms consumer-silence limit, and a
   conservative 1000ppm relative clock-rate allowance. Unknown/expired bounds
   cannot issue a fresh certificate. Reconnect or a detected consumer stall
   retires the bound. A new nonce reply establishes an observation barrier;
   buffered telemetry older than that barrier is rejected. The consumer also
   rejects wrong boots/connections, duplicate/reordered product sequence, and
   malformed clocks. [Six replay tests][freshness-tests] include delayed packets
   arriving with zero publication age, stalled delivery followed by queued
   telemetry, and a queued probe reply. Production still needs the probe
   request/reply protocol and reconnect/suspend/timer wiring. A clock-rate
   bound that cannot be justified must remain timing-unverified.

3. **Overload evidence covers the capture.** [contracts.ts](contracts.ts) and
   [the Rust evidence validator][types] replace nullable instantaneous clear
   flags with named ADC/DDC counter brackets. Evidence includes boot,
   acquisition generation, configuration, `[first,end)` source ordinal interval,
   observation time, validity/coverage, and counter-generation/start/end values.
   Clear evidence must cover the entire capture with no reset or counter
   increment. An old clear reading, incomplete bracket, wrong generation,
   wrong physical ADC, unarmed/unavailable source, reset, or decreasing count
   cannot certify a new window. The required sources are the selected physical
   ADC and both DDC0/1 clamp counters. A hit in a broader bracket invalidates
   conservatively; it does not locate the event inside the narrower capture.
   Exact hardware evidence/coverage is **not** manufactured by the decoders.

## Offline host implementation

```text
P2 DDC0 packet replay --- P2Adapter --------+
                                          |
same-owner XDMA words --- DirectXdmaAdapter+--> fenced bounded ring
                 |                              |
          ordinary DDC6 samples                  v
                                          shared Controller
                                          /              \
                             raw RF before mutation    NativePs port
                             bounded analyzer handoff  exact 1024-pair arrays
                                                      serialized native owner
```

| Component | Implemented and tested | Still missing before production selection |
| --- | --- | --- |
| [Shared controller][controller] | Native-owner port; conservative verification gates; boot/config/generation/TX-epoch fences; preallocated bounded ring; exact 1024-pair assembly; queue and partial-window staleness; finite/rate/route/length checks; selected overload coverage; 16 raw native status integers; actual delay return; bitmask severe-overdrive handling; latched faults; old-curve calibration-count baseline; MOX-off before reset/drop. Configure alone cannot key or arm adaptation, even during an existing TX. | Binding to the existing single TXA owner; authenticated command mapping; bounded cross-thread acquisition handoff; automatic attenuation policy; native asynchronous quiescence/event fencing and snapshot association; approved fault bypass/controlled-unkey RF envelope. |
| [P2 adapter][adapters] | DDC0 only; exact 1444-byte, 24-bit, 238-IQ-entry packet; 119 paired samples; signed24/8388608 scaling; feedback then reference order; wrapping u32 continuity; gap/duplicate/reorder/malformed input poisons the adapter until a fresh generation. | Existing P2 owner configuration/restore transaction, ingress fence assignment, fault notification and bounded handoff. Packet ordinals do not prove an FPGA clock or silent FPGA loss. |
| [Direct XDMA adapter][adapters] | Exact pair-only `0x1f` and pair+DDC6 `0x10001f` layouts; four paired samples per mux frame; optional ordinary DDC6 demux; generation-relative ordinals; at most one 136-byte partial frame; split-word delivery; malformed header/layout and explicit loss poison the decoder. Plan distinguishes P2 TX-reference source2 from hardware register source3, and requires shared LO with independent-LO debug off. | Attach to the sole existing acquisition owner; save/apply/verify/restore DDC/tuning/attenuation state; connect FIFO/DMA/host-loss evidence; verify exact RXC1 pair integrity/shared reset/backpressure semantics. No new reader or register transaction was started. |
| [Telemetry consumer][freshness] | Conservative transit-aware freshness and replay tests. | Server probe/envelope implementation and browser lifecycle integration; read-only observer/panel wiring. |
| [Native replay][native-replay] | Real pinned WDSP fitting, display capacities/canaries, returned delay, modeled nonlinear positive control. | The actual `NativePs` implementation on the shared production TX engine; installed-library identity; full closed-loop native TX output, raw-feedback analyzer and native display epoch consistency. |

The controller's `NativePs::reset` returns failure if a future adapter cannot
establish quiescence. Failure retires the generation and refuses a new
configuration. The mock tests cannot prove native asynchronous cancellation.
Correction-applied remains a **native reported flag**, additionally requiring
a completed calibration newer than the generation's baseline; it is not an
RF qualification or a claim that native display data is atomically timestamped.

`observed_ns` in decoder replay is host delivery time. Source ordinals are
explicitly generation-relative. Neither is silently promoted to an FPGA
acquisition timestamp. Native or browser freshness checks do not detect
unreported upstream delay/loss. Those limits remain part of the topology and
transport qualification.

## Real WDSP replay evidence

The numerical replay was built from detached, clean source pins:

- TAPR WDSP `b02d5bac675dd2f33ec2bab2b339f79a597c47dd` (2.10 / PS3).
- Linux port `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`.
- Existing Saturn `build-wdsp2-linux-arm.sh`, executed on **x86-64 WSL**, with
  target `generic`; the archive is not an ARM/G2 artifact. Compiler: Ubuntu
  GCC 11.4.0; FFTW 3.3.8. Existing numerical flags/patches were preserved.
- Built `calcc.c`, `TXA.c`, and `wdsp.h` hashes match the independently checked
  upstream blobs; no fitting algorithm was patched.
- Archive SHA-256:
  `8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
- Replay executable SHA-256:
  `5576813ab72bde01d0309572b28105c6a6720c410b902dcd19b1acec2863ddcb`.
- Replay source SHA-256:
  `5000f3f3ff1c268f59952fe5577d59fb162b71f76f6193de60c14aa9f457f6ab`.

The fixture supplies deterministic varied-envelope complex reference samples
and a known memoryless model with AM/AM compression and AM/PM rotation. It calls
the real `pscc`, `GetPSInfo`, and `GetPSDisp`. Guard words surround each measured
and correction array, feedback/reference input, DSP exchange buffers, and
native output counts/phase/status storage. It checks exact populated capacities,
finite correction values, successful native calibration/application, and
improvement when the resulting inverse magnitude curve is applied to the
known synthetic model. This is not the RF spectrum analyzer or an IMD result.

Three successful runs produced:

| Native replay result | Observed value |
| --- | --- |
| WDSP version | 210 |
| Completed calibration / applied reported | 1 / true |
| Measured / correction capacities populated | 4096 / 512 per array |
| Input, display and DSP canaries | Passed |
| Requested / native-returned delay | 150ns / 140.224359ns |
| Uncorrected modeled amplitude-error RMS | 0.102006762, normalized |
| Corrected modeled amplitude-error RMS | 0.00147866877, normalized |
| Feedback blocks until asynchronous completion | 249, 301, 299 |

Two initial unsuccessful attempts are retained as diagnostic history, not
hidden qualification passes. First, native startup FFT planning exceeded a
30s test timeout; the fixture now bounds FFTW planning time without changing
WDSP numerical source. Second, `pscc` alone with TXA stopped populated fitted
display data but did not complete application (`info[15]=6`, `info[5]=0`,
`info[14]=0`). Source inspection showed native `SetTXAiqcStart/Swap/End` waits
for `xiqc` to acknowledge its ramp. The fixture now advances the real TXA DSP
using zero-mic `fexchange0` calls; output is discarded in memory. **The shared
production adapter must preserve DSP advancement while draining native
start/swap/end/reset work.** Display presence alone is not an applied calibration.

## Verification and reproduction

All checks were local and offline with respect to the radio. The port source
was fetched from its upstream Git repository; no G2 access was needed.

- New host/controller/decoder tests: **21 passed**. Native port is fake in
  these tests, explicitly separate from the real C fixture.
- Existing Bridge binary tests with native stubs: **377 passed, two ignored**.
- Browser suite including seven new tests: **699 passed**.
- Browser and revised contract TypeScript checks: passed.
- New Rust library `cargo clippy --lib -- -D warnings`: passed.
- Pinned source/header checker: passed; incompatible older seven-array
  `GetPSDisp` declaration rejected. This checker still correctly labels its
  own scope source/header-only.
- `git diff --check`: passed.

```sh
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --offline \
  --manifest-path update_manager/saturn-bridge/Cargo.toml

cd update_manager/remote-web
npm run typecheck
npm test -- --pool=forks --poolOptions.forks.singleFork
```

For the **known freshly built offline archive above**, from the repository root:

```sh
cc -std=c11 -O2 -Wall -Wextra \
  -I/tmp/saturn-puresignal-native.FYgKcV/build \
  update_manager/saturn-bridge/tests/puresignal-native-replay.c \
  /tmp/saturn-puresignal-native.FYgKcV/build/libwdsp.a \
  -lfftw3 -lm -lpthread \
  -o /tmp/saturn-puresignal-native.FYgKcV/native-replay
timeout 30s /tmp/saturn-puresignal-native.FYgKcV/native-replay
```

Do not substitute an unidentified installed archive merely because it exports
the same symbol names. The local scratch source/build archive was retained;
temporary-directory paths and x86 archive identity are not installation paths
or release instructions. The original M1 review zip remains unchanged.

## Next offline slice

1. Implement the real native port on the existing TXA owner, preserving the DSP
   handshake during fault/reset/unkey, proving bounded quiescence and preventing
   old asynchronous callbacks/curves from crossing the acquisition fence.
2. Bind P2's existing paired ingress and configuration/ordinary-RX restore
   transaction to the shared core through a bounded handoff. Exercise the real
   native controller pipeline, not only independent fake-core/native-C tests.
3. Verify the frozen exact RXC1 feedback topology/overload observation coverage,
   then attach the Direct decoder to the existing sole acquisition owner with
   transactional pair/DDC6 configuration and rollback tests. Leave its capability
   unverified until the evidence supports it.
4. Implement analyzer numerical fixtures and native snapshot/epoch consistency
   before wiring PS Monitor, Distortion Analyzer and AmpView. Add the telemetry
   probe protocol, authenticated read-only observers and panel lifecycle tests.

Installed G2 native-library identity and exact RXC1 feedback-path verification
remain open. Neither this source implementation nor the local native replay
justifies a new FPGA or kernel-driver change, a deployment, or RF testing.

[types]: ../../saturn-bridge/src/puresignal/mod.rs
[controller]: ../../saturn-bridge/src/puresignal/controller.rs
[adapters]: ../../saturn-bridge/src/puresignal/adapters.rs
[freshness]: ../../remote-web/src/puresignal/freshness.ts
[freshness-tests]: ../../remote-web/tests/puresignal-freshness.test.ts
[measurement-tests]: ../../remote-web/tests/puresignal-measurement-contract.test.ts
[native-replay]: ../../saturn-bridge/tests/puresignal-native-replay.c
