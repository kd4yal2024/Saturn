# Saturn FPGA V28 Lab — Handoff & Status

Written 2026-09-11 by Claude, grounded directly against the state of this
repo (`C:\Users\jd\Saturn`, branch `fpga-v28-lab`) rather than against a
plan document written without repo access. Use this to resume work with
Codex (or anyone else) without re-deriving or redoing what's already done.

## 1. Environment — confirmed working on this machine

- Repo: `C:\Users\jd\Saturn`, remote `kd4yal2024/Saturn` (fork of
  `laurencebarker/Saturn`).
- Branch: `fpga-v28-lab`, **0 commits ahead of `main`** — every change
  described below exists only in the working tree right now.
- Vivado 2023.1 ML Standard is installed at `C:\Xilinx\Vivado\2023.1`,
  matching the version the project itself pins (see `FPGA/README.md`
  changelog: "V9, Sept 29 2023: updated project to vivado 2023.1").
- WSL2 (`Ubuntu-24.04`) is present and starts correctly.
- Golden fallback image `FPGA/saturnfallback.bin` exists. `FPGA/README.md`:
  "DON'T program this unless you need to!"

## 2. URGENT — uncommitted work is currently unprotected

Nothing below is committed anywhere. There is no backup other than the
working tree on disk.

**Untracked** (`git status`):
- `FPGA/lab/` — the entire Phase 0 lab (Makefile, tcl/, formal/, python/,
  scripts/, rtl-tests/, vectors/)

**Modified, uncommitted**:
- `FPGA/IP/CODEC_IQMOD_IP/.../IQModCodectb.sv`
- `FPGA/IP/DDCIP/DDCIP.srcs/sim_1/imports/testbenches/RX_DDC_tb.v`
- `FPGA/IP/DUCIP/TX_DUC_tb.v`
- `FPGA/sources/testbenches/RX_DDC_tb.v`
- `FPGA/sources/testbenches/TX_DUC_tb.v`
- `FPGA/sources/verilogmodules/activitywatchdog.v` — real RTL, not just a
  testbench. Diff reviewed: it's a safe, `ifdef FORMAL`-gated addition that
  exposes an internal counter to the formal harness; it has no effect on
  normal synthesis.

**Action**: commit this before anything else. Don't `git clean`, `git
reset --hard`, or start a second clone (e.g. at `C:\xilinxdesigns\Saturn`,
which an earlier planning doc suggested) — that would abandon all of it,
since `FPGA/lab/` isn't committed anywhere to be picked up by a fresh
clone.

(Note: `.gitattributes` already normalizes line endings — the CRLF/LF
warnings git prints on these files are expected `text=auto` behavior, not
a misconfiguration. No action needed there.)

## 3. What's already built — Phase 0 lab inventory

Everything below exists now, in `FPGA/lab/`, uncommitted (see §2).

### Makefile targets (`FPGA/lab/Makefile`)

| Target | Does |
|---|---|
| `doctor` | Environment sanity check; prints golden fallback SHA256 |
| `python-env` | Creates `~/.venvs/saturn-fpga`, installs pinned numpy/scipy/matplotlib |
| `lint` / `lint-strict` | Verilator `--lint-only` over the 3 gated modules below |
| `formal` | Runs SymbiYosys `prove` + `cover` on the watchdog |
| `python-test` | Runs the DSP analyzer unit tests |
| `check` | `lint` + `formal` + `python-test` together |
| `vivado-validate` | Opens/validates the checked-in project, no synthesis |
| `vivado-build` | Full synth → impl → bitstream → reports → manifest |
| `sim-ddc` / `sim-duc` / `sim-iqmod` / `sim-all` | Batch Vivado behavioral sims |

### Lint gate (Verilator, `scripts/lint.sh`)
Covers exactly 3 modules: `activitywatchdog.v`, `FIFO_Monitor.v`,
`DDCMux.v`.

### Formal gate (SymbiYosys, `formal/watchdog.sby`)
**Only the watchdog has a formal proof right now** — `prove` + `cover`
tasks, depth 24, `smtbmc boolector` engine, against
`watchdog_formal.sv` + the real `activitywatchdog.v`. Results already
exist under `FPGA/lab/results/formal/watchdog-{prove,cover}/`. The
`rtl-tests/fifo-monitor/README.md` and `rtl-tests/ddc-mux/README.md`
explicitly defer formal + self-checking coverage for those two modules to
"Phase V28" — this is a known, intentional gap, not an oversight.

### Vivado build (`tcl/build.tcl`, `scripts/vivado-windows.sh`)
- Rejects any Vivado version other than 2023.1 unless
  `SATURN_ALLOW_UNSUPPORTED_VIVADO=1` is set.
- Uses the actual authoritative checked-in runs: **`synth_2_copy_1`** and
  **`impl_1_copy_1`** (not `synth_1`/`impl_1` — an external plan got this
  wrong).
- Backs up and restores tracked project/IP files around every Vivado
  invocation (guards `FPGA/saturn_project`, `IP/DDCIP`, `IP/DUCIP`,
  `IP/CODEC_IQMOD_IP`), because Vivado tends to touch project state just by
  opening it.
- Quality gate rejects negative setup/hold slack and any DRC `Error`.
  Critical warnings and unconstrained paths still require human review.
- Produces a JSON manifest: git SHA, dirty-tree flag, Vivado version,
  SHA256 of the `.bit`.
- **No PROM/BIN generation yet** — README states this explicitly: "The
  first known-good GUI PROM export must be recorded before that step is
  automated."

### Known open issue: IQ-modulation sim can't just be "repeated"
The checked-in IQ-mod IP test project (`IQBLKTB`) was last saved in
**Vivado 2021.2**. Upgrading its locked IP to 2023.1 breaks the AXI BRAM
controller interface — the block-design wrapper no longer elaborates. DDC
and DUC baselines pass; IQ-mod is an explicit, documented Phase 0
migration item. **Do not patch the generated wrapper to hide this** (the
README says so directly).

### Python DSP analyzers (`python/`)
`analyze_ddc.py`, `analyze_duc.py`, `compare_builds.py`, `sfdr.py`, with
regression tests in `python/tests/test_dsp.py`. Report sample count,
FFT-bin width, tone frequency, amplitude, RMS, DC, largest spur, SFDR as
JSON.

## 4. Corrections to the earlier external plan (summary)

Full detail was already given in this conversation; short version for
reference:

1. Don't clone fresh into `C:\xilinxdesigns\Saturn` — real work is at
   `C:\Users\jd\Saturn` on `fpga-v28-lab`, entirely uncommitted (§2).
2. The plan's Steps 1, 5, 6, 9, 10, 11, 13, 16 describe things that
   already exist here, often in a more correct form (clock period already
   fixed, branch already created, watchdog already formally proved, build
   script already has quality gates + correct run names).
3. The plan's example `build.tcl` uses the wrong run names
   (`synth_1`/`impl_1` vs. the real `synth_2_copy_1`/`impl_1_copy_1`).
4. The plan treats IQModtb as a routine repeat of the DDC/DUC sims; it's a
   known-broken Vivado-version migration item.
5. Everything the plan said about `load-FPGA`'s `-f` flag and the stale
   `sw_tools/load-FPGA/readme.md` was independently verified as accurate.

## 5. Adjacent context: don't invent a second telemetry scheme

V28 FPGA telemetry goals (atomic FIFO/ADC snapshots, 17-bit ADC magnitude,
event counters, min/max FIFO occupancy, watchdog diagnostics, build ID)
overlap with software telemetry that's **already shipped**:

- `project_documentation/P2_P3_ADC_Peak_Status_Message.md` (2026-03-16,
  `P2_app`/`P3_app` V45): extends the 60-byte high-priority status packet
  with ADC1/ADC2 peak amplitude (bytes 39–42), gated on **FPGA firmware
  >= 27**, sourced from "the ADC overflow register block." `P3_app` is
  archived; `P2_app` is the supported implementation. Optional runtime
  export via `/dev/shm/saturn_p23_adc_peak_telemetry.json`, opt-in, capped
  at 1 Hz.
- `sw_projects/P2_app/TX_DUC_FLOW_CONTROL_PLAN.md` (2026-03-23): a
  two-stage ingress/writer thread model that already actively manages TX
  DUC FIFO occupancy against a target reserve band and drops stale frames
  — i.e., the software side already has real FIFO-occupancy telemetry and
  policy, independent of any new FPGA counters.

**Implication for V28**: new FPGA-side counters (min/max FIFO occupancy,
watchdog diagnostics, build ID) should extend the existing ADC-overflow
register block / status-packet convention that `P2_app` already reads,
not introduce a second, parallel telemetry path.

## 6. Prioritized path back on track

1. **Commit or explicitly checkpoint current state.** Nothing here is
   backed up yet.
2. Run `make doctor` in WSL — confirm the lab environment is actually
   sane on this machine and record the fallback SHA256 somewhere outside
   the repo.
3. Run `make check` (lint + formal + python-test) — confirm the existing
   gates actually pass here; they're built but it's unclear from repo
   state alone whether they've been exercised end-to-end on this machine.
4. Run `make vivado-validate`, then `make vivado-build` — get a baseline
   V27 timing/utilization/DRC report and manifest. This satisfies Phase 0
   acceptance criterion 4, which per the README is not yet done.
5. Decide how to handle the IQModtb Vivado-version migration issue before
   declaring Phase 0 sim baselines complete (fix, or explicitly scope out
   with a tracked follow-up).
6. Do the PROM/BIN GUI export once, record the exact settings, then
   automate it in Tcl — explicitly flagged as the last unfinished Phase 0
   automation piece.
7. Only after all 5 Phase 0 acceptance criteria in `FPGA/lab/README.md`
   are met, start V28 telemetry work — cross-check the wire/register
   format against §5 so `P2_app` doesn't need a second protocol.
8. Expand formal coverage from watchdog-only to `FIFO_Monitor.v` and
   `DDCMux.v`, as already earmarked in their `rtl-tests/*/README.md`
   files.

## 7. Open questions worth resolving with the user before continuing

- Has `make doctor` / `make check` actually been run to completion on
  this machine yet, or is the scaffold built but unexercised?
- Is the golden fallback SHA256 recorded anywhere outside this repo yet?
- Is a G2 unit currently connected for the CM4/XDMA programming and
  hardware-in-the-loop steps, or is work still simulation/synthesis-only
  for now?
