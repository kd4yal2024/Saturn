# Saturn FPGA / Protocol 2 Handoff

Date: 2026-03-26

## Purpose

This is an internal working handoff note for future Saturn FPGA, Protocol 2,
`P2_app`, and `P3_app` work.

It consolidates what we currently know from:

- the Saturn FPGA source tree
- the local openHPSDR Protocol 2 v4.4 specification
- the Saturn `P2_app` / `P3_app` implementation

This document is meant to be the fast-start reference for future development
and review work. It is not intended as end-user documentation.

## Primary Sources Reviewed

- `FPGA/README.md`
- `FPGA/create_saturn_project.tcl`
- `FPGA/sources/wrapper/saturn_top_wrapper.v`
- `FPGA/sources/verilogmodules/DDCMux.v`
- `FPGA/sources/verilogmodules/wideband_collect.v`
- `FPGA/sources/verilogmodules/AXI_FIFO_overflow_latch_reader.v`
- `FPGA/sources/verilogmodules/activitywatchdog.v`
- `FPGA/sources/verilogmodules/axi_stream_deinterleaver.v`
- `FPGA/sources/verilogmodules/AXILite_Alex_SPI.v`
- `FPGA/sources/verilogmodules/usr_reg_access.v`
- `FPGA/sources/verilogmodules/axil_config256_reg.v`
- `FPGA/sources/verilogmodules/axil_config64_reg.v`
- `FPGA/sources/verilogmodules/axil_read64_reg.v`
- `../../../OpenHPSDR-Firmware/Protocol 2/Documentation/openHPSDR Ethernet Protocol v4.4.docx`
- `../../../OpenHPSDR-Firmware/Protocol 2/Documentation/openHPSDR Ethernet Protocol v4.4.pdf`
- `sw_projects/P2_app`
- `sw_projects/P3_app`
- `sw_projects/common`

Related companion note:

- `docs/protocol2/Saturn_vs_openHPSDR_v4.4.md`
- `docs/protocol2/Saturn_LinuxDriver_XDMA_Handoff.md`

## Executive Summary

The FPGA tree confirms that Saturn hardware is richer than what the current
discovery reply advertises.

The most important confirmed facts are:

- Saturn hardware has `2` ADC front ends.
- Saturn hardware has `10` DDC channels.
- Saturn hardware has a real wideband capture block.
- Saturn hardware has real FIFO overflow and ADC peak-hold support.
- Saturn hardware enforces TX watchdog protection in the FPGA.
- Saturn hardware has a TX-sample feedback path (`TXSamplesToRX`).
- Saturn is physically wired for a `4`-lane PCIe endpoint at the FPGA boundary.

The current Saturn apps are best described as:

- "Protocol 4.3 plus selected 4.4 fields"

not:

- "full Protocol 4.4 implementations"

The biggest current software/protocol mismatches remain:

- discovery byte `12` advertises protocol `43`, not `44`
- discovery byte `20` advertises `4` DDCs while FPGA hardware implements `10`
- discovery byte `23` is used as an app-version field instead of the spec beta flag
- high-priority data bytes `1396-1397` (`ClientControl`) are not implemented

The practical takeaway is:

- the FPGA should be treated as the hardware source of truth
- the app layer should be treated as a compatibility layer on top of that hardware

For the Linux kernel / XDMA layer that sits between FPGA hardware and the app
layer, see:

- `docs/protocol2/Saturn_LinuxDriver_XDMA_Handoff.md`

## What The FPGA Tree Tells Us

### 1. Build And Device Baseline

From `FPGA/README.md`:

- the intended build flow is to open the checked-in Vivado project directly
- the recommended tool version is `Vivado 2023.1`

From `FPGA/create_saturn_project.tcl`:

- target device is `xc7a200tfbg676-2`
- project name is `saturn_project`

The FPGA README version history is also useful because it dates major hardware
feature additions:

- `V18`: wideband data collection introduced
- `V20`: TX watchdog added
- `V26`: debug LO DDS selection added

### 2. Top-Level Hardware Shape

From `FPGA/sources/wrapper/saturn_top_wrapper.v`:

- there are `ADC1_*` and `ADC2_*` interfaces
- there is one DAC / TX chain
- there are codec / I2S interfaces
- there are Alex / RF control signals
- the PCIe MGT interface is exposed as `[3:0]` lanes

This confirms a design centered around:

- `2` ADCs
- `1` TX path
- a physically `x4`-capable PCIe endpoint interface

Important note:

- Linux on the Pi may still negotiate the link as `x1`
- that does not change the FPGA-side physical topology

### 3. DDC Count Is Hardware-Confirmed At 10

From `FPGA/sources/verilogmodules/DDCMux.v`:

- the module comment explicitly says `10 input axi streams`
- `localparam DDCChannels=10`
- inputs `s00` through `s09` are wired

From `FPGA/IP/repo/DDC/saturn_project.tcl`:

- five dual-DDC blocks are instantiated and wired into the mux:
  - `dualDDC10`
  - `dualDDC32`
  - `dualDDC54`
  - `dualDDC76`
  - `dualDDC98`

That is the strongest FPGA-side evidence that Saturn hardware implements `10`
DDCs.

Implication:

- the current app discovery reply under-reporting `4` DDCs is a software
  compatibility choice or legacy artifact, not a hardware limitation

### 4. Wideband Capture Is Real And Limited To Two ADC Sources

From `FPGA/sources/verilogmodules/wideband_collect.v`:

- the block records `4K-16K` samples from `ADC0` or `ADC1`
- it is controlled through AXI-Lite registers
- it writes sample batches into a FIFO via AXI stream

From `FPGA/create_saturn_project.tcl`:

- the block is memory-mapped at `0x0000D000`
- FPGA ADC1 is wired into `Wideband_Collect_0/adc0`
- FPGA ADC2 is wired into `Wideband_Collect_0/adc1`

Implications:

- Saturn wideband support is hardware-backed
- the hardware view is still fundamentally `2`-ADC, even if generic Protocol 2
  language elsewhere can describe more ADC inputs on other hardware

### 5. FIFO Overflow And ADC Peak Hold Are Hardware Features

From `FPGA/sources/verilogmodules/AXI_FIFO_overflow_latch_reader.v`:

- FIFO overflow bits are latched until software reads them
- ADC peak magnitudes are accumulated and latched alongside the overflow period
- the register interface exposes:
  - overflow latch data
  - ADC1 peak
  - ADC2 peak

From `FPGA/IP/repo/DDC/saturn_project.tcl`:

- DDC overflow signals from all five dual-DDC blocks feed the receiver-side
  overflow reader

Implications:

- the high-priority status fields for overflow and ADC max magnitude are backed
  by dedicated FPGA support
- those fields are not just software estimates

Important naming note:

- the FPGA uses `ADC1` / `ADC2`
- the Protocol 2 v4.4 spec labels equivalent byte positions as `ADC0` / `ADC1`
- this is primarily a naming offset, not a byte-position mismatch

### 6. TX Watchdog Protection Is Enforced In Hardware

From `FPGA/sources/verilogmodules/activitywatchdog.v`:

- TX enable is asserted only while FIFO activity is being observed
- if activity stops for the timeout period, TX enable drops

From `FPGA/create_saturn_project.tcl`:

- watchdog enable and watchdog TX state are explicitly wired through the design

Implication:

- TX safety is partly enforced in FPGA hardware
- it is not purely a Linux or `P2_app` / `P3_app` policy decision

### 7. TX Sample Feedback Exists In Hardware

From `FPGA/sources/verilogmodules/axi_stream_deinterleaver.v`:

- the block comment explicitly says it can pass I/Q TX samples onward

From `FPGA/create_saturn_project.tcl`:

- `Transmitter/TXSamplesToRX` is wired into `Receiver/tx_samples`

Implication:

- the FPGA contains a real TX-sample-to-RX feedback path
- this supports debug and PureSignal-style routing concepts

### 8. Alex / RF Control Is Hardware-Backed

From `FPGA/sources/verilogmodules/AXILite_Alex_SPI.v`:

- there is a dedicated AXI-Lite-to-Alex SPI control block
- it supports separate RX and TX data words
- it includes backward-compatibility behavior for older clients that do not
  provide separate TX antenna data

From `FPGA/create_saturn_project.tcl`:

- Alex SPI is memory-mapped at `0x0000B000`

Implication:

- Saturn app handling of Alex words is clearly backed by a dedicated FPGA block

### 9. FPGA Identity And Readback Exist

From `FPGA/sources/verilogmodules/usr_reg_access.v`:

- the bitstream exposes `USR_ACCESS` data

From `FPGA/create_saturn_project.tcl`:

- FPGA readback registers are wired through `AXIL_ReadReg_64`
- ID/status style information is exposed through the PCIe AXI-Lite map

Implication:

- firmware identity and some build/version information can be exposed directly
  from FPGA-side registers

## XDMA / AXI-Lite Address Map Notes

The most relevant Saturn FPGA control/status regions currently visible in the
block-design TCL are:

| Address | Block |
| --- | --- |
| `0x00000000` | Receiver `AXIL_ConfigReg_256_RX_0` |
| `0x00001000` | Receiver `AXIL_ConfigReg_256_RX_1` |
| `0x00002000` | PCIe `AXIL_ConfigReg_256_2` |
| `0x00003000` | PCIe `AXIL_ConfigReg_64_0` |
| `0x00004000` | PCIe `AXIL_ReadReg_64_0` |
| `0x00005000` | PCIe overflow reader |
| `0x00006000` | Receiver overflow reader |
| `0x00007000` | PCIe `AXIL_ConfigReg_64_1` |
| `0x00009000` | FIFO monitor |
| `0x0000A000` | ADC SPI |
| `0x0000B000` | Alex SPI |
| `0x0000C000` | FPGA ID / readback |
| `0x0000D000` | Wideband collect |
| `0x00010000` | Quad SPI |
| `0x00018000` | XADC |
| `0x0001C000` | TX IQ modulation BRAM |

Large AXI memory windows for stream reader/writer blocks are also assigned at:

- `0x00000000` on `M_AXI`
- `0x00040000` on `M_AXI`
- `0x00080000` on `M_AXI`

Reference:

- `FPGA/create_saturn_project.tcl`

## Protocol 2 Implications

### Discovery Reply

Current software behavior versus what the FPGA implies:

- board type `10 = SATURN`: consistent
- firmware code version from FPGA: consistent
- DDC count `4`: inconsistent with hardware
- protocol version `43`: likely a compatibility choice
- app version in discovery byte `23`: software-specific reuse, not spec-clean

### High-Priority Status

The FPGA strongly supports the Saturn additions already present in software:

- FIFO overflow status
- ADC max magnitude

This means these fields are safe to treat as real hardware telemetry.

### High-Priority Data

The missing `ClientControl` implementation at bytes `1396-1397` appears to be
a software gap. Nothing reviewed in the FPGA tree so far showed an already-wired
Saturn-specific hardware block that obviously corresponds to those bytes.

This does not prove no supporting hardware will be needed, only that the gap is
currently visible on the software side first.

### Wideband

The FPGA confirms that Saturn wideband capture is a specific 2-ADC feature and
not an open-ended multi-ADC model.

### ADC Naming

Keep this terminology mismatch in mind:

- FPGA naming: `ADC1`, `ADC2`
- Protocol 2 v4.4 naming in status bytes `39-42`: `ADC0`, `ADC1`

The byte positions can still be compatible even if the labels differ.

## What This Means For Future Work

### 1. Treat The FPGA As The Hardware Truth

When app code and FPGA disagree about capability:

- the FPGA should win unless proven otherwise

Today the clearest example is:

- DDC count = `10` in FPGA, `4` in discovery reply

### 2. Treat Discovery Changes As Compatibility Changes

The following discovery fields should not be changed casually:

- byte `12` protocol version
- byte `20` DDC count
- byte `23` beta/app-version byte

These are client-compatibility changes, not cleanup changes.

### 3. Low-Risk Spec Cleanup Is Still Possible

The safest likely protocol-alignment work remains:

- implement `ClientControl` at bytes `1396-1397`

This should be much lower risk than changing discovery version reporting.

### 4. Hardware Capability Exceeds What We Currently Expose

The FPGA already contains:

- `10` DDCs
- wideband capture
- watchdog logic
- ADC peak hold
- TX sample feedback

Future app work should assume the platform can grow into those capabilities
rather than assuming the current client-facing surface is the hard limit.

## Recommended Next Steps

### Highest Value

1. Map the FPGA AXI-Lite address blocks to `sw_projects/common/saturnregisters.c`.
2. Build a register-by-register note showing:
   - what exists in FPGA
   - what `P2_app` / `P3_app` currently use
   - what exists but is not exposed in software
3. Capture live traffic for discovery and high-priority packets to confirm the
   source review against actual wire data.

### Protocol Decision Work

1. Decide whether discovery byte `20` should stay at `4` for compatibility or
   move to `10`.
2. Decide whether discovery byte `12` should remain `43` until clients are
   explicitly validated against a `44` advertisement.
3. Decide whether byte `23` should remain a Saturn app-version field or return
   to the spec beta-flag meaning.

### Software Follow-Up

1. Implement `ClientControl` at bytes `1396-1397`.
2. Audit whether Saturn software currently uses all available FPGA telemetry:
   - receiver overflow latch
   - PCIe-side overflow latch
   - FPGA ID/readback blocks
   - wideband status

## Biggest Wins To Improve Saturn

If we want the highest-value improvements from here, the priority order is:

### 1. Make PCIe / XDMA Bring-Up Robust Across Reboot And Kernel Updates

This is the biggest real-world failure point.

When the FPGA endpoint does not enumerate on PCIe, nothing above it matters:

- no XDMA binding
- no `/dev/xdma*`
- no `P2_app` / `P3_app`

The right mental model is:

1. FPGA
2. PCIe link
3. XDMA module
4. `/dev/xdma*`
5. Saturn app
6. Protocol 2 client

This layer is covered in more detail in:

- `docs/protocol2/Saturn_LinuxDriver_XDMA_Handoff.md`

The concrete staged plan for this item is:

1. add a single `saturn-xdma-doctor` diagnostic helper with explicit failure
   stages
2. add a dedicated XDMA readiness service that gates app startup
3. keep `fix-xdma.sh` as the supported repair path, but end it with a doctor
   summary
4. add kernel post-install XDMA staging for newly installed kernels
5. surface the doctor result in Saturn Go so field support has one standard
   evidence path

Current status on this plan:

- the doctor helper and readiness helper now exist in `scripts/`
- `fix-xdma.sh` now emits doctor output after repair and on load failures
- the `p2app-control` installer now wires `p2app.service` through a dedicated
  XDMA readiness gate
- kernel post-install staging and Saturn Go surfacing remain the next steps

### 2. Complete The End-To-End Register Map

The FPGA clearly exposes more capability than the software currently documents
or cleanly surfaces.

The highest-leverage internal improvement is to map:

- FPGA AXI-Lite address blocks
- `saturnregisters.c`
- `P2_app` / `P3_app` register usage

That will make future feature work, debugging, and protocol review much faster.

### 3. Clean Up Protocol 2 Alignment Deliberately

The major known protocol mismatches are already clear:

- discovery byte `12`
- discovery byte `20`
- discovery byte `23`
- missing `ClientControl` at bytes `1396-1397`

The safest protocol-alignment improvement is:

- implement `ClientControl` first

The riskiest changes remain:

- discovery version reporting
- discovery DDC-count reporting

Those should be treated as client-compatibility changes, not cleanup.

### 4. Reduce Drift Between `P2_app` And `P3_app`

`P3_app` is already carrying more hardening and newer behavior.

Longer term, one of the biggest software-quality wins will be reducing
unnecessary drift in:

- CAT parsing
- packet framing
- register/control handling
- telemetry and diagnostics

### 5. Improve Field Diagnostics

Support effort will improve substantially if the platform can quickly separate:

- PCIe enumeration failure
- XDMA rebuild/load failure
- missing device nodes
- app startup failure
- client/protocol issues

A single supported diagnostic path that captures those layers would save a lot
of time in field support.

### 6. Expose Existing Hardware Capability Before Chasing New Transports

The FPGA already supports substantial capability beyond the most conservative
client-facing surface:

- `10` DDCs
- wideband capture
- ADC peak telemetry
- TX watchdog
- TX sample feedback

Those are higher-value near-term improvements than introducing an entirely new
transport such as native USB.

### Recommended Top Three

If only three items are pushed next, the recommended order is:

1. PCIe / XDMA reboot and update robustness
2. FPGA register-map to software-map documentation
3. Protocol cleanup with a client compatibility test matrix

## Working Conclusions To Preserve

- Saturn is a `2`-ADC, `10`-DDC FPGA design.
- Saturn wideband support is real hardware, not a placeholder.
- Saturn ADC peak and overflow status are real hardware telemetry.
- Saturn TX watchdog is enforced in hardware.
- Saturn has a TX sample feedback path.
- Saturn `P2_app` / `P3_app` are not full Protocol 2 v4.4 implementations.
- Discovery reporting is still the most compatibility-sensitive area.
- The FPGA tree supports the view that several current software limitations are
  policy/compatibility choices rather than hardware limits.
