# Saturn FPGA V29 lab handoff

Updated 2026-09-13. This file describes the V29 source and RX hardware
qualification state on branch `fpga-v29-lab`. Historical V27/V28 investigation
details remain available in Git history; they are not current release claims.

## V30 development branch

Branch `fpga-v30-adc-telemetry` advances the source identity to FPGA V30 and
P2 V52 without changing the V29 signal path or the deferred speaker refill
policy. It adds physical ADC overrange episode, high-clock duration, and
associated peak telemetry so lightning/static bursts can be distinguished
from repeated software observation of one sustained condition. The V30
compatibility candidate also restores the V27 legacy FIFO read-to-clear
boundary and reserves legacy bit 31 for configured-capacity observations;
almost-full transitions remain in the V29 extended event counters. No V30
build, package, G2 load, or hardware qualification is claimed here until those
gates are completed and recorded separately.

The first V30 default-strategy implementation completed routing but failed the
setup gate at WNS `-0.373 ns`; the worst path was entirely inside the generated
XDMA PCIe receive-valid filter, with routing contributing 76% of its data-path
delay. Reimplementing the same synthesized V30 netlist with
`ExtraNetDelay_high` placement and `AggressiveExplore` physical optimization
and routing passed at WNS `+0.109 ns`, WHS `+0.049 ns`, with zero DRC, unwaived
CDC, or methodology critical findings. Those directives are now the V30 build
defaults. PROM export additionally requires a clean, current, hash-matching
successful-build manifest so a failed rebuild cannot package stale output.

## Current state

- V29 is running on Jerry's G2 with P2 V51. Runtime inventory reports firmware
  29, BIT date `09122026`, all clocks present, fallback inactive, and the V29
  build marker `0x56323900`.
- A controlled 30-minute RX soak with the ADC1 antenna path moved to a
  1500-watt 50-ohm dummy load passed all strict gates: 361 samples, zero V29
  snapshot timeouts, no ADC overflow, and no new speaker underrun or
  queue-ready event.
- A representative 30-minute antenna RX soak during active low-band static
  also passed all strict gates: 361 samples, zero V29 snapshot timeouts, and
  no new speaker underrun, queue-ready, queue-empty, gap, stall, transport, or
  network error. Seven sampled ADC report clusters contained eight reports;
  those environmental observations were non-gating by profile.
- Two earlier antenna-connected attempts stopped only on ADC overflow. A live
  follow-up captured ADC1 at positive full scale (`32767`) during a storm and
  showed one brief clip burst producing multiple `adc_overflow_events`
  reports. Those runs remain valid environmental/ADC evidence, but do not
  implicate the V51 speaker instrumentation.
- `scripts/rx-soak.py` now defines separate `controlled` and `antenna`
  profiles. Controlled input keeps ADC overflow as a hard gate; antenna use
  records sampled ADC report clusters without assigning speaker-test
  causality. All speaker, transport, network, identity, routing, and V29
  snapshot gates remain strict.
- The queue-ready refill-policy reproduction remains unconfirmed, so no V52
  speaker change has been implemented.
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

## Verified on RX hardware

- Controlled dummy-load soak:
  `/home/pi/saturn-v29-validation/v29-p2v51-pacing-rx-30m-dummy-20260913T144252Z`
  (`PASS`, 1800.078 seconds, 361 samples; all artifact hashes verified).
- Antenna attempt 1:
  `/home/pi/saturn-v29-validation/v29-p2v51-pacing-rx-30m-20260913T141128Z`
  (speaker gates clean; stopped after one ADC overflow report).
- Antenna attempt 2:
  `/home/pi/saturn-v29-validation/v29-p2v51-pacing-rx-30m-20260913T141304Z`
  (speaker gates clean; stopped after an ADC report burst of 24).
- Final controlled-profile dual-DDC smoke:
  `/home/pi/saturn-v29-validation/v29-p2v51-rx-controlled-dual-smoke-20260913T152434Z`
  (`PASS`, 10.068 seconds; all artifact hashes verified).
- Final antenna-profile execution smoke, still on the dummy load:
  `/home/pi/saturn-v29-validation/v29-p2v51-rx-antenna-profile-smoke-20260913T152852Z`
  (`PASS`, 10.067 seconds; all artifact hashes verified).
- Full antenna-profile dual-DDC soak:
  `/home/pi/saturn-v29-validation/v29-p2v51-rx-antenna-30m-20260913T154630Z`
  (`PASS`, 1800.067 seconds, 361 samples; ADC reports `62 -> 70`, no
  strict-gate failures, and all artifact hashes verified).

The checked-in collector is deployed read-only at
`/home/pi/saturn-v29-validation/rx-soak.py`. Its SHA256 is
`b2905797a963086a749fcc1b923edcfedd77beae73155be3dddc303d3c4610c6`.
The live Thetis workload at the final smoke had DDC2 and DDC3 enabled at 384
ksps, so both were declared explicitly and frozen by the collector.

The ADC report counter is not a physical edge or episode counter. P2 polls the
read-to-clear status during the RX reporting wait and immediately emits a
high-priority report after a hit. A sustained or closely spaced ADC overrange
condition can therefore produce multiple counter increments. Preserve the
counter for compatibility and immediate Thetis indication, but use the
antenna soak profile when qualifying unrelated speaker behavior.

## V29 rebuild and package procedure

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
  running P2 service. Use the host-exposed `/p23_perf` V29 telemetry for
  read-only qualification instead of direct register access.
- Reloading V29 requires an operator-approved artifact hash, a known-good
  primary backup, protected fallback confirmation, and a documented recovery
  path for a valid image that breaks PCIe/XDMA.
- Hardware qualification begins RX-only. TX, PureSignal, flash operations,
  and recovery tests remain separate operator-approved steps.

## Release verdict

V29 and P2 V51 have passed both controlled dummy-load and representative
antenna-connected 30-minute RX qualification. The antenna run included eight
ADC reports without any correlated speaker or transport failure. TX,
PureSignal, post-TX RX recovery, flash recovery, and broader mixed-use
qualification remain separate operator-approved steps, so V29 is not yet a
fully qualified production release. The queue-ready refill-policy
reproduction remains unconfirmed and does not justify a V52 speaker change.
