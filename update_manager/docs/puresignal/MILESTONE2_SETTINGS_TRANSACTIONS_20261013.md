# PureSignal milestone 2 — ordered settings transaction correction

Date: 2026-10-13. Isolated SilverForge base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome

Separate settings messages no longer independently derive competing snapshots
from the last publication. The producer serializes projected settings and links
each accepted proposal to its predecessor's owner-issued completion receipt.
The owner finishes one transaction before dequeuing the next. Mic gain followed
by EQ in two separate messages preserves both updates, with distinct configuration
IDs and explicit client-visible completion.

Offline review only. Installed startup remains Legacy; SharedReview is disabled
by default. Opus and split-media TX remain inhibited. No commit, push, deployment,
G2, RF, firmware, FPGA, driver or polling change. GoldMarsh's tree is untouched.
Firmware 1.31.002 / `0x53460004` remains unchanged.

## Source-linked contract

```text
separate operator settings messages
 -> original capture + sealed settings permission + cancellation epoch
 -> serialized projected model/PS/source + explicit predecessor receipt
 -> bounded existing command FIFO (16)
 -> owner stop receipt -> native cleanup -> model installation/publication
 -> applied receipt containing exact owner boundary
 -> next settings transaction checks that receipt against current boundary

release / disconnect / fault / successful fresh arm
 -> invalidate settings epoch (release closes output fence first)
 -> pending/queued old transactions rejected, not rebound
```

[SharedIngress](../../saturn-bridge/src/tx_thread/ps_ingress.rs) retains the
original stamp. The settings continuation is deliberately different from arm
and audio authority. A first transaction still requires an exact stamp match.
A successor requires a successful predecessor receipt matching **all fields of
the current owner stamp**, plus original boot/release, unexpired cancellation
epoch, and live operator permission. Configuration cleanup legitimately advances
acquisition generation/configuration/feedback epoch; only this receipt chain can
bridge that transition. Unknown generation/revision changes cannot match it.
An arbitrary stale command gets no fresh stamp or authority. Failed predecessors
cannot authorize successors. Projected settings are never published as applied.

[SettingsPermission](../../saturn-bridge/src/tci/ps_authorization.rs) is sealed
before queuing and revoked by session removal or operator change. It cannot key
the radio. [OwnerLoop](../../saturn-bridge/src/tx_thread/ps_owner_loop.rs) continues
periodic watchdogs, release service, audio rejection and lifecycle cleanup while
command dequeue waits for the bounded pending configuration. Configuration
Drop supplies a terminal rejection for otherwise discarded envelopes; terminal
receipt state is write-once, so later drops cannot overwrite a failure reason.
Cancellation during native installation remains ordinary cancellation, not a
new fault. Native errors retain their fault and stop/cleanup progression.

Neither settings nor their completion re-arms TX. The old pre-settings arm is
still rejected. A fresh authorized action after committed publication is needed.
Committed state is published before the applied reply; a client responding to
completion can capture the new boundary. SATP source selection still requires
the existing lease/readiness check. Existing output fencing, age provenance and
native exclusivity are unchanged.

## Client results and bounded delivery

Each nonempty settings batch produces a bridge-session transaction ID. Up to 64
settings in one WebSocket message remain one atomic proposal; separate messages
produce separate proposals. Wire text on the originating operator connection:

| Result | Text |
| --- | --- |
| Proposal queued (not completion) | `tx_config:0,<transaction>,queued,<proposed-config>;` |
| Committed and published | `tx_config:0,<transaction>,applied,<config>;` |
| Rejected/canceled | `tx_config:0,<transaction>,rejected,<Fault>,<proposed-config>;` |

Validation before a model is built uses proposed-config `0`. IDs may have gaps;
they are not persisted across bridge restarts. Queue-full proposals have queued
then rejected results. Same-message release now reports rejection of its retained
settings batch rather than silently clearing it. Existing `tx_fault` messages
remain supplementary, not transaction completion.

[Outbound transaction replies](../../saturn-bridge/src/tci/outbound.rs) use a
dedicated 64-slot FIFO, with safety/unkey messages retaining priority. Replies
are not coalesced or evicted by ordinary telemetry/control/safety churn. Writer
retry retains its one in-flight item (maximum 65 including that item). If a reply
cannot be queued, cancellation closes the output fence before session revocation
and close-message queueing. Future authority is refused; no successful enqueue
is interpreted as an applied setting.

This is not durable end-to-end delivery: a disconnected socket cannot receive
its rejection, and closing an overloaded session may leave an undelivered
result. Clients must treat absence of a terminal result/connection loss as
unknown, not success, reconnect/read committed state, then submit a new authorized
request. The replay verifies wire replies; no production browser UI is enabled
or changed by this slice.

## Executed evidence

| Check | Result |
| --- | --- |
| Focused deterministic settings cases | 9 passed |
| Owner Cargo with native stubs | 46 library + 427 binary passed; 2 existing ignored |
| Default Cargo with native stubs | 24 library + 379 binary passed; 2 existing ignored |
| Real WDSP owner runner | 5 serial combined tests passed, both backend fixtures |
| Standalone controller/proposal/lifecycle and native controls | 24 / 32 / 46 passed; native positive control/canaries passed |
| Formatting, whitespace and runner ShellCheck | Passed |

The nine cases cover two separate messages before any owner turn, release while
the first update is pending, disconnect during that interval, native installation
failure with a dependent successor, cancellation inside native installation,
command-queue saturation with one terminal result per message, and reply-lane
saturation with no eviction and explicit cancellation/revocation, plus stop-receipt
and native-cleanup timeouts producing rejection instead of hanging pending. Happy/release
controls require fresh configuration/arm; old arm authority still fails. Tests
assert no further output, empty queues and actual file-fixture stop readback.

The actual WebSocket/SATP/full-native replay now sends separate mic-gain and EQ
messages, waits for the second applied reply, verifies both published values and
native panel gain/EQ state, then exercises three corrected generations on P2 and
Direct XDMA. It retains the single-exchange-per-turn assertions and completed
stop/native cleanup. Feedback is synthetic through real decoders, not live RXC1
acquisition. Loopback packet completion/file readback do not prove physical RF-off,
G2 latency, production feedback thresholds or installed native-library identity.

Current evidence is `offline-native/settings-*`; older transcripts/reports are
historical. Results are SilverForge's executed evidence, not the external
reviewer's independent execution. Reproduce from extracted archive root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
bash update_manager/saturn-bridge/tests/run-puresignal-owner-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
```

Requires Rust/cached dependencies, compiler/binutils and FFTW development files.
The runner verifies the 22 pinned WDSP inputs. Bundled x86-64 offline archive:
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
Never install the offline archive on G2. This correction requests only offline
source/test review, not live admission or hardware-test authorization.
