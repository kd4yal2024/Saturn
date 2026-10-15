# PureSignal milestone 2 — split generation / Opus-history contract

2026-10-13, isolated SilverForge base
`861f0fbc79e0e36c68a5c8ddc157e010b8be20df`.

## Outcome and boundaries

Implemented an **explicit offline PSG1 review profile** in the existing shared
owner. Actual split WebSocket control/media sockets now exercise tagged PCM and
banked Opus through real decoding, the full native TX wrapper and both P2/direct
fixtures. Each replay establishes correction across release/re-arm and a SATP
source change, injects delayed old media after re-arm, then completes fault
stop/cleanup. The accepted ordered-settings path remains intact.

Production remains disabled by default. Installed startup still selects Legacy;
the earlier SharedReview profile still inhibits split media and Opus unless this
additional profile is explicitly selected before initial configuration and
frontend attachment. No URL/env switch, installed call or legacy fallback was
added. No commit/push, deployment, G2, RF, FPGA, driver or polling change.
Firmware 1.31.002 / `0x53460004` and GoldMarsh's work remain untouched.

This is **not drop-in qualification of the existing continuous Opus browser
sender**. It proposes separate short encoder/decoder streams (banks), not an
invented finite memory horizon for a continuous decoder. The browser reference
is unselected; actual AudioWorklet/capture recreation and real WebCodecs execution
are still integration/qualification work. Bank-reset speech quality, CPU load,
latency and spectral behavior have not been qualified. Do not enable live TX on
the strength of these offline tests.

## Source-linked path

```text
fresh authorized TX action -> existing owner opens output generation
 -> owner publishes opaque PSG1 token on CONTROL lane
 -> client replaces capture/resampler/framing history, awaits reset acknowledgement
 -> immutable token/sequence/oldest age recorded BEFORE framing or encoding
 -> MEDIA lane frame echoes token (PCM or four-packet independent Opus bank)
 -> original bridge-ingress capture -> current token/lane/codec/sequence checks
 -> serialized decoder -> oldest bank observation -> stamped existing audio FIFO
 -> SAME shared lifecycle/full native engine/output adapter

release / disconnect / reconfiguration / fault
 -> output fence closes -> old publication retired
 -> receipt/native cleanup -> fresh action/new token required
```

[RemoteMedia](../../saturn-bridge/src/tx_thread/ps_media.rs) separates short owner
publication locking from decoder locking. Neither cancellation nor publication
waits on libopus/history. Decoder replacement is lazy on the first admitted
packet of a new token or bank; retained state is never rebound to a new token.
[SharedIngress](../../saturn-bridge/src/tx_thread/ps_ingress.rs) supplies original
observations and feeds the existing audio queue, not a second TX engine.

[Actual socket routing](../../saturn-bridge/src/tci/client.rs) checks current
operator/session liveness and structural pairing before admission. A paired
session accepts mic only on its media lane. The control owner's codec selection
is authoritative: codec negotiation may precede the other socket's handshake,
so a lagging legacy mirrored codec/decoder is not an authority source. Old
generation checks precede codec/lane hazards. Untagged/old packets are rejected
without decoding or canceling a healthy new generation. Current-generation
hazards stop; no automatic PCM fallback, PLC or legacy mic tee occurs.

[Conditional cancellation](../../saturn-bridge/src/tx_owner_stop.rs) closes the
matching output revision with one atomic CAS. A delayed fault belonging to an
old frame cannot close a newly reopened generation. Permitted writes already in
flight remain in flight; a complete stop still requires the existing receipt.
P2 command completion and direct readback are not physical RF-off measurements.

## Wire contract (review version PSG1)

The existing 64-byte TCI mic header is retained. The extension uses previously
reserved bytes; no FPGA/driver format changes are involved.

| Field | Contract |
| --- | --- |
| Initial capability | `saturn_tx_media_caps:psg1,pcm,opus_wb,bank4;` |
| Control offer | `saturn_tx_generation:psg1,<32 lowercase hex token>,<u64 serial>;` |
| Retirement | `saturn_tx_generation:psg1,off,<serial>;` |
| Bytes 0–3 | Receiver 0 only, little-endian |
| Bytes 32–35 | u32 packet sequence, first 0, strictly contiguous, no wrap |
| Bytes 44–59 | Raw 16-byte token (hex-decoded in textual order) |
| Bytes 60–63 | Little-endian `0x50534731` |
| Payload | Exact declared byte length; mono, 48 kHz, at most 1,024 decoded samples |
| Opus WB | Codec 2 / sample type 1 / exactly 960 samples (20 ms); NB unsupported |
| Opus bank | Sequences 0–3 one stream, 4–7 a fresh encoder/decoder, and so on |

Tokens derive from a fresh OS-random 256-bit secret and checked monotonic
publication serial via HMAC-SHA256 (first 16 bytes). RNG or counter failure is
fail-closed; re-arm/boot do not deliberately reuse tokens. These are replay
bindings, **not new authentication**. Existing trusted operator/pairing checks
remain essential. The control client shares the token with its media writer.
Offer delivery uses the existing non-evicting transaction FIFO; backpressure
cancels/revokes rather than silently losing publication.

The token binds output generation/revision, boot, configuration and release.
It does not rotate at the feedback/acquisition-epoch transition when key receipt
arrives: the output generation has not changed. Each queued audio envelope still
retains its actual acquisition/feedback stamp; pre-key queued data is handled by
the existing owner checks/clear. Source changes/configuration/release retire the
token; codec or pairing changes during active TCI request a stop.

## Opus history and browser producer

The [Opus decoder API](https://opus-codec.org/docs/opus_api-1.5/group__opus__decoder.html)
describes ordered stateful decoding. We do not infer that a 20 ms packet bounds
all decoder history. The review policy destroys/recreates decoder state every
four packets and requires the sender to create a **matching independent encoder
bank**, not merely change timestamps on continuous encoded packets. No loss/FEC
concealment is requested (`decode_fec=0` and no null packets).

All samples decoded in a bank conservatively carry its oldest bridge arrival.
A fresh packet cannot refresh that age. Stale history (150 ms limit), gaps,
reordering, wrapping, wrong codec/route, invalid formats or non-finite data are
refused; ingress requests a matching-generation stop. The oldest observation is
preserved through TCI partial mic blocks and native output envelopes as before.

The [client reference](../../remote-web/src/transport/tx-generation-review.ts)
is not imported by the installed entrypoint. It requires explicit enablement and
a capture factory acknowledging replacement after the requested generation
boundary. Old source deliveries/promises cannot revive a generation. It freezes
authority before async encoding, flushes each four-input encoder bank, retains
oldest input age across callbacks/reordering, and closes a bank only after flush
and all expected outputs. Two pending banks/eight ready packets bound queues.
Missing/unsolicited output, stale age or overflow releases rather than inventing
sequence-zero data. Release invalidates first, sends unkey, then cleans up.
Late callbacks/errors from closed banks cannot acquire current authority.

Monotonic offer serials reject previously seen old offers and delayed retirement
of an older generation. A new WebSocket/bridge session must create a new client
instance; old callbacks remain tied to the retired instance. This API's reset
acknowledgement must be supplied by a real capture/resampler/worklet adapter
before selectable production integration. It is **not a claim that today's
ordinary browser worklet has already been migrated**.

## Executed checks and limits

| Check | Result |
| --- | --- |
| Decoder/generation unit cases | 8 passed |
| Actual role-parser/shared-owner media cases | 7 passed |
| Owner Cargo with native stubs | 46 library + 442 binary passed; 2 existing ignored |
| Default Cargo with native stubs | 24 library + 379 binary passed; 2 existing ignored |
| Real WDSP combined runner | 7 serial tests passed, both backend fixtures |
| Browser reference + existing framing/Opus tests | 38 passed (12 new reference cases) |
| Full web worktree suite | 711 passed across 83 files |
| Web typecheck, Rust formatting, whitespace, runner ShellCheck | Passed |
| Standalone strict/proposal/lifecycle and native controls | 24 / 32 / 46 and positive control/canaries passed |

Native split replays use two actual loopback sockets, real libopus decoding,
native rendering/output, synthetic feedback through actual decoders, three
corrected generations, delayed old media on re-arm, authenticated SATP and
latched discontinuity/cleanup. They assert one native exchange per turn. Decoder
bank reset is compared with a freshly created real decoder. The encoded payload
is the existing deterministic Opus fixture, **not a real browser microphone or
WebCodecs encoder**. Client callback/encoder tests use WebCodecs API-shaped shims.

Host libopus: package `libopus0:amd64 1.3.1-0.1build2`,
`/usr/lib/x86_64-linux-gnu/libopus.so.0.8.0`, SHA-256
`f63e3597e52b57f8957bafb309e9f4d91108a3987136ace5f386f269a0947e55`.
It is a test dependency, not bundled or an identification of G2's library.
Pinned offline x86-64 WDSP archive remains
`8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248`.
Never install the offline archive on G2.

Timestamps establish client-local source history and bridge-ingress/decoder age,
not shared wall-clock, transit time or exact RF-output age. Installed-library
identity, real acquisition-owner feedback hookup/topology, suitable live level
thresholds, capture adapter/browser execution, voice/IMD quality, and measured G2
stop/recreation timing remain unqualified. Existing slow telemetry is not fast
overload protection. No faster polling or additional FPGA/driver work follows
from this host review.

Current transcripts: `offline-native/media-*`; previous slices are historical.
These are SilverForge execution results, not the external reviewer's independent
execution. The package includes the complete web source and targeted transport
tests; the 711-test result was run in the full worktree, not a claim that every
unrelated UI/template fixture is included. Reproduce from extracted root:

```sh
sha256sum --check SHA256SUMS
bash update_manager/saturn-bridge/tests/run-puresignal-owner-adapters.sh
bash update_manager/saturn-bridge/tests/run-puresignal-owner-native.sh offline-native/wdsp-build
bash update_manager/saturn-bridge/tests/run-puresignal-offline-native.sh offline-native/wdsp-build
cd update_manager/remote-web
# Install locked dev dependencies (or use an existing matching node_modules).
npm ci
npm run typecheck
npm test -- tests/tx-generation-review.test.ts tests/tx-opus-encoder.test.ts tests/tx-uplink.test.ts
```

Requires Rust/cached crates, compiler/binutils, FFTW development files, libopus
runtime, Node and locked web dependencies. Review the wire/banked-stream policy
and unselected browser API before authorizing a selectable production slice.
