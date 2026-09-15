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
