# Saturn

Saturn SDR project FPGA

Configuration artifacts and their purposes:

- `saturnfallback.bin`: complete fallback image. Do not program this during
  normal development.
- `saturn-primary-vXX-<sha>.bin`: slot-relative primary image created by the
  guarded lab export. This is the only generated BIN intended for the
  loader's default primary destination.
- `saturn-lab.bin`: complete multiboot image for archival or an external
  programmer. Do not give this file to the default primary loader path.

Version history:

V30 ADC telemetry source candidate, 13/09/2026: preserves the V29 signal path
and legacy ADC status/peak registers while adding boot-lifetime, saturating
per-ADC physical overrange episode counts, total overrange clocks, longest
continuous episode, and coherent latest/current episode duration and peak.
Durations use the exported 122.88 MHz observation clock. P2 V52 exposes the
guarded V30 bank and does not change the deferred speaker refill policy. The
V30 compatibility revision restores the V27 FIFO legacy read-to-clear boundary
and keeps almost-full transitions in extended telemetry instead of reporting
them as terminal legacy overflow. Configured FIFO capacity still asserts legacy
bit 31. This entry does not claim a qualified bitstream or hardware result until
the full build, packaging, load, and controlled/antenna validation gates pass.
The initial V30 implementation missed setup timing inside generated XDMA PCIe
logic; the checked-in V30 build flow now uses the timing strategy that closed
the same synthesized design at positive setup and hold slack and prevents PROM
export without a clean, matching successful-build manifest.

V29 RX-qualified candidate, 12/09/2026: fixes I2S receive AXI-Stream
backpressure, makes FIFO/ADC AXI-Lite read responses stable under stalls,
defines no-loss read-clear boundaries, adds coherent FIFO/ADC diagnostics,
and strengthens build/netlist/PROM safety gates. The clean Vivado 2023.1 build
and guarded primary export passed timing, DRC, CDC, methodology, and routed
telemetry gates. P2 V51 / V29 passed 30-minute controlled dummy-load and
representative antenna RX soaks on the G2. TX, PureSignal, post-TX recovery,
and flash-recovery qualification remain separate. The tracked loader artifact
is `saturn-primary-v29-984d5237.bin` (9,730,652 bytes; SHA256
`039092b1a4691c31f4a51ed1e6a5fd1bd449a7e56ba40ad41cef9dbc55a10aeb`).
No RF-performance improvement is claimed.

V28 lab candidate, 12/09/2026: first G2-loaded telemetry candidate. Basic RX
and operator-supervised TX worked, but strict 30-minute RX qualification did
not pass because microphone FIFO status observations remained unexplained.
The extended telemetry registers were not safely exposed by the host. Retain
V28 as test evidence, not as the final production release.

V27, 03/06/2026: established G2 baseline preceding the V28/V29 lab work.
The exact source/artifact identity must be taken from its archived manifest,
not inferred from this history entry.

V26. 04/01/2026. Added debug LO DDS selection to allow a Thetis debug mode to be used. No benefit for normal operation. 

V25. 07/06/2025: Added drives to select HPF in TX path for future Saturn PCB. Does not affect behaviour with current PCB.
V24. 20/05/2025: Cordic removed and replaced by original DDS. Cordic had slightly worse broadband noise performance.
V23. 15/05/2025: experimental replacement of TX DDS by CORDIC derived from that in Orion. fixed I/Q amplitude for debug use set to 0.9 ampl from 1.0
V22. 18/04/2025: minor non functional change to TX DUC block design (results in the same code being generated)
V21. 05/04/2025: replaced codec SPI interface with new IP in readiness for '3204 replacement codec device
V20, 17/02/2025: added watchdog to cancel TX if client s/w does not service FIFOs for more that 2 seconds
V19, 26/11/2024: minor update
V18, 20/11/2024: introduced wideband data collection
V17: 20/6/2024: fixed edge rate sidetone added alongside the variable edge rate RF envelope
V15, 16: experimental NOT RECOMMENDED builds investigating TX composite noise
V14, 2/5/2024: maximum CW ramp length extended to 20ms. This is still an experimental release and **not recommended** for use yet.
V13, 1/4/2024: revised TXZ chain to reduce overall TX noise level. This is an experimental release and **not recommended** for use yet.
V12, Jan 9 2024:  Revised Alex SPI core with separate TX antenna bits to address CW keyer coupling energy to RX antennas
V11, 15 Dec 2023: added TUNE input from LDG ATU; changed reset timing for ALEX SPI output
V10, Sept 30 2023:FIFO depths increased; updated FIFO monitor IP
V9, Sept 29 2023: updated project to vivado 2023.1
V8, August 23 2023: changed FPGA to use the left data path from codec line input
V7, July 29 2023: Assert PTT out if CW keyer asserts PTT
V6, July 16 2023: DAC ALC ouptut now clocked at 122.88MHz.
V5, June 2 2023: fixed DDC6 sample rate issue
V4, April 11 2023: Fixed DC spike in DDC passbans
Jan 25 2023: Added Iambic keyer.



Recommended build procedure

Vivado 2023.1 is the project authority. The older
`create_saturn_project.tcl` recreation path is retained for reference but is
not the recommended production build path. Use the checked-in project and the
fail-aware lab wrapper:

```bash
cd /mnt/c/Users/jd/Saturn
SATURN_VIVADO_JOBS=12 make -C FPGA/lab vivado-build
make -C FPGA/lab export-prom
```

The build is successful only when the quality and routed-netlist gates pass.
See `FPGA/lab/README.md`, `FPGA/lab/HANDOFF.md`, and
`FPGA/lab/PROM_BIN_EXPORT.md` for the complete commands and safety contract.

Manual project procedure (legacy reference):

1. Install Vivado 2023.1
2. Create a suitable folder: I recommend c:\\xilinxdesigns\\Saturn
3. do a git pull to copy the complete Saturn repository (https://github.com/laurencebarker/Saturn.git) into the folder (I use github desktop)
4. Run Vivado then open project file C:\\xilinxdesigns\\Saturn\\FPGA\\saturn\_project\\saturn\_project.xpr
5. Wait patiently: it will take some time before the source files are listed in the Sources window.
6. Find design source file listed as saturn\_top\_i: saturn top (saturn\_top.bd)
7. right click the file and select Create HDL wrapper. Allow Vivado to manage the file.
8. click Generate Bitstream in the project manager window.
9. Wait for implementation and inspect timing, DRC, CDC, and methodology
   reports; a generated bitstream alone is not a release verdict.
10. Use the guarded lab export for a primary BIN. Do not manually combine or
    offset an image unless following the documented recovery procedure.
