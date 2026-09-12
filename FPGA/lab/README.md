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

Generated files under `results/vivado/` include:

- timing summary with unconstrained paths
- hierarchical utilization
- DRC, clock interaction, CDC, and methodology reports
- machine-readable setup/hold and DRC-error quality gate
- copied `.bit` artifact
- JSON manifest containing Git identity, dirty state, Vivado version, and SHA256

PROM/BIN export is scripted in `tcl/export-prom.tcl` and recorded in
`PROM_BIN_EXPORT.md`. It produces the uncompressed 32-Mbit SPIx1 multiboot
layout used by Saturn (golden at `0x00000000`, primary at `0x00980000`, with
the two timer payloads at `0x0097FC00` and `0x01300000`). Run it only after a
validated bitstream build from a Vivado 2023.1 Tcl console.

The automated quality gate rejects negative setup or hold slack and any DRC
with `Error` severity. Critical-warning and unconstrained-path reports still
require engineering review before a build is accepted.

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
- IQ modulation: 100000 ns key-down interval

Override those defaults for the original full-length regressions:

```bash
SATURN_DDC_SIM_SAMPLES=4096 SATURN_DDC_SIM_DISCARD=100 make sim-ddc
SATURN_DUC_SIM_SAMPLES=262144 SATURN_DUC_SIM_DISCARD=100000 make sim-duc
SATURN_IQMOD_KEY_HOLD_NS=20000000 make sim-iqmod
```

Set `SATURN_SIM_WAVES=1` when an interactive waveform database is useful.

If XSim stops during compile/analyze after a crashed run, set
`SATURN_SIM_FRESH=1` to archive the existing simulator cache before launch:

```bash
SATURN_SIM_FRESH=1 make sim-iqmod
```
Wave capture is disabled by default in batch smoke tests.

The checked-in IQ-modulation test project was last saved by Vivado 2021.2.
Vivado 2023.1 can leave a stale hierarchical child wrapper in the ignored
`CODEC_IQMOD_IP.ip_user_files` tree while regenerating the outer wrapper under
`.gen`. The stale child omits `S_AXI_keyerBRAM` burst/cache/lock ports, while
the fresh outer wrapper connects them, producing XSIM `cannot find port`
errors. `sim-common.tcl` now synchronizes the generated nested-BD top wrapper
into the ignored user-files location before compiling. Do not patch generated
wrapper Verilog to conceal this.

The permanent source-design cleanup is to normalize the child
`S_AXI_keyerBRAM` interface properties (`HAS_BURST`, `HAS_CACHE`, and
`HAS_LOCK`) to match the parent AXI-VIP interface (currently all `0`), then
validate/save both source BDs and regenerate child before parent output
products. The synchronization step keeps the regression reproducible while
that Vivado GUI/Tcl migration is completed.

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
