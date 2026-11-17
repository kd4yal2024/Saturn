# P2 v4.4 DUC sequence lifecycle

Reference: TAPR **openHPSDR Ethernet Protocol v4.4**, source commit
[`09998920f4f6f1aff52261667b7efbdb46b34037`](https://github.com/TAPR/OpenHPSDR-Firmware/blob/09998920f4f6f1aff52261667b7efbdb46b34037/Protocol%202/Documentation/openHPSDR%20Ethernet%20Protocol%20v4.4.pdf).

## Wire behavior

DUC-specific control packets carry zero in bytes 0–3 on every send. The Bridge
previously incremented this field. Both the ordinary control sender and managed
feedback-layout sender now use the same fixed-zero builder. A literal 60-byte
specimen independently checks the complete control packet.

DUC IQ data packets retain an incrementing, wrapping 32-bit sequence. A new
session starts at zero. A successful RUN=0 command submission resets the next
data sequence to zero; ordinary PTT release with RUN=1 preserves it across
overs. The Bridge previously kept incrementing after RUN stopped.

A session mutex covers sequence assignment, packet construction and completed
UDP submission. RUN=0 submission and reset take that same mutex, preventing a
reset from crossing a concurrent data send. Failed sends do not advance/reset
the counter. Successful submission means the local socket accepted the complete
datagram; it does not prove receipt by the radio.

Existing output authorization remains authoritative. Fenced callers acquire
their existing output permit before the sequence mutex; generation checks and
closed/drained release handling remain in place. The mutex orders packet
submission; it does not grant TX permission or cancel Legacy audio queues.
The background high-priority loop only sends RUN=1 states and cannot reset the
data sequence.

## Verification

Actual loopback datagrams cover repeated zero control sequences, IQ continuity
through PTT off/on, explicit stop and generic RUN=0 reset, failed stop, 32-bit
wrap, concurrent writers and stop, and managed release/re-key/RUN-stop with
stale-generation rejection. The original control-sequence and stop-reset
defects were reproduced before the repair.

Run the focused suite from the repository root:

```sh
SATURN_BRIDGE_STUB_NATIVE=1 cargo test --locked \
  --manifest-path update_manager/saturn-bridge/Cargo.toml \
  --features managed-puresignal --bin saturn-bridge p2 -- --test-threads=1
```

Full default/managed suites and real-WDSP receive/release/rearm and Auto ATT
fixtures also pass. Real-WDSP fixtures use files and loopback sockets, without
radio access. Evidence is retained in
`Downloads/Saturn_P2_v44_Slice2_20261117`.

This repair changes neither sample scaling nor TX ownership, FPGA code, driver
behavior, PS admission, accessory control or timestamp interpretation. It is
not a complete v4.4 conformance or physical RF qualification claim.
