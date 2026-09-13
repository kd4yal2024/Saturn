# Saturn FPGA V29 lab handoff

Updated 2026-09-12. This file describes the V29 source candidate on branch
`fpga-v29-lab`. Historical V27/V28 investigation details remain available in
Git history; they are not current release claims.

## Current state

- V28 is the image currently exercised on Jerry's G2. Runtime inventory
  reported firmware 28, BIT date `09122026`, all clocks present, and fallback
  inactive.
- V28 passed basic RX/TX operation, but its 30-minute RX qualification failed
  the strict gate after two microphone FIFO status observations. Host evidence
  could not prove two distinct overflows or count lost microphone frames.
- V29 is a source candidate only. It has not yet been synthesized, packaged,
  loaded, or qualified on hardware.
- Vivado 2023.1 remains the required full-design build and vendor-IP
  simulation authority.

## V29 changes

- `I2S_rcv.v` now obeys AXI-Stream backpressure: `TDATA` and `TVALID` remain
  stable until the pending word is accepted. A physical I2S frame arriving
  during a prolonged stall cannot overwrite the older unaccepted word.
- FIFO and ADC AXI-Lite readers latch one response per transaction and hold it
  stable while `RREADY` is low.
- ADC status sampling clears only at the accepted status-address boundary;
  peak-only and extended telemetry reads do not consume live overflow state.
- FIFO legacy read-to-clear logic retains a condition present on the response
  transfer edge. Snapshot/event clear retains a simultaneous boundary event.
- ADC magnitude conversion is exhaustively checked for all 65,536 signed
  input codes, including `-32768`.
- Four top-level AXIS FIFOs expose `almost_full` to `FIFO_Monitor`. The
  extended registers provide coherent occupancy snapshots, min/max occupancy,
  saturating state-transition counters, and build marker `0x56323900`
  (`V29\0`). These counters are diagnostic boundary transitions, not proven
  sample-loss counts.
- The build gate checks setup/hold timing, DRC, methodology, reviewed CDC, and
  the presence of telemetry logic in the routed netlist. Firmware identity is
  fixed at 29 and stale module-reference checkpoints are invalidated for a
  non-reuse build.
- PROM export emits a slot-relative primary BIN and rejects an image that
  would cross the protected `0x01300000` timer barrier. The CM4 loader has the
  same primary-size guard.

## Verified offline

Run these from `/mnt/c/Users/jd/Saturn`:

```bash
make -C FPGA/lab telemetry-test
make -C FPGA/lab check
```

The telemetry regression covers FIFO/ADC snapshot behavior, stalled AXI read
responses, read-clear boundary events, the exhaustive ADC magnitude sweep,
and I2S receive backpressure. `make check` also includes Verilator lint,
watchdog formal prove/cover, and the Python DSP analyzer tests.

The routed-netlist audit previously passed against the V28 checkpoint. That
proves the persisted telemetry hierarchy was present in that checkpoint; it
does not replace a fresh V29 implementation run.

## Required V29 build and package

After committing the source, run one non-reused build so the artifact name and
manifest contain the V29 commit SHA:

```bash
cd /mnt/c/Users/jd/Saturn
unset SATURN_REUSE_SYNTH SATURN_SKIP_RESET
set -o pipefail
SATURN_VIVADO_JOBS=12 make -C FPGA/lab vivado-build 2>&1 |
  tee FPGA/lab/results/vivado/build-v29-console.log
```

Do not call the build complete unless it prints both
`SATURN_LAB_TELEMETRY_NETLIST_OK` and `SATURN_LAB_BUILD_OK`, and
`quality-gate.txt` contains nonnegative WNS/WHS with zero DRC, CDC, and
methodology critical findings.

Then export the guarded primary payload:

```bash
make -C FPGA/lab export-prom
```

Only `saturn-primary-v29-<sha>.bin` is suitable for the loader's default
primary destination. `saturn-lab.bin` is a complete multiboot image and must
not be passed to the normal loader path.

## Hardware boundary

- No automatic lab command flashes, reboots, performs MMIO writes, or keys TX.
- Never use generic file tools such as `od`, `dd`, or `head` on
  `/dev/xdma*_user`; a prior direct read disrupted the G2 and caused an
  unclean reboot.
- Register access requires a reviewed accessor and coordination with the
  running P2 service. V29 extended telemetry is not yet exposed by the host.
- Loading V29 requires an operator-approved artifact hash, a known-good
  primary backup, protected fallback confirmation, and a documented recovery
  path for a valid image that breaks PCIe/XDMA.
- Hardware qualification begins RX-only. TX, PureSignal, flash operations,
  and recovery tests remain separate operator-approved steps.

## Release verdict

V29 is ready for a fresh production-gated build after the source commit. It is
not yet a production firmware release: implementation evidence, a guarded
primary BIN, host exposure of the telemetry contract, and G2 qualification are
still pending.
