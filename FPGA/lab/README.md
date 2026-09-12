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
make check         # lint + formal + Python regression
```

The Phase 0 lint gate covers `activitywatchdog.v`, `FIFO_Monitor.v`, and
`DDCMux.v`. Formal coverage begins with the TX watchdog and expands alongside
V28 telemetry work.

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

Set `SATURN_VIVADO_JOBS` to change the default of eight jobs. Set
`SATURN_SKIP_RESET=1` only when intentionally resuming existing run products.

`make vivado-validate` skips compile-order refresh by default because Vivado
can hang while migrating older projects. Set
`SATURN_VALIDATE_UPDATE_COMPILE_ORDER=1` to force that refresh for diagnosis.

Generated files under `results/vivado/` include:

- timing summary with unconstrained paths
- hierarchical utilization
- DRC, clock interaction, methodology, and raw/reviewed/waived CDC reports
- machine-readable setup/hold, DRC-error, and unwaived-Critical-CDC quality gate
- copied `.bit` artifact
- JSON manifest containing Git identity, dirty state, Vivado version, and SHA256

PROM/BIN export is scripted in `tcl/export-prom.tcl` and recorded in
`PROM_BIN_EXPORT.md`. It produces the uncompressed 32-Mbit SPIx1 multiboot
layout used by Saturn (golden at `0x00000000`, primary at `0x00980000`, with
the two timer payloads at `0x0097FC00` and `0x01300000`). Run it only after a
validated bitstream build from a Vivado 2023.1 Tcl console.

The automated quality gate rejects negative setup or hold slack, any DRC with
`Error` severity, and any unwaived CDC finding with `Critical` severity. CDC
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
target for intentional future catalog changes; ordinary Vivado commands still
restore incidental project-file churn. `sim-common.tcl` synchronizes nested-BD
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
