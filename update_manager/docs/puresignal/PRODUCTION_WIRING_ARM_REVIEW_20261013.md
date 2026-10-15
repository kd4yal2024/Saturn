# Default-off production wiring — ARM review candidate

The requested host wiring is implemented and exercised offline. This is a
**review candidate, not a production release or live admission approval**.
The source is in the isolated `saturn-puresignal-production-review` worktree on
`cc09c61f91030d4c5a38d6c570a20c28b5d97e03`. No G2 connection or change was made
for this assignment. No deployment, restart, RF operation, commit/push, FPGA,
kernel-driver or RXC1 polling change occurred. GoldMarsh's branch, the original
SilverForge worktree and the accepted production-base ARM ZIP remain intact.

The package's `review.json` identifies the new source-content pin, ARM executable,
native archive, observer source/header pins, web bundle and evidence. A Git base
alone is not this uncommitted candidate's identity. Verify both checksum indexes.
The full source snapshot includes new files omitted by the tracked Git patch.

## Implemented paths

| Requirement | Source | Guarantee and limit |
|---|---|---|
| Startup selection | [main](../../saturn-bridge/src/main.rs), [selector](../../saturn-bridge/src/tx_thread/owner_selection.rs), [shared backend](../../saturn-bridge/src/shared_backend.rs), [constructor](../../saturn-bridge/src/tx_thread/production_owner.rs) | Default remains Legacy. Shared requires an explicit build feature/host selection and pinned inputs. It creates no parallel Legacy TX/native engine and never falls back. |
| P2 feedback | [existing session](../../saturn-bridge/src/p2/session.rs), [acquisition adapter](../../saturn-bridge/src/p2/managed_feedback.rs) | One receive socket/owner; DDC0 physical ADC1 RF and DDC1 returned TX reference, paired at 192 kHz/24 bits, shared LO, normal RX DDC retained. Command submission is not FPGA acknowledgment; delayed wire packets have no firmware-generation tag. |
| Direct feedback | [operational RX owner](../../saturn-bridge/src/xdma_rx.rs), [layout transaction](../../saturn-bridge/src/xdma_rx/managed_feedback.rs) | One C2H reader; PCB2 `1.31 / 0x53460004` gate. Hardware input0 is ADC1, input3 is returned TX reference. Paired `0x10001f` retains DDC6. Real RXC1 pair/backpressure/reset integrity remains unqualified. |
| Generation/age | [acquisition Link](../../saturn-bridge/src/tx_thread/feedback_owner.rs), [assembly](../../saturn-bridge/src/tx_thread/feedback_stream.rs), [owner loop](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs) | Scopes captured before read/receive. Known old scopes and partial histories retired; 1,024-pair direct deliveries retain oldest read time. These are host times, not exact RF capture times. |
| Matched cleanup | [full native wrapper](../../saturn-bridge/src/wdsp/ps_native.rs), [observer](../../saturn-bridge/src/wdsp/ps_observer.c), [build checks](../../saturn-bridge/build.rs) | Exact archive and every prepared private/public header pinned. IQC run/busy are atomically read as their actual C `long` width. Local real-native execution passed; ARM/G2 behavior and latency are not established. |
| Transports | Actual TCI listener/tagged PCM ingress; accepted SATP provenance/shared owner retained | Banked Opus remains stopped. Historical Opus regressions are not production approval. Chrome/actual Go proxy were not rerun for this slice; earlier accepted evidence remains historical. |

Direct acquisition uses serial-specific reader pause acknowledgments: an
already-started read finishes before layout writes/queue retirement. It saves,
verifies and restores rate, input, DDC0/1 phases and ADC attenuation. Only ADC1
TX attenuation changes; RX attenuation, ADC2 and other bits are preserved.
The existing disable/reset settling sequence is retained. DDC6 center changes
are applied before preparation completion, or while clean/off.

P2 receive-only bootstrap precedes its closed fence and client admission.
Paired-DDC/DUC-specific attenuation writes belong to acquisition and grant no
RF/HP/IQ authority. Untagged Legacy writers remain refused. Keyed heartbeats
require generation permits; off heartbeats hold a closed-state writer guard.

## Stop and re-arm contract

```text
release/disconnect/fault
  -> atomic output inhibition + retirement of feedback request
  -> permitted output drain + queued mic/IQ discard
  -> P2 complete unkey burst OR direct shutdown/control readback
  -> acquisition restoration and matching host completion
  -> native cleanup with DSP advancing, then full-channel recreation
  -> clean configuration publication
  -> fresh authorized action for a new output generation
```

RF shutdown progresses before acquisition restoration. Backend completion is
latched: no repeated TX FIFO writes or extra P2 burst commands while RX restores.
Native cleanup waits for the matching complete receipt. Layout I/O/readback
failure blocks re-arm; best-effort restoration is not certified completion.
An absent reader can time out restoration: that is failure, not successful unkey
completion. Uncertain native storage is retained rather than unsafely freed.

P2 evidence is local command-burst completion; direct evidence is control readback.
**Neither proves physical RF-off.** Slow telemetry may supplement positive
hazard handling, but is not fast overload protection. No clear ADC/DDC bracket
is invented or polling rate increased. Unclipped measurement confidence remains
unavailable without valid interval coverage.

## Default-off policy and limitations

Build feature: `managed-puresignal`. Host selector:
`SATURN_BRIDGE_TX_OWNER=shared`. Absent selection is Legacy; unsupported/unknown
selection fails without fallback. Installed service/env files were not changed.
A browser URL cannot select the host owner or create RF permission.

Shared requires explicit hardware peak, requested/loop delay, feedback freshness/
settling, mic age, source stall, TX duration, authority/cleanup timeouts, maximum
watts, feedback attenuation and a separate startup timeout. RF permission and
reviewed-topology attestation are independent and default false. Startup/FFT
planning has its own budget, not an invented RF response bound. RT scheduling
is bypassed only by test builds; no G2 timing claim follows.

Strict adaptation admission remains default. Existing interfaces do not provide
complete interval overload brackets, so strict admission is not advertised as a
usable, qualified live correction loop. Operational admission requires separately
supplied active-reference fraction, RF RMS floor and component limit. Fixture
limits are **not production defaults or reviewed thresholds**. Review these
path-specific values before any hardware admission.

Feedback geometry is fixed. Changed ADC, antenna, receive attenuation or sample
rate requests receive explicit transaction rejection; browser tracking includes
these results. Idempotent requests are permitted. DDS/frequency and supported TX
settings use ordered lifecycle transactions. Some advanced RX/local-monitor
controls and full normal performance/telemetry parity remain unavailable.
Unsupported controls are reported. Shared readiness/performance records say
`review-only`, `rf_safe:null`, `rf_qualified:false`: they cannot retain a stale
Legacy `ready` result or masquerade as a normally installable release.

## Executed evidence — local x86/WSL, not ARM

| Check | Result |
|---|---|
| Managed stub suite | 551 binary + 48 library + 14 harness tests passed; two binary benchmarks ignored. |
| Default Legacy stub suite | 464 binary + 24 library + 14 harness tests passed; two binary benchmarks ignored. |
| Existing real-WDSP combined regressions | Seven passed; historical transport cases retained without managed-Opus approval. |
| Matched production constructor/combined path | Two explicit serial tests passed, each covering P2 and Direct fixtures. Default-off startup, single native ownership, real listener/tagged PCM, actual pre-queue rejection, two corrected generations, release/restoration, fresh re-arm, stale-generation fault refusal and completed shutdown. |
| Web | 812 tests passed; typecheck, build and asset-budget checks passed. |
| ARM public ABI comparison | Nonempty compiler/header hashes, actual quoted command, `exit_status=0`; syntax-only, no ARM execution. |
| ARM candidate | AArch64/Cortex-A72 cross-build against the previously verified Trixie sysroot/matched WDSP archive. ELF/build evidence and hashes in bundle. Not executed or installed. |

Acquisition tests exercise pause boundaries, partial-frame age, buffered-tail
retirement, demultiplexing, stale completions, release during layout write,
restore-after-stop ordering, I/O/readback mismatch, attenuation preservation and
P2 queued-datagram retirement. The real-time dequeue test accepts a genuine
observation arriving after initial tick while still rejecting a truly future
observation; no sample timestamp is rewritten.

Integration found and corrected P2 managed sends incorrectly using the untagged
helper, an omitted controller key-edge generation increment, and the pre-dequeue
clock race. Failed development logs remain alongside final evidence. The first
cold-planning test lacked the fixture FFTW planning cap and timed out. A toy
client without TCP_NODELAY exercised bounded-queue failure; the final local
client sets TCP_NODELAY. No production TCP setting changed. This is **not a
sustained uplink/WAN or G2 latency qualification**.

## Native identity and evidence cleanup

- WDSP source: `b02d5bac675dd2f33ec2bab2b339f79a597c47dd`, version 210.
- Public header: `3dde5a9bfafe7e87f76684eeaab7da22fd5d421c08a8a725b0e2ec3cbd5416af`.
- Local replay archive: `8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
- ARM archive: `f5f21c7cb76df1add17647c15a700f564a7f045e930e84be870333fa6e74973e`.
- Observer uses the same target and prepared private headers; source and header
  pins are independently recorded in `review.json`.

The old empty `puresignal-production-arm-native-abi-verified.txt` was not
verification. It remains historical, insufficient evidence. The replacement
`puresignal-production-wiring-arm-abi-command-status.txt` records the actual
successful syntax check using the included runner. Public `wdsp.h` and private
`comm.h` use separate translation units because their declarations/macros
conflict. Syntax checking is not installed-runtime qualification.

The candidate statically embeds the pinned WDSP archive; it is not the G2's
installed executable. Earlier installed version/source labels did not recover
the installed static-library hash or exercise its PS ABI. The new identity is
for the **candidate**, not live qualification.

## Review and rollback handoff

Review source/package before a separately authorized controlled G2 test. That
test must establish actual RXC1 routes, attenuation/levels, delay, pair continuity,
ARM behavior, stop/readback/restoration under faults, CPU/queue timing, disconnect
behavior, physical RF-off and spectral correction. Synthetic nonlinear replay
does not qualify RF performance. Old clear telemetry cannot certify a newer IMD
capture. No FPGA, driver or faster polling work is justified by this slice.

Rollback is host-only: request controlled stop and establish available stop
evidence before stopping acquisition; restore the preserved Legacy executable/
release and remove explicit Shared selection. Confirm exactly one acquisition
and TX/native owner before resuming the existing qualified Legacy operation.
Firmware stays `1.31.002 / 0x53460004`. No rollback/activation is performed by
build/packaging scripts. The review bundle is not an installer payload.
