# PureSignal production-base and ARM build review — 2026-10-13

## Outcome and boundary

This is **build preparation and offline integration evidence, not a deployable
managed-PureSignal release**. The accepted offline changes have been reconciled
with the G2's deployed host base in a separate worktree. A matched Cortex-A72
WDSP archive and AArch64 Bridge build are recorded independently of the
installed binary. Production-owner selection and real acquisition wiring remain
unfinished; do not activate this candidate on the G2.

The original SilverForge worktree and GoldMarsh branch are preserved. No commit,
push, installation, activation, restart, RF operation, FPGA update, kernel-driver
change or RXC1 polling change was performed. SSH was used only for identity
inspection and copying development headers/link libraries to a local sysroot.
Compilation and replay take place on WSL, not on the G2.

The intended result remains shared-owner correction on P2 and Direct XDMA,
trustworthy measurements and three dockable panels. Tagged PCM is the first
managed browser transport, SATP is retained, and the four-packet banked-Opus
proposal stays stopped. Historical Opus fixture tests are regression evidence,
not approval to select that proposal.

## Source and installed identity

| Item | Evidence / qualification |
|---|---|
| Accepted predecessor | `Saturn_PureSignal_Milestone2_Real_TLS_Proxy_Verified_20261013.zip`; SHA256 `c6503f40a278973a4116a272946390ad7f9b257f92f4b18b20936963081a4f48`; its entire checksum manifest verified before source recovery. |
| Original review base | `861f0fbc79e0e36c68a5c8ddc157e010b8be20df` on `silverforge/puresignal-milestone1`; preserved. |
| New reconciliation base | `cc09c61f91030d4c5a38d6c570a20c28b5d97e03`, the deployed G2 release; separate detached worktree, not a rewrite of GoldMarsh's branch. |
| Installed executable | SHA256 `21bed70518acdf8049ff9aa3f3011af84566d3b1301415deff508f78080217b5`; running `/proc/2138187/exe` and active-release binary agreed at observation time. |
| Host ABI | AArch64 CM4/Cortex-A72, Debian 13 Trixie; libc `2.41-12+rpt1+deb13u4`; FFTW `3.3.10-2+b1`. |
| Installed WDSP labels | Published `wdsp2-2.10`, commit `b02d5bac675dd2f33ec2bab2b339f79a597c47dd`, RX DSP size 256. These are executable provenance labels, not a uniquely recovered static-library hash or an independently exercised installed PS ABI. |
| Firmware target | Operator-qualified `1.31.002 / 0x53460004` remains intended and unchanged. This read-only check obtained published major/minor `1.31`; it did not independently reread the exact FPGA subversion/build ID. |
| Existing TX flags | Ready telemetry reported normal TX qualified/enabled/requested. Those flags are **not managed-PureSignal qualification**, and were not changed. |

GoldMarsh's newer accepted-socket handling, bounded command draining and tests
were retained when resolving the TCI and TX-thread overlaps. The merged test
suite includes these newer tests rather than reverting to the older review base.

The package's `SOURCE_SHA256SUMS` pins every included source snapshot file; its
SHA256 is the candidate source pin passed to `SATURN_BRIDGE_SOURCE_PIN`. A Git
base alone does not identify this uncommitted candidate. `review.json` records
both the base and content pin. The bundle is for review, not a release manifest
accepted by Saturn's installer.

## Native contract and ARM candidate

The ARM archive is rebuilt from the prepared C/H files recovered from the
verified predecessor. It uses the same upstream WDSP pin and public header:

- WDSP commit: `b02d5bac675dd2f33ec2bab2b339f79a597c47dd` (version 210).
- Public header SHA256: `3dde5a9bfafe7e87f76684eeaab7da22fd5d421c08a8a725b0e2ec3cbd5416af`.
- Reviewed x86 archive SHA256: `8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
- Newly built ARM archive SHA256: `f5f21c7cb76df1add17647c15a700f564a7f045e930e84be870333fa6e74973e`.

`native/native-build.json` records each source hash, compiler and command. The
cross-build uses Clang 14, Rust 1.98.1 and an explicitly copied G2 ABI sysroot;
it does not install a cross-toolchain or modify G2 packages. The candidate
binary's hash and ELF dependencies are recorded separately. The native source
snapshot is included to make the WDSP input auditable.

The new `--native-build-info` query runs before FFTW wisdom, sockets and radio
devices, and reports the built input identities and the unfinished wiring state.
**Do not invoke this new flag on the installed older executable:** it has not
been shown to recognize it. The ARM candidate has not been executed on the G2.

The reviewed PS contract uses 1,024 paired complex samples per `pscc` call at
192 kHz, interleaved doubles with signed 24-bit input normalized by `2^23`.
`GetPSDisp` capacities are 4,096 measured points and 512 correction points per
array. Native TX delay is the value returned by `SetPSTXDelay`, not an assumed
requested delay. The existing full wrapper owns mic DSP, PS processing and
cooperative cleanup together; it is still an offline constructor.

Candidate compilation does not establish equivalence to every feature in the
deployed native build. In particular, optional neural/noise-processing features
and installed private IQC-state ABI must not be inferred from version labels.

## Actual production path versus remaining work

```text
Current default:
  TCI/SATP -> legacy TX scheduler -> existing P2 / Direct XDMA output
  P2 RX owner -> decoded DDC0 feedback -> legacy TX command queue
  Direct XDMA sole C2H reader -> DDC6-only parser -> normal RX

Required managed path (not selected at startup):
  authenticated ingress -> shared lifecycle + one full native owner
  sole existing RX owner -> generation-tagged paired feedback -> same owner
  shared lifecycle -> fenced backend output -> matching stop receipt -> cleanup
```

| Integration point | Current source evidence | Remaining production contract |
|---|---|---|
| Startup owner selection | [Selection](../../saturn-bridge/src/tx_thread/owner_selection.rs) defaults to Legacy; [spawn](../../saturn-bridge/src/tx_thread.rs) passes that default. The SharedReview branch refuses construction. | Explicit default-off selection, validated before admitting clients; no legacy fallback or parallel native owner. |
| TCI/SATP producers | [Shared ingress](../../saturn-bridge/src/tx_thread/ps_ingress.rs), ordered transactions, tagged PCM and SATP are exercised through offline owners. | Install ingress atomically before accept/receive activity; migrate real startup/event paths, not a browser-only switch. |
| P2 acquisition | [RX thread](../../saturn-bridge/src/rx_thread.rs) timestamps legacy feedback after decode and sends an untagged legacy command. [Session](../../saturn-bridge/src/p2/session.rs) can configure paired DDC0/1. | Capture observation and generation at acquisition, preserve raw-format validation and continuity, retire old queued UDP packets on layout/generation changes. |
| Direct XDMA acquisition | [Operational session/parser](../../saturn-bridge/src/xdma_rx.rs) supports normal DDC6. [TX radio](../../saturn-bridge/src/xdma_tx_radio.rs) still refuses PS feedback configuration. | Same acquisition owner must transactionally save/apply/read back/restore the paired layout, demultiplex PS and DDC6, retire DMA buffers/partial frames across changes and report loss. No second C2H reader. |
| Feedback formats | [Adapters/plan](../../saturn-bridge/src/puresignal/adapters.rs) validate P2 DDC0 and proposed XDMA `0x1f` / `0x10001f` frames in replay. | Exact unchanged-RXC1 topology and pair-integrity admission remain unverified. A replay plan is not permission for live BAR writes. |
| Native cleanup state | [Full native wrapper](../../saturn-bridge/src/wdsp/ps_native.rs) uses a pinned offline observer for IQC run/busy and SATP history. | Match a production observer/shim to the exact candidate native source, and connect it to the single production owner. Do not treat public PS display/status fields as private cleanup atomics. |
| Feedback limits | The explicitly selected draft policy has replay thresholds and distinguishes adaptation from unclipped-capture qualification. | Path-specific limits remain unqualified; noisy/mistuned/unrelated feedback, speech pauses and envelope minima need coverage before production defaults are set. |

These are host integration tasks. None establishes a reason to change FPGA,
driver or polling. The exact RXC1 layout evidence and installed-native identity
remain distinct from the working offline controller.

## Protection, measurements and stop guarantees

Retain immediate output fencing, generation-specific cancellation and fault
latching. Release/disconnect/fault must retire queued commands, mic/IQ and
feedback; only a fresh authorized TX action after matching stop completion and
native cleanup may re-arm. Configuration completion must not renew old arm
authority. Periodic lifecycle checks must service missing-feedback deadlines
without needing another packet, even under empty or busy queues.

P2 stop receipts establish complete command-burst submission. Direct XDMA stop
receipts establish shutdown and matching control readback. Neither proves
physical RF-off. Already-permitted writes remain in flight until drained; a
closed fence or early `Unkeyed` event does not replace completion. The x86
replay exercises these distinctions, but it does not measure CM4 scheduling,
DMA-write, native recreation or RF shutdown latency.

Missing periodic overload telemetry must never be replaced with fabricated
clear brackets. A measurement is proven unclipped only when valid, unsaturated
counter evidence covers its capture interval. Slow RXC1 telemetry may supplement
protection; it is not fast overload protection. Bridge-ingress timestamps do
not establish exact RF-output age. Audio codec spectrum is not RF IMD.

## Reproduction and evidence

Evidence filenames and hashes are listed in `review.json` and covered by
the bundle's `SHA256SUMS`. The following checks were run locally against the
reconciled source, not merely copied from predecessor claims:

1. Stub-native owner-adapter suite (48 library, 537 binary and 14 browser-fixture
   tests; two benchmark cases ignored).
2. Seven combined real-WDSP owner tests using actual loopback/file-backed owner
   adapters, TCI sockets and SATP UDP. These are not physical radio devices.
3. Strict/proposal/lifecycle controller suites and real-native P2/Direct replay,
   including watchdog, unkey-during-fit and display-storage canaries.
4. Default-feature regression suite (24 library, 464 binary and 14
   browser-fixture tests; two binary tests ignored) and read-only build-info
   checks, recorded separately from the experimental features. A stub build
   with a real native directory still reports `stub` identities; an invalid
   source-pin string reports `unverified` rather than a fabricated identity.
5. ARM compile/link and static ELF dependency/version inspection. Not an ARM
   execution/performance or RF test.

The old browser/proxy exercise was not repeated or relabeled as new hardware
evidence. Native modeled correction is a positive control, not measured RF IMD.
Simulation timing in replay logs is not a measured G2 response bound.

## Rollback and controlled-review prerequisites

The active release remains `cc09c61f91030d4c5a38d6c570a20c28b5d97e03`.
There is nothing to roll back from this work: no candidate has been installed.
Retain that release, its manifest, service configuration and existing normal-TX
authorization before any separately approved future test.

A future host-only candidate must be built and staged as an **inactive** release
with the regular validated release pipeline. Before activation, obtain explicit
approval for the interruption/test scope, release/unkey all clients and verify
existing safe-stop conditions. Rollback uses the normal validated activation
manager to return to the retained release; do not replace binaries beneath a
running process or flash the firmware. Preserve old and new logs and identities.

Do not present this bundle for RF testing until the remaining startup,
acquisition and native-state wiring is complete, offline-tested on both backends,
and reviewed. Live feedback limits and physical stop timing require a separate
controlled hardware qualification. The existing normal-TX qualification does
not authorize silently selecting experimental correction.
