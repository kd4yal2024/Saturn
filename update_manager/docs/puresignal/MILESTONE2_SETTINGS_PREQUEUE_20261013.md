# Milestone 2 — managed PCM pre-queue settings rejection

2026-10-13. Offline-only correction in the isolated SilverForge worktree,
base `861f0fbc79e0e36c68a5c8ddc157e010b8be20df`. Uncommitted; no push,
deployment, installed host selection or G2 access.

## Outcome and bounded change

The managed client now completes an outstanding settings request when the
Bridge rejects it before queueing. It clears that request, displays the actual
Bridge reason and labels the result **before queueing**, rather than suggesting
configuration `0` was committed. Output remains inhibited; rejection does not
authorize re-arm, acknowledge a host fault or retry/restamp a command.

The behavioral changes are confined to
[ManagedPcmSession](../../remote-web/src/transport/managed-pcm.ts), its
[unit tests](../../remote-web/tests/managed-pcm.test.ts), and an opt-in rejection
case in the existing
[actual-page runner](../../remote-web/experiments/run_managed_pcm_ui.py).
The Bridge, native owner, template, startup selection, firmware, FPGA,
kernel driver and RXC1 polling implementation are unchanged by this correction.

## Association contract

The Bridge's existing
[configure_batch](../../saturn-bridge/src/tx_thread/ps_ingress.rs) allocates a
transaction ID before validation. Validation failures have no owner envelope
or queued receipt and report:

```text
tx_config:0,<id>,rejected,<BridgeReason>,0;
```

The client accepts this as an initial/terminal result only when there is a
request awaiting an initial reply **on the current control session**, the result
is `rejected` with an explicit reason and configuration `0`, and its ID is newer
than the initial-result watermark. It consumes exactly one oldest awaiting
request. This FIFO association uses the existing ordered control connection and
the Bridge's FIFO transaction-reply queue; IDs are not credentials.

Queued requests retain the existing ID/configuration matching for completion.
The watermark applies to *initial* results, not terminal results of already
queued transactions: older queued work may still finish after newer work.
Known duplicate terminal results are ignored; the watermark prevents IDs
evicted from the bounded duplicate cache from consuming a fresh request.
Unsolicited/unmatched results still fail closed, and results from an old socket
epoch are ignored without consuming new-session work. Disconnect resets all
association state. No wire protocol or authorization policy was changed.

## Reproduced failure and executed verification

Before rebuilding the bundle, the new unit cases failed on the original
client: **3 failed / 25 passed**. The actual-page replay then reproduced the
reported defect through the real Bridge parser/validation path:

```text
Bridge: tx_config:0,7,rejected,InvalidConfig,0;
UI:     fault: Unmatched settings completion
        Settings 6: applied; configuration 17 · 1 pending
```

That intentional failure, its raw socket frames and UI trace are preserved in
`settings-prequeue/before-fix-P2/` and `offline-native/prequeue-before-*`.
An earlier runner-launch failure selected system Python 3.10 against isolated
dependencies built for the supplied CAD-suite interpreter. Its transcript is
retained as a tooling error, not a Bridge/client or radio finding. The successful
runs use the matching explicit interpreter; no global packages were installed.

The rebuilt actual template/IIFE then passed the same real Bridge rejection
path on **both P2 loopback and Direct XDMA ordinary-file fixtures**, together
with the existing full native WDSP lifecycle replay. A gesture on the actual
drive control requests 6 W against the fixture's 5 W admission ceiling while
unkeyed. This is an invalid configuration proposal, **not RF output**.
No fabricated `tx_config` reply substitutes for the handler.

Each backend returned `tx_config:0,7,rejected,InvalidConfig,0;`, with no queued
receipt for that ID, and displayed:

```text
Settings 7: rejected (InvalidConfig); before queueing · 0 pending
```

The displayed result was identical after another **5.2 seconds**, and the
status remained faulted with `InvalidConfig`, not a missing-acknowledgment
timeout. JSON assertions, screenshots, raw frames and owner cleanup evidence
are in `settings-prequeue/final-{P2,DirectXdma}/`.

| Current replay | P2 fixture | Direct XDMA fixture |
| --- | ---: | ---: |
| Fresh Active generations before invalid settings case | 6 | 6 |
| Committed configurations (bootstrap + reconnect gain/EQ pairs) | 7 | 7 |
| Native microphone blocks | 390 | 390 |
| Fixture IQ writes | 1,625 | 1,633 |
| Deadline stops | 0 | 0 |
| Stop/native cleanup, empty queues, final IQC off | complete | complete |
| Parallel legacy TX output | none | none |

The invalid proposal did not increase the committed-configuration count. The
native owner had no primary, authority or cleanup fault. Its `rejected` counter
includes other cancelled/obsolete fixture work and is not an RF-loss measure.

Executed against this correction:

- **742 web tests**, including **28 managed-session cases**. The existing
  queued-then-rejected test remains. Six added cases cover pre-queue completion,
  FIFO/duplicate handling, unsolicited and non-prequeue results, duplicate-cache
  eviction, old sessions, and older queued completions.
- **48 library + 452 owner-adapter binary tests**, with two existing benchmarks
  ignored; no host implementation change was needed.
- Actual-page/real-WDSP replay: **1 passed covering both backends**, 32.03 seconds,
  including ordinary release/re-arm, keyed socket disconnect, reconnect and
  final cleanup before/after the added invalid-settings case.
- Typecheck, production bundle/asset budget, template/API seam, migration
  coverage and diff-whitespace checks passed.

The pinned x86 WDSP archive/source checks passed before the native replay:
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
This is not the installed G2 library identity. Historical standalone/default
suite transcripts remain in the inherited package; they were not rerun or
relabeled as new evidence for this client-only correction.

## Reproduction and remaining boundary

Build `update_manager/remote-web` with `npm run build`. Run the existing
`tests/run-puresignal-managed-pcm-ui.sh VERIFIED_X86_WDSP_BUILD_DIR`, with the
documented explicit template/bundle/browser/speech/results paths and
**`PS_UI_PREQUEUE_REJECTION=1`**. Results directories must be new. The runner
remains loopback-only and the native owners use UDP/regular files, not hardware.
The original browser case remains available without the opt-in environment
variable. Run the unit test using
`npm test -- --run tests/managed-pcm.test.ts`.

The same-origin replay proxy still mirrors production lane initialization; it
does **not** execute the actual Saturn Go TLS/proxy binary. That real-proxy
offline verification remains the next integration step, before any consideration
of host enablement. No new TLS claim or physical RF qualification is made here.

Managed tagged PCM remains opt-in/off by default, SATP retains the same single
owner, legacy continuous Opus remains separate, and banked Opus stays stopped.
No G2, firmware, FPGA, driver, polling or GoldMarsh worktree changes occurred.
The archive preserves the previous managed-UI package evidence; older reports
describe historical slices rather than this corrected result.
