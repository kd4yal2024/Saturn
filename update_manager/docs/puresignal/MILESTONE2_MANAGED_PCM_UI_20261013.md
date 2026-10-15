# Milestone 2 — selectable managed tagged-PCM Saturn Remote UI

2026-10-13. Isolated SilverForge worktree, base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`. Uncommitted, no push or deployment.

## Outcome

The **actual Saturn Remote template and IIFE** now contain a selectable managed
tagged-PCM path, with settings transaction results and shared-owner readiness.
The full-page real-Chrome/native replay passes on both backend fixtures: six
Active generations each, separate settings messages, release/re-arm, deliberate
socket loss while keyed, reconnect and complete stop/native cleanup.

Selection remains **Legacy by default**. The new per-page selector lives in
Settings → Transmit → PureSignal; change it only while disconnected. It is not
persisted, restored from preferences or activated through a URL. Browser
selection cannot enable the host owner: installed Bridge startup still chooses
Legacy and its shared-review startup branch remains inhibited pending review.
The offline fixture is the only tested selectable shared host here.

SATP still uses the same lifecycle/native owner, without a browser microphone
or a parallel legacy TX consumer. Banked Opus stays stopped; the new integration
does **not** import that reference client. Existing legacy continuous Opus and
its tests remain intact. No G2 access, RF, firmware/image change, FPGA, kernel
driver or RXC1 polling change occurred. GoldMarsh's worktree was not edited.

## Integrated source path

```text
actual Saturn Remote UI (off-by-default selector, PTT/MOX, settings)
 -> ManagedPcmSession: contract / operator / cleanup / transaction gates
 -> new getUserMedia track + AudioContext + AudioWorklet per generation
 -> 512 samples, 48 kHz mono PCM16 + PSG1 token + generation-local sequence
 -> actual split adapter OR existing single socket (no implicit fallback)
 -> actual Bridge parser + operator credential + bounded shared ingress
 -> existing sole OwnerLoop + full WDSP native owner
 -> P2 output adapter OR Direct XDMA output adapter
 -> stop receipt / cooperative native cleanup / clean owner publication
 -> only another fresh authorized UI action can re-arm
```

[Actual UI](../../templates/saturn-remote-next.html),
[bundle seam](../../remote-web/src/remote-next-entry.ts),
[managed controller](../../remote-web/src/transport/managed-pcm.ts),
[fresh microphone capture](../../remote-web/src/audio/managed-pcm-capture.ts),
[shared ingress/status](../../saturn-bridge/src/tx_thread/ps_ingress.rs),
[PCM-only media profile](../../saturn-bridge/src/tx_thread/ps_media.rs), and
[existing native scheduling loop](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs).

## Contracts and visible state

| Contract | Required behavior |
| --- | --- |
| `saturn_tx_media_caps:psg1,pcm,owner1,transactions1;` | Require all four capability names. Old bank4-only or legacy capability sets are explicitly unsupported; no legacy/Opus fallback inside the managed session. |
| `tx_codec_caps:0,pcm;` / `tx_codec_accept:0,pcm;` | Managed advertising is PCM-only, independent of legacy Opus overrides. A different accepted codec inhibits output. The separate Bridge profile rejects Opus before creating decoder history. |
| `saturn_tx_generation:psg1,<token-or-off>,<serial>;` | Token and sequence belong to one authorized output generation. Ignore obsolete serials; invalidate capture before release/disconnect cleanup. Late capture acquisition cannot acquire new authority. |
| `tx_config:0,<id>,queued,<config>;` followed by `applied` or `rejected,<reason>` | Track separate messages in order, bounded pending results and deadlines. Display terminal result/reason. Applied result alone cannot replace matching committed owner state or refresh stale arm authority. |
| `saturn_tx_owner:shared,<serial>,<phase>,<stop-id>,<config>,<tci-or-satp>,<clean>,<fault>;` | Serial/stop ID are counters, not timestamps. `clean` requires installed config, ReadyOff, completed native cleanup and no latched fault. After release, require a newer completed stop ID, not an old clean snapshot. |

UI states explicitly distinguish disabled, disconnected, negotiating,
unsupported, unavailable operator, ready, arming, capture streaming, release
progress and faults. Missing capabilities/completion or mic permission/worklet
failures cannot silently choose a legacy producer. PTT/MOX readiness respects
the managed gates; capture streaming is **not** translated into keyed/on-air.
The UI uses the owner's AwaitingKey/Active/RX phase for TX presentation.

Native correction/feedback/calibration measurement panels show **unavailable /
unqualified** in this managed slice rather than default zero/bypassed readings
masquerading as live telemetry. Generated/two-tone source admission is also
unavailable here. This does not remove the existing legacy generators.

Managed reconnect reads committed host TX settings rather than replaying stale
browser TX preferences/VFO state. RX restoration remains separate. Explicit
subsequent TX-setting gestures still generate transactions and visible results.
Changing selection invalidates unresolved legacy mic requests; legacy mic and
late encoder callbacks cannot write alongside the managed producer.

## Cancellation publication finding closed during integration

The full-page replay exposed a remaining publication boundary: the owner could
still have an old `OutputStamp` when atomic cancellation had already closed the
fence. A final snapshot could combine that stamp with the new revision and
advertise a fresh token while retiring the generation. The client then saw a
late offer after local release as unsolicited.

[Publication eligibility](../../saturn-bridge/src/tx_owner_stop/rearm.rs) now
validates the stamp/grant/fence before it can enter an ingress snapshot. Keying
retains its existing cancellation classification separately. The deterministic
paused-cancellation tests assert **no publication output** before release service,
and the actual UI replay exercises the combined path. The client also inhibits
an already-in-transit offer during local release without granting authority or
inventing a fault. A genuinely unsolicited offer remains a fault.

Separate regressions require a **newer completed stop** after release and a
committed owner config matching the applied settings result. A stale clean
snapshot cannot turn either boundary into readiness. Existing authorization
fault latches and capture-age limits remain intact; nothing was silently
acknowledged, restamped or given a longer freshness deadline.

## Executed evidence

Final full-page run, `managed-pcm-ui/final-{P2,DirectXdma}/`:

| Per backend | P2 loopback | Direct XDMA regular files |
| --- | ---: | ---: |
| Actual new control/media pairs | 3 | 3 |
| Fresh Active generations | 6 | 6 |
| Configurations: bootstrap + two actual UI messages per connection | 7 | 7 |
| Microphone blocks rendered | 387 | 395 |
| Actual fixture IQ packet writes | 1,605 | 1,609 |
| Deadline stops | 0 | 0 |
| Complete stop/native cleanup, empty queues, final IQC off | yes | yes |
| Parallel legacy TX output | none | none |

The actual gain control receives both its input and change gestures; EQ is a
separate button message. No direct reference-controller call substitutes for
PTT/MOX. The second connection is interrupted by the fixture while Active,
then the UI cancels retry, reconnects, applies settings and authorizes fresh TX.
Assertions require six non-vacuous keyed generations, no primary/authority/
cleanup faults, and at most one native exchange per owner turn.

Real Linux headless Chrome **153.0.8010.52**, actual `getUserMedia` and AudioWorklet,
Chrome fake microphone backed by a local PCM16 WAV. Both float analyzer input
and PCM16 conversion/hash are retained. This is **not physical microphone
testing**. The short local user recording was not uploaded and carries no
assertion of public redistribution rights.

The same-origin test proxy mirrors the existing
[remote TLS lane initialization](../../rust-server/src/remote_tls.rs): public
`/saturn/control`/`media` dial upstream `/control`/`media`, then initialize
`session_lane` before browser traffic. The actual template, split adapter and
Bridge handler run; the production TLS/proxy executable itself is **not** run
by this fixture. Initial fixture mistakes (hidden-control fill, missing proxy
initialization) are not radio findings. Relevant failed integration runs are
retained separately, including the false late-offer client fault and earlier
Stale stops; passing final runs do not imply all possible timing faults are
exhaustively excluded.

Executed checks:

- **736 web tests**, including 22 managed-controller tests and unchanged legacy
  SATP/release/Opus coverage. Existing extracted-template harnesses explicitly
  select Legacy; the new path has its own behavioral and actual-page coverage.
- **48 library + 452 owner-adapter binary tests**; 2 existing benchmarks ignored.
- Default/stub: **24 library + 379 binary tests**; 2 existing benchmarks ignored.
- **7 combined actual-owner/full-WDSP regressions**, including actual
  authenticated SATP and native provenance/cleanup; standalone suites **24 / 32 /
  48** and real native fitting/controller/watchdog/reset replay passed.
- Actual-page/native test: **1 passed covering both backends**, 21.54 seconds.
  Earlier post-fix and audited actual-page passes are retained separately.
- Project typecheck, bundled build/asset budget, template/API seam, settings
  migration coverage, shell syntax, Rust formatting and diff whitespace passed.

Current transcripts are `offline-native/managed-pcm-*`. Source, raw socket
frames, UI state traces, actual page/bundle hashes, browser identity, screenshots,
original failures and earlier frozen milestone evidence are in the package.
The offline x86 WDSP archive remains pinned to
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`;
it is not the installed G2 library identity and must not be installed there.

## Review boundary / missing production qualification

Ready for **offline integrated-path review**, not production enablement or RF
qualification. Host startup selection is still inhibited/default Legacy; no
hardware feedback topology, installed native-library identity, physical RF-off,
G2 latency bound, mobile microphone behavior, TLS runtime integration, live
feedback thresholds, overload-interval qualification or RF IMD/linearity is
established by this package. Browser timestamps describe conservative graph
observation age, not physical microphone ADC or RF-output age. Slow telemetry
is not fast overload protection.

Next review should evaluate this managed PCM UI/host contract and its explicit
gates before authorizing a production host selection. SATP stays with the same
sole owner. Continuous Opus design/qualification remains separate and does not
hold up PCM; banked Opus stays stopped. No FPGA, driver or polling assignment
is justified by this slice.
