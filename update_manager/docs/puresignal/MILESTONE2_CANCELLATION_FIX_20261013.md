# PureSignal milestone 2 — coherent cancellation correction

2026-10-13. Offline SilverForge worktree, base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`. No commit or push.

## Outcome and scope

The fence/mailbox race now has a deterministic passing cancellation regression,
not merely a passing characterization of the defect. Ordinary release and
matching-generation cancellation inhibit output immediately, complete the stop
receipt/native-cleanup sequence, and permit settings and a fresh authorized
re-arm without a false `TxAuthority` latch. Genuine authorization failures
remain latched; release does not acknowledge them.

Real Chrome tagged-PCM lifecycle replay passed on both existing backend
fixtures. This is an offline correction only: installed production still
selects Legacy, the shared path remains disabled by default, and no G2, RF,
firmware, FPGA, kernel driver, polling or deployment work was performed.
GoldMarsh's worktree was not edited. Installed firmware 1.31.002 /
`0x53460004` was neither inspected nor changed during this task.

The four-packet banked-Opus proposal is stopped. Its original extra-packet,
truncation and cancellation failure evidence is retained unchanged in the prior
archive and copied as historical evidence into this review package. Neither
discarding flush packets nor redesigning managed Opus is part of this fix.
Legacy continuous Opus remains unchanged; SATP remains supported by the prior
offline path. Tagged PCM is the next managed-browser integration path, not a
newly enabled production selection.

## Source-linked correction

```text
release / matching-generation cancellation
 -> atomic fence state: CLOSED + new revision + new cancellation ticket
    -> final output permit denied immediately; permitted writes stay in flight
    -> lifecycle ReleaseHandle reads the SAME ticket
    -> settings epoch reads the SAME ticket; no delayed invalidation publication
 -> periodic owner service: release first, health check, release recheck
 -> actual backend stop receipt after queue discard / permitted-write drain
 -> one native owner cooperatively cleans up
 -> published clean state + completed settings
 -> only a fresh authenticated action can authorize a new generation
```

- [OutputFence](../../saturn-bridge/src/tx_owner_stop.rs) packs closed, revision
  and cancellation into one atomic word: 1 closed bit, 32 revision bits and
  31 cancellation-ticket bits. Exhaustion is terminal and inhibits reopening;
  host/wire envelope fields remain `u64`. Generic `close()` invalidates output
  without inventing a release ticket. Generation-specific cancellation uses a
  conditional CAS and does nothing for an old or already-closed generation.
- [ReleaseMailbox and lifecycle](../../saturn-bridge/src/puresignal/tx_lifecycle.rs)
  allow the actual owner to supply that same atomic state; standalone simulated
  authorities retain their counter mailbox. Release still takes priority, and
  a second check after successful health polling catches release arriving
  during the health check before key/native progress.
- [Owner health/re-arm](../../saturn-bridge/src/tx_owner_stop/rearm.rs) recognizes
  a changed cancellation ticket only for its current active output generation.
  Credential revocation or unexplained closure without that ticket remains an
  authorization fault. Old cancellation cannot inhibit a fresh generation.
- [P2 key gate](../../saturn-bridge/src/p2/session.rs) and
  [Direct XDMA key gate](../../saturn-bridge/src/xdma_tx_radio.rs) distinguish
  permission refusal before I/O from writer/backend failure. Only the former,
  with matching cancellation evidence, is an ordinary cancelled attempt.
  Genuine backend errors are not broadly suppressed. A raced conditional
  reopen refusal similarly requires a newly published release ticket.
- [Cancel/settings epochs](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs)
  and [transaction validation](../../saturn-bridge/src/tx_thread/ps_ingress.rs)
  now use the cancellation ticket plus an owner-local mutation epoch. Cancelling
  never publishes a separate late settings increment. This prevents a paused
  cancellation caller from invalidating newer settings after completed cleanup.
  Existing ordered completion/rejection, generation checks and fresh-arm
  requirements remain intact; stale commands are never blindly restamped.

## Deterministic regression evidence

[Actual owner tests](../../saturn-bridge/src/tx_owner_stop/tests.rs),
[scheduled tests](../../saturn-bridge/src/tx_owner_stop/scheduled.rs), and
[lifecycle tests](../../saturn-bridge/src/puresignal/tx_lifecycle/tests.rs) cover:

1. Pause cancellation after its atomic commit; run the owner while the caller
   is suspended. Require ordinary cancellation, no new output, complete stop
   receipt and native cleanup, then settings and a healthy fresh re-arm.
2. The same boundary for generation-specific cancellation; after re-arm, replay
   the old stamp and require no revision/ticket change or new-generation stop.
3. Hold an already-permitted writer. Cancellation cannot wait on it, new permits
   are refused, and completed stop still waits for that writer to drain.
4. Cancel immediately before backend key permission on **both** actual fixture
   adapters. Require no new key/IQ write and normal stop completion.
5. Release during health polling or conditional arm refusal. Require no false
   authority fault, no key completion, and completed cleanup; a real authority
   error remains latched even when release also occurs.
6. Complete cleanup and apply new settings while the cancellation caller stays
   paused; leave another fresh settings transaction queued, resume cancellation,
   and require that transaction to apply. Cancelled old settings explicitly
   reject. A fresh authorized action can subsequently arm.
7. Genuine identity revocation and generic unexplained fence closure latch
   `TxAuthority`, surviving subsequent release/cleanup and blocking settings.
8. Revision/ticket exhaustion and overlapping close/reopen remain fail-closed.

The original failing deterministic test and original Direct browser failure
are preserved in `offline-native/browser-cancellation-reproducer.txt`,
`offline-native/browser-actual-native-failure.txt`, and
`browser-experiment/stale-delivery-failed-DirectXdma/`. Their historical passing
characterization must not be mistaken for execution of the corrected test.

## Real-browser lifecycle verification

[PCM-only runner](../../saturn-bridge/tests/run-puresignal-cancellation-browser.sh)
invokes the ignored `browser_cancellation_pcm_lifecycle_both_backends` native
test. [Browser runner](../../remote-web/experiments/run_ps_browser.py) uses
`--lifecycle-pcm-only`; it does not run the banked/continuous comparison.

Actual Linux headless Chrome 153.0.8010.52, recorded MediaStream, real
AudioWorklet, tagged PCM, actual split WebSockets, actual parser/authorization,
shared scheduling loop, full pinned WDSP, and existing output-owner adapters:

| Final audit, per backend | P2 loopback | Direct XDMA regular files |
| --- | ---: | ---: |
| New control/media connection pairs | 3 | 3 |
| Fresh Active output generations | 6 | 6 |
| Committed configurations (bootstrap + separate settings) | 7 | 7 |
| Rendered microphone blocks | 564 | 564 |
| Actual fixture IQ packet writes | 2,542 | 2,503 |
| Deadline-triggered stops | 0 | 0 |
| Stop/native cleanup | complete | complete |
| Final IQ correction flags | off | off |
| Parallel legacy output | none | none |

Each connection exercises release/re-arm. The second disconnects while keyed
without sending an ordinary `trx:false` first. The third reconnects, applies
settings and authorizes fresh TX. Old-generation and prior-session packet
injections are rejected without faulting current output. Assertions require
no primary, authority or cleanup fault at every owner turn, empty final queues,
and at most one native exchange per turn. Browser records show zero delayed
output after release in each case. Additional rejected queue work is expected
at deliberate cancellation/stale-injection boundaries, not a loss counter.

Current artifacts are under `cancellation-fix/audit-{P2,DirectXdma}/`; the earlier
post-fix browser pass is separately retained under `final-*`. Metadata includes
browser identity, input/source hashes, frames, settings replies, owner states,
and final result. Local user speech recording stays local; inclusion of its
short transformed clip makes no public redistribution-rights claim.

## Executed checks and limits

- Owner-adapter stub suite: **48 library + 451 binary tests passed**, 2 existing
  benchmark tests ignored. The browser replay is separately run with real native
  linkage and its explicit ignored-test selection.
- Default/stub checkout: **24 library + 379 binary tests passed**, 2 ignored.
- Combined actual-owner/full-WDSP regression filter: **7 passed**, including
  authenticated SATP provenance and shared native cleanup.
- PCM-only real-browser/full-native lifecycle: **1 test passed covering both
  backend fixtures**, final audit 24.06 seconds; earlier pass 25.51 seconds.
- Standalone policy/lifecycle suites: **24 / 32 / 48 passed**; pinned native
  fitting, controller, watchdog and cooperative-reset replays passed.
- Web unit suite: **714 passed**; project typecheck and strict experiment
  typecheck passed. Shell syntax and `git diff --check` passed.

The x86 offline WDSP archive remains
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
This is not verification of the library installed on G2. Test transcripts are
`offline-native/cancellation-*`; the package manifest freezes source/evidence.

P2 command completion is still only local command-burst evidence; Direct
fixture shutdown/readback is not physical RF-off evidence. No hardware response
bound, live feedback threshold, RF IMD, linearity, on-radio correction or
physical-microphone qualification is claimed. These checks establish the
offline cancellation lifecycle, not PureSignal RF performance.

Next review: the selectable managed tagged-PCM host integration, retaining SATP
and the same sole owner. Managed Opus requires a separate design decision and
does not block PCM progress. No additional FPGA, driver or polling work follows
from this correction.
