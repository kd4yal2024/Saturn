# Saturn FPGA Lab — Phase 0

This directory is the repeatable verification and build layer around the
checked-in Saturn Vivado 2023.1 projects. It does not replace Vivado for the
complete Artix-7 image; it provides fast RTL checks, formal proofs, numerical
DSP analysis, batch simulations, implementation reports, and build manifests.

## Safety boundary

- `FPGA/saturnfallback.bin` is the golden fallback image. Never overwrite it
  during normal development.
- Development images target the primary flash slot. Do not pass `-f` to
  `load-FPGA` during normal development.
- Phase 0 does not program hardware or enable TX.
- Do not use generic file-dump tools (`od`, `dd`, `head`, or similar) on
  `/dev/xdma*_user`. Their skip/read behavior is not a positioned, single-MMIO
  transaction contract and can traverse side-effect registers or wedge the
  XDMA path. Hardware register diagnostics must go through a reviewed Saturn
  accessor while the owning service is quiesced or explicitly coordinating
  access.

The lab doctor prints the fallback image SHA256 so it can be recorded outside
the repository:

```bash
cd FPGA/lab
make doctor
```

## Layout

```text
lab/
├── Makefile                 common entry points
├── formal/                  SymbiYosys proofs
├── python/                  DDC/DUC measurements and comparisons
├── results/                 ignored generated reports and captures
├── rtl-tests/               module-level regression homes
├── scripts/                 WSL checks, lint, formal, Python setup
├── tcl/                     Vivado build, report, and simulation drivers
└── vectors/                 checked-in RX/TX stimulus vectors
```

## WSL tool environment

Install/extract the official Linux x64 OSS CAD Suite under
`$HOME/opt/oss-cad-suite`, then activate it before lab commands:

```bash
source "$HOME/opt/oss-cad-suite/environment"
cd /mnt/c/Users/jd/Saturn/FPGA/lab
make python-env
make doctor
```

`make python-env` creates `$HOME/.venvs/saturn-fpga` on WSL's native filesystem
and installs the pinned-range NumPy, SciPy, and Matplotlib dependencies. Set
`SATURN_VENV` to use a different path. Generated results are ignored by Git.

## Fast verification

```bash
source "$HOME/opt/oss-cad-suite/environment"
cd /mnt/c/Users/jd/Saturn/FPGA/lab

make lint          # warnings reported; syntax/semantic errors fail
make lint-strict   # warnings also fail
make formal        # watchdog safety proof and expiry cover trace
make python-test   # numerical measurement regression
make check         # lint + formal + Python + V29/V30 telemetry regression
```

The Phase 0 lint gate covers `activitywatchdog.v`, `FIFO_Monitor.v`, the ADC
overflow reader, `DDCMux.v`, and `I2S_rcv.v`. `make check` also runs the
self-checking V29/V30 FIFO/ADC/I2S telemetry regression
(`make telemetry-test`).

## Hardware RX soak profiles

`scripts/rx-soak.py` captures a read-only, version-pinned P2/FPGA RX soak from the
Saturn Go `/p23_perf` endpoint. It does not read raw XDMA registers, clear FPGA
accumulators, restart services, or change RX/TX state. The operator must place
the radio in the required RX-only workload and declare every expected DDC
before starting it. Run the collector on the Saturn node so its default
loopback endpoint reaches Saturn Go. For example, deploy the checked-in copy
from the workstation with:

```bash
scp FPGA/lab/scripts/rx-soak.py \
  pi@192.168.0.139:/home/pi/saturn-v29-validation/rx-soak.py
```

Use `controlled` with a known stable 50-ohm termination or dummy load. Any ADC
overflow remains a hard failure in this profile:

```bash
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
python3 /home/pi/saturn-v29-validation/rx-soak.py \
  --profile controlled \
  --expect-ddc 2:384 --expect-ddc 3:384 \
  --output "/home/pi/saturn-v29-validation/v29-p2v51-rx-controlled-${stamp}"
```

Use `antenna` for representative field operation. ADC overflow reports and
sampled channel/peak context are preserved in the summary but do not assign
speaker-test causality or stop the run. Speaker, transport, network, FPGA
snapshot, identity, routing, and RX-only gates remain strict:

```bash
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
python3 /home/pi/saturn-v29-validation/rx-soak.py \
  --profile antenna \
  --expect-ddc 2:384 --expect-ddc 3:384 \
  --output "/home/pi/saturn-v29-validation/v29-p2v51-rx-antenna-${stamp}"
```

The default duration is 1800 seconds at five-second intervals. `--duration`
may be shortened for a smoke test. The interval may be lengthened but is
enforced at a minimum of five seconds so consecutive samples can reliably see
the published V29 generation advance. The output directory must be new or
empty; the collector refuses existing contents before writing any status.
Each completed artifact contains a copy of the collector and a `SHA256SUMS`
manifest over the defined output files.

The expected receiver set is explicit. It defaults to the original single
`DDC2` 384 ksps non-interleaved workload. The examples above declare the
current dual-receiver Thetis workload. Repeat `--expect-ddc` for any
intentional multi-receiver run; any later routing drift still fails closed:

```bash
--expect-ddc 2:384 --expect-ddc 3:384
```

The historical defaults remain P2 V51 / FPGA V29 / BIT `09122026`. For a V30
candidate, pin all three identity fields explicitly; V30 runs also require the
guarded physical-episode telemetry and a stable, advancing coherent snapshot:

```bash
--expect-p2-version 52 --expect-fpga-version 30 --expect-bit-date 09132026
```

An optional third field accepts `interleaved` or `noninterleaved`, for example
`--expect-ddc 2:384:interleaved`.

The existing `adc_overflow_events` value counts nonzero status reports, not
independent physical clip episodes. Antenna-profile ADC entries are therefore
classified as sampled report clusters. Do not infer episode count or duration
from them without finer-grained runtime telemetry.

## Vivado 2023.1 batch build

From WSL, the supplied launcher uses the Windows installation at
`C:\Xilinx\Vivado\2023.1` (override with `VIVADO_HOME_WINDOWS`):

```bash
cd /mnt/c/Users/jd/Saturn/FPGA/lab
make vivado-validate
make vivado-build
make vivado-cdc
make export-prom
```

The first command opens and validates the checked-in project without running
synthesis. The launcher preserves and restores all tracked project files,
while Vivado's ignored generated wrapper, caches, and run products remain
available for later commands. You can also run the build directly in a Windows Vivado
2023.1 command prompt from the repository root:

```powershell
vivado -mode batch -nolog -nojournal -source FPGA/lab/tcl/build.tcl
```

The script deliberately rejects other Vivado versions unless
`SATURN_ALLOW_UNSUPPORTED_VIVADO=1` is set. It uses the checked-in authoritative
runs:

- synthesis: `synth_2_copy_1`
- implementation: `impl_1_copy_1`

The WSL launcher passes the commit, branch, and dirty state explicitly to
Windows Vivado. This keeps artifact provenance intact for linked Git worktrees,
whose `.git` file can contain a WSL-only path. Keep Windows-side checkout or
worktree paths short (for example, `C:\V30`); Vivado 2023.1 rejects generated
IP paths longer than 260 bytes.

Set `SATURN_VIVADO_JOBS` to change the default of eight jobs. Set
`SATURN_SKIP_RESET=1` only when intentionally resuming existing run products.
When `SATURN_REUSE_SYNTH=1` is selected, the build retains the compile order
captured by that completed synthesis checkpoint instead of asking an older
project to refresh it during the reuse-only pass. Combining it with
`SATURN_SKIP_RESET=1` also reuses a completed implementation rather than
relaunching a run that has no pending steps.

The V30 build applies its qualified timing-closure strategy by default:
`ExtraNetDelay_high` placement plus `AggressiveExplore` pre-route physical
optimization, routing, and post-route physical optimization. The selected
values are written to `results/vivado/implementation-strategy.txt` and printed
in the console log. `SATURN_PLACE_DIRECTIVE`, `SATURN_PHYSOPT_DIRECTIVE`,
`SATURN_ROUTE_DIRECTIVE`, and `SATURN_POST_ROUTE_PHYSOPT_DIRECTIVE` are retained
only for controlled experiments; a release build should use the checked-in
defaults.

`make vivado-validate` skips compile-order refresh by default because Vivado
can hang while migrating older projects. Set
`SATURN_VALIDATE_UPDATE_COMPILE_ORDER=1` to force that refresh for diagnosis.

Generated files under `results/vivado/` include:

- timing summary with unconstrained paths
- hierarchical utilization
- DRC, clock interaction, methodology, and raw/reviewed/waived CDC reports
- machine-readable setup/hold, DRC, CDC, and methodology quality gate
- exact implementation-directive record
- copied `.bit` artifact
- JSON manifest containing Git identity, dirty state, Vivado version, and SHA256

PROM/BIN export is scripted in `tcl/export-prom.tcl` and recorded in
`PROM_BIN_EXPORT.md`. It produces both a slot-relative
`saturn-primary-v30-<sha>.bin` for the default `load-FPGA` primary destination and
an uncompressed 32-Mbit `saturn-lab.bin` complete multiboot image (golden at
`0x00000000`, primary at `0x00980000`, timers at `0x0097FC00` and
`0x01300000`). Never pass the complete image to `load-FPGA`; the loader adds
the primary offset itself. Run export only after a validated Vivado 2023.1
bitstream build.

PROM export fails closed unless `manifest.json` proves that the selected V30
bitstream came from the current clean commit, passed the build gates, and still
matches its recorded SHA256. Starting a rebuild moves the preceding manifest to
`manifest.previous.json`, preventing a failed same-SHA rebuild from authorizing
an older bitstream.

The automated quality gate rejects negative setup or hold slack, DRC errors or
critical warnings, unwaived CDC Critical findings, and methodology Critical
Warnings. CDC
waivers are endpoint-bound in `tcl/cdc-waivers.tcl`; hierarchy drift or a new
destination therefore fails closed. The reviewed contracts cover Xilinx XDMA
internals, diagnostic clock-monitor sampling, static PCB revision straps,
protocol-timed SPI MISO sampling, and XPM FIFO asynchronous-reset assertion
with synchronized release. Critical-warning and unconstrained-path reports
still require engineering review before a build is accepted.

## Vivado simulations

The scripts use the separate checked-in IP test projects that actually contain
the three baseline benches:

```powershell
vivado -mode batch -nolog -nojournal -source FPGA/lab/tcl/sim-ddc.tcl
vivado -mode batch -nolog -nojournal -source FPGA/lab/tcl/sim-duc.tcl
vivado -mode batch -nolog -nojournal -source FPGA/lab/tcl/sim-iqmod.tcl
```

Or run all three with `FPGA/lab/tcl/test.tcl`. The drivers generate missing IP
simulation products, require the testbench to reach `$finish`, and verify the
expected capture length. Fast smoke-test defaults are:

- DDC: 256 captured samples after 64 discarded samples
- DUC: 4096 captured samples after 16384 discarded samples
- IQ modulation: 16 accepted CW samples during a 100000 ns key-down interval;
  Q must be zero and I must follow the programmed monotonic 8192-count ramp

Override those defaults for the original full-length regressions:

```bash
SATURN_DDC_SIM_SAMPLES=4096 SATURN_DDC_SIM_DISCARD=100 make sim-ddc
SATURN_DUC_SIM_SAMPLES=262144 SATURN_DUC_SIM_DISCARD=100000 make sim-duc
SATURN_IQMOD_KEY_HOLD_NS=20000000 make sim-iqmod
SATURN_IQMOD_SIM_SAMPLES=64 make sim-iqmod
```

Set `SATURN_SIM_WAVES=1` when an interactive waveform database is useful.

If XSim stops during compile/analyze after a crashed run, set
`SATURN_SIM_FRESH=1` to archive the existing simulator cache from WSL before
Windows Vivado starts:

```bash
SATURN_SIM_FRESH=1 make sim-iqmod
```
If Windows still has the directory open, the launcher stops with an explicit
message; close the matching `vivado.exe`, `xvlog.exe`, `xelab.exe`, or
`xsim.exe` process and retry.
Wave capture is disabled by default in batch smoke tests.

The IQ-modulation source BDs and catalog IP are persistently migrated to
Vivado 2023.1. `make vivado-migrate-iqmod` is the guarded, write-back migration
target for future catalog changes. `make vivado-migrate-telemetry` persists the
V29 FIFO `almost_full` wiring in an existing top-level project. Ordinary Vivado
commands restore incidental project-file churn. `sim-common.tcl` synchronizes nested-BD
simulation wrappers into the ignored user-files tree before compile because
Vivado keeps both generated locations.

The DDC and canonical DUC testbench clocks are 122.88 MHz
(`8.138020833 ns`), matching Saturn hardware.

## DSP analysis

After simulation, analyze the captured text files:

```bash
$HOME/.venvs/saturn-fpga/bin/python python/analyze_ddc.py results/simulation/ddc/ddcdata.txt \
  --sample-rate 1536000 --output results/ddc-v27.json

$HOME/.venvs/saturn-fpga/bin/python python/analyze_duc.py results/simulation/duc/ducoffbindata.txt \
  --sample-rate 122880000 --output results/duc-v27.json

$HOME/.venvs/saturn-fpga/bin/python python/compare_builds.py \
  results/ddc-v27.json results/ddc-v28.json
```

The analyzers report sample count, FFT-bin width, tone frequency, amplitude,
RMS, DC, largest spur, and SFDR as machine-readable JSON.

## Phase 0 acceptance

Phase 0 is complete when:

1. `make doctor` reports `LAB_READY`.
2. `make check` passes.
3. all three Vivado simulations complete and their baseline results are saved.
4. the Vivado batch build completes with non-negative setup/hold slack, no
   unexplained unconstrained paths, and no unexplained critical DRC/CDC issues.
5. the known-good PROM/BIN export settings and primary-slot CM4 programming are
   exercised and recorded without using `load-FPGA -f`.
