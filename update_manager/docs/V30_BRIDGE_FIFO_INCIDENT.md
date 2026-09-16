# V30 Direct-XDMA bridge FIFO incident

Status: repair candidate, not hardware-qualified

Observed: 2026-09-15 on `saturn-g2`

FPGA artifact under test: `saturn-primary-v30-a904f42b.bin`
FPGA source commit: `a904f42b1dcc3e23a08b4af604983f90a161453b`

## Symptom

Saturn Remote's split control WebSocket opened and immediately closed with
code 1005; the media lane then failed with code 1006. TLS and authentication
were successful. Saturn Go closed the browser-facing sockets because its
upstream `saturn-bridge` process exited.

The decisive bridge error was:

```text
operational XDMA RX FIFO fault: depth=16385 overflow=1 threshold=1 underflow=0
```

One reproduction processed an eight-command browser startup batch for 54,058
microseconds immediately before this fault. The operational bridge serviced
DDC C2H reads, WDSP, WebSocket publication, and client control in one loop.
The control batch therefore withheld C2H service for approximately the fill
time of the 16,384-word DDC FIFO at 384 ksps.

The G2 checkout was commit `24e8f5c8cebd517e7ddc726cb1506e148c3ff976`.
Its bridge admitted firmware 1.30 but still used the older unconditional
`overflow || underflow` fatal rule. It did not contain commit `f322c15`
(`Handle V29+ RX FIFO status correctly`). The installed bridge SHA-256 was
`ca2c859d91ca7827339c67722818095bbf48a7ad1958bdcaba416243c2b4dd1a`.

## Why V27 appeared to work

V27's top-level block design disabled `HAS_AFULL` on the four monitored AXIS
FIFOs and tied all four `FIFO_Monitor` overflow inputs to constant zero. Its
legacy status bit 31 could consequently never assert. The old bridge could
pause long enough to fill the DDC FIFO without terminating on bit 31, although
that behavior did not prove that samples were preserved.

The first V30 compatibility attempt excluded the new `almost_full` inputs from
legacy bit 31 but synthesized that bit from `count >= configured depth`. That
was not the exact V27 host contract. When control work starved C2H and the DDC
count reached 16,385, V30 asserted bit 31 and the deployed legacy bridge exited.

## Repair contract

The repair has two independent requirements:

1. V30 legacy status must exactly preserve V27's observable boundary. Legacy
   bit 31 remains zero; bits 29 and 30 retain their V27 read-to-clear behavior.
   Almost-full and configured-capacity observations remain available through
   the extended minimum, maximum, and transition telemetry.
2. The bridge must not depend on hiding a full FIFO. A dedicated priority-22
   C2H owner continuously drains `/dev/xdma0_c2h_0` into a preallocated,
   page-aligned, locked 256-buffer/8 MiB ring. WDSP, WebSocket, filesystem, and
   control work consume that ring independently. If the consumer exhausts the
   bounded reserve, the oldest unread host buffer is discarded and explicitly
   counted so the hardware FIFO remains serviced.

V29 retains its version-specific legacy status interpretation. V30 uses the
restored V27 legacy contract. Direct RF transmission remains inhibited for
unqualified V28/V29/V30 firmware.

## Required qualification

The repair is not qualified merely because Vivado timing and implementation
gates pass. Before release it requires an end-to-end G2 test with the Direct-
XDMA backend at 384 ksps that:

- opens both split WebSocket lanes and sends the normal browser startup burst;
- exercises repeated connect, disconnect, and reconnect cycles;
- verifies `saturn-bridge.service` never restarts;
- verifies FPGA FIFO extrema/events and host-ring drop/discontinuity counters;
- completes controlled RX and antenna RX soaks;
- keeps TX inhibited until a separate dummy-load TX qualification.

The original V30 artifact and logs remain historical evidence. A repaired
artifact must carry a new Git SHA and manifest and must not overwrite the
`a904f42b` build.

## Repaired-FPGA field follow-up

The repaired FPGA artifact built from commit
`104d5c569054909aef0de27a3458844d0fd4df17` passed implementation with
WNS `+0.119 ns`, WHS `+0.049 ns`, and all DRC, CDC, methodology, and telemetry
netlist gates clear. Its primary-slot image is
`saturn-primary-v30-104d5c56.bin`, SHA-256
`d80eebf7eb126e23e9922bf05c155a5a7bfa29f190a4eb0d850b0cd1d5f82974`.

After that FPGA was loaded, the browser completed one split-WebSocket session
and received 384 kHz IQ, confirming that TLS, authentication, proxy routing,
split-lane pairing, and V30 DDC output could all operate. A later reboot still
used the old installed bridge binary
`ca2c859d91ca7827339c67722818095bbf48a7ad1958bdcaba416243c2b4dd1a` and the
old service ceiling `LimitRTPRIO=21`. That bridge repeatedly exited with:

```text
operational XDMA RX FIFO remained over threshold after 16 bounded startup drains
```

Systemd restarted it five times and then marked `saturn-bridge.service`
failed. `saturn-go.service` remained active and its subsequent proxy attempts
failed with connection refused because no bridge listener remained. This is
the expected incomplete state when only the FPGA half of the two-part repair
is installed; it is not evidence that the repaired FPGA reintroduced the
original bit-31 failure. Hardware qualification begins only after the matching
priority-22 dedicated-reader bridge and service unit are deployed.

## Direct-RX byte-order field follow-up

Deploying the matching priority-22 bridge allowed split control and media
lanes to remain connected, but received audio and spectrum were unusable
full-scale noise with no discernible stations. This was a separate Bridge
ownership bug, not another V30 FIFO-monitor failure.

Read-only capture through the Bridge's TCI IQ output showed ten consecutive
frames at approximately `0.57` to `0.60` normalized RMS (`-4.9` to `-4.4`
dBFS), with peaks repeatedly reaching `0.999`. Reinterpreting the exact same
24-bit sample bytes in the opposite order produced approximately `0.014` to
`0.027` RMS (`-37.2` to `-31.4` dBFS). The stream had no header errors,
resynchronizations, FIFO faults, host-ring drops, or discontinuities.

P2 establishes the required representation during startup with
`SetByteSwapping(true)`, which sets RF GPIO bit 26 before DDC operation. The
Direct-XDMA RX path decoded every sample as signed 24-bit network byte order
but never set or verified that global FPGA bit. Earlier V27 tests could inherit
the correct bit from a preceding P2 owner; a cold boot or ownership sequence
that did not run P2 left the power-on local byte order active. The resulting
byte-reversed sample magnitudes looked like near-full-scale random noise even
though DMA framing remained structurally valid.

The Bridge repair now sets and reads back RF GPIO bit 26 before enabling either
the probe or operational DDC stream. Failure to establish the byte order is
fatal before samples are published. Correcting the sample representation also
removes the false near-full-scale input that drove WDSP's meter to the observed
`S9+51` level; ordinary per-radio S-meter calibration remains a separate trim.
