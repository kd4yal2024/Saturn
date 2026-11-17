# P2 v4.4 packet contract — first slice

This slice corrects the Rust Bridge's discovery metadata and validates its
selected big-endian, signed 24-bit DDC profile. It does not change the C server's
advertisement, status cadence, TX sequence lifecycle, owner, firmware, driver,
sample scaling or PureSignal admission.

Reference: TAPR **openHPSDR Ethernet Protocol v4.4**, source commit
[`09998920f4f6f1aff52261667b7efbdb46b34037`](https://github.com/TAPR/OpenHPSDR-Firmware/blob/09998920f4f6f1aff52261667b7efbdb46b34037/Protocol%202/Documentation/openHPSDR%20Ethernet%20Protocol%20v4.4.pdf).

## Discovery

The previous parser read byte 13 into `protocol_version`, conflating firmware
and protocol revisions. It now records the separate standard fields:

| Offset | Meaning |
| --- | --- |
| 12 | Protocol revision |
| 13 | Firmware code version |
| 20 | Implemented independent DDC count |
| 21 | Frequency representation: 0 Hz, 1 phase word |
| 22 | Available endian/data-format flags; zero means big-endian three-byte IQ |
| 23 | Beta version |

The current Saturn server uses byte 23 for its p2app build number. That raw beta
value remains available, and `saturn_app_version()` labels it as p2app only for
the standard Saturn device code 10. Other boards retain their beta value without
being called Saturn builds. The status line retains its `proto`, `p2app` and
MAC tokens and adds raw firmware, DDC count, frequency format, data-format flags
and beta. Non-Saturn app builds are `n/a`.

Capabilities are metadata in this slice. Unknown capability values remain raw;
they neither select a new format nor add a controller-admission gate. A legacy
4.3 reply remains accepted and reported as 4.3. The C server still advertises
4.3; this parser repair does not silently upgrade that claim.

## Normal DDC decoding

The Bridge's existing General command selects big-endian three-byte IQ. The
ordinary DDC parser now requires a complete 1,444-byte datagram, declared width
24, and a nonzero sample count fitting the 1,428-byte payload (at most 238
complex samples). Rejection occurs before allocation or sample decoding.
Unsupported widths are not interpreted as 24-bit audio or IQ. Smaller positive
counts remain valid; bytes beyond the declared samples are not decoded.

Valid sample conversion is unchanged: signed 24-bit integer divided by
8,388,608. The 22/Q24 and saturating-output firmware changes preserve that
transport. A loopback dispatch regression verifies an invalid-width datagram is
consumed as socket work, with no RX event, and a following valid packet still
decodes. Rejection does not add an idle sleep or stop the session. The managed
PS feedback decoder keeps its separate existing validation and ownership.

## Independent fixtures and validation

[`fixtures/README.md`](../saturn-bridge/src/p2/fixtures/README.md) documents four
synthetic wire specimens derived from the pinned specification tables. Tests
read literal hex, not output from production encoders. Distinct protocol and
firmware bytes expose the original offset error. These are not live captures.

Coverage includes Saturn and generic/busy discovery, legacy replies, raw unknown
capabilities, status reporting, sample-width rejection, packet/count boundaries,
signed extremes, the final packet sample, reference/RF deinterleaving, and v4.4
FIFO/ADC-peak status offsets. The nonzero DDC timestamp fixture is opaque to this
decoder; it is not interpreted as a fresh FPGA acquisition timestamp.

Run the packet/dispatch suite with:

```sh
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --locked \
  --manifest-path update_manager/saturn-bridge/Cargo.toml \
  --features managed-puresignal --bin saturn-bridge p2:: -- --test-threads=1
```

Full default and managed stub suites, formatting and clippy correctness checks
are separate from real-native file/loopback receive/release/rearm and Auto ATT
tests. Native tests run serially in fresh processes with the reviewed WDSP
archive; they do not open a radio. Neither set of results establishes physical
RF performance, a CM4 CPU gain or complete v4.4 conformance.

Evidence is retained in `Downloads/Saturn_P2_v44_Slice1_20261116`, including the
two failing baseline regressions, repaired tests, serial native logs and source
checksums. The subsequent [DUC sequence repair](P2_V44_SEQUENCE_LIFECYCLE.md)
and [status cadence repair](P2_V44_STATUS_CADENCE.md) document their own scope
and verification. C identity advertisement and further performance work remain
follow-ups.
