# Saturn FPGA V28 Lab — Handoff & Status

Written 2026-09-11 by Claude, grounded directly against the state of this
repo (`C:\Users\jd\Saturn`, branch `fpga-v28-lab`) rather than against a
plan document written without repo access. Use this to resume work with
Codex (or anyone else) without re-deriving or redoing what's already done.

## 1. Environment — confirmed working on this machine

- Repo: `C:\Users\jd\Saturn`, remote `kd4yal2024/Saturn` (fork of
  `laurencebarker/Saturn`).
- Branch: `fpga-v28-lab`, with safety checkpoint commits `114a00a` and
  `f40132d`. The cosmetic Vivado `.xpr` drift was reverted; current lab
  changes are documented below.
- Vivado 2023.1 ML Standard is installed at `C:\Xilinx\Vivado\2023.1`,
  matching the version the project itself pins (see `FPGA/README.md`
  changelog: "V9, Sept 29 2023: updated project to vivado 2023.1").
- **WSL distro matters**: the lab toolchain (OSS CAD Suite, the
  `saturn-fpga` Python venv, the locally built GNU Make, and the
  `.bashrc` PATH exports for all of it) is installed in the **`Ubuntu`**
  WSL distro. `Ubuntu-24.04` is the **default** distro
  (`wsl -l -v` marks it with `*`) and has none of this installed — plain
  `wsl` drops you there and `make doctor` will report `LAB_NOT_READY`.
  Always use `wsl -d Ubuntu` for lab work, or run
  `wsl --set-default Ubuntu` once to change the default.
- `.bashrc`'s PATH/tool exports only take effect in a genuinely
  **interactive** shell (stock Ubuntu `.bashrc` has an
  `case $- in *i*) ;; *) return;; esac` guard). `source ~/.bashrc` from a
  non-interactive `bash -c`/script will silently no-op; a real typed
  terminal session is fine.
- Golden fallback image `FPGA/saturnfallback.bin` exists. `FPGA/README.md`:
  "DON'T program this unless you need to!"

## 2. Safety checkpoint

The Phase 0 lab and the reviewed RTL/testbench changes are protected by
commits `114a00a` and `f40132d`.

**Current worktree state (verified 2026-09-11):** the cosmetic Vivado
`FPGA/saturn_project/saturn_project.xpr` rewrite (path and source ordering)
was reverted to the checked-in canonical project. The IQ-mod simulation fix
is in `FPGA/lab/tcl/sim-common.tcl` and is not generated HDL.

The IQ-mod failure is a stale ignored child wrapper: the simulation fileset
uses `CODEC_IQMOD_IP.ip_user_files/bd/IQ_Modulation_Select_inst_0/sim`,
which omits AXI burst/cache/lock ports, while Vivado 2023.1 regenerates the
outer wrapper and fresh child under `.gen` with those ports. The lab now
synchronizes that generated nested-BD top wrapper before simulation. The
long-term source-BD cleanup is to normalize the child `S_AXI_keyerBRAM`
`HAS_BURST`, `HAS_CACHE`, and `HAS_LOCK` properties to the parent AXI-VIP
values (all `0`) and regenerate child before parent products.

The checkpoint contains:

- `FPGA/lab/` — the entire Phase 0 lab (Makefile, tcl/, formal/, python/,
  scripts/, rtl-tests/, vectors/)
- The reviewed RTL/testbench updates listed below.

The original handoff inventory was written before the checkpoint and listed
these as uncommitted:

**Previously untracked**:
- `FPGA/lab/` — the entire Phase 0 lab (Makefile, tcl/, formal/, python/,
  scripts/, rtl-tests/, vectors/)

**Previously modified**:
- `FPGA/IP/CODEC_IQMOD_IP/.../IQModCodectb.sv`
- `FPGA/IP/DDCIP/DDCIP.srcs/sim_1/imports/testbenches/RX_DDC_tb.v`
- `FPGA/IP/DUCIP/TX_DUC_tb.v`
- `FPGA/sources/testbenches/RX_DDC_tb.v`
- `FPGA/sources/testbenches/TX_DUC_tb.v`
- `FPGA/sources/verilogmodules/activitywatchdog.v` — real RTL, not just a
  testbench. Diff reviewed: it's a safe, `ifdef FORMAL`-gated addition that
  exposes an internal counter to the formal harness; it has no effect on
  normal synthesis.

Do not `git clean`, `git reset --hard`, or start a second clone (e.g. at
`C:\xilinxdesigns\Saturn`, which an earlier planning doc suggested). The
active work is in `C:\Users\jd\Saturn`.

(Note: `.gitattributes` already normalizes line endings — the CRLF/LF
warnings git prints on these files are expected `text=auto` behavior, not
a misconfiguration. No action needed there.)

## 3. What's already built — Phase 0 lab inventory

Everything below exists now, in `FPGA/lab/`, and is protected by the
checkpoint in §2.

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
- PROM/BIN generation is now scripted in `tcl/export-prom.tcl` and recorded
  in `PROM_BIN_EXPORT.md`, using the authoritative uncompressed 32-Mbit
  SPIx1 multiboot layout. The script still needs to be executed once from a
  working Vivado 2023.1 Windows shell to capture the new lab image's BIN/PRM
  hashes.

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
   `C:\Users\jd\Saturn` on `fpga-v28-lab`, protected by the checkpoint (§2).
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

## 5.5 Build/functional signoff status — NOT complete (verified 2026-09-11)

The build/export mechanics work, but this is a **V27 baseline build
artifact, not a signed-off release**, and does not yet satisfy Phase 0
acceptance criterion 4 in `FPGA/lab/README.md` ("no unexplained... critical
DRC/CDC issues"). Every item below was independently reproduced, not just
read from a report.

| Area | Result | Status |
|---|---|---|
| Synthesis/implementation | 100% complete | Pass |
| Setup timing | WNS +0.176 ns | Pass |
| Hold timing | WHS +0.049 ns | Pass |
| Routing | 0 failed/unrouted nets | Pass |
| DRC | 0 errors, 0 critical warnings | Pass |
| PROM export | Valid 32-Mbit layout | Pass |
| Functional simulation | All 3 reach `$finish` | **Not sufficient for signoff** |
| CDC | 23 Critical findings | **Review/fix required** |
| I/O timing coverage | 11 inputs, 32 outputs missing delay | **Review required** |

**1. IQ testbench had a real wiring bug**, not just a migration
side-effect. `FPGA/IP/CODEC_IQMOD_IP/CODEC_IQMOD_IP.srcs/sim_1/imports/CODEC_IQMOD_IP/IQModCodectb.sv`
instantiates the UUT with:
```
.TXIQIn_tvalid       (TXIQIn_tdata),
```
— a 64-bit data bus wired into a 1-bit valid input (verified by direct
read of the file). The same mistake existed in the canonical copy. The
connection is now corrected in all four maintained testbench copies and the
control signals are initialized. `make sim-iqmod` reaches `$finish` after
the migration; it remains a smoke check because it does not yet assert IQ
output values or compare DSP metrics.

**2. The simulation gate is a smoke gate only.**
`FPGA/lab/tcl/sim-common.tcl` (`saturn_lab::run_simulation`, around line
128) checks for fatal errors, `$finish`, and expected capture length —
confirmed by direct read. It does not validate output values or compare
DSP metrics. Confirmed by rerunning `python/analyze_ddc.py` myself:
DDC fundamental exactly `-300000.0` Hz, SFDR `54.07` dBc — plausible. But
the first DUC smoke capture was suspicious: its 16,384-sample discard left
the 4,096 captured samples at `32760`–`32775` (offset-binary, mid-scale
`32768`). After fixing launcher environment forwarding and rerunning with
the intended 100,000-sample discard, the capture ranged `4156`–`61378`, with
a 7.11 MHz fundamental at `-1.66 dBFS` and `60.87 dBc` SFDR. The flat result
was therefore an insufficient warm-up artifact, not current evidence of a
DUC failure. A 262,144-sample baseline remains useful for formal DSP
regression, but is no longer a release blocker.

**3. CDC is not signed off.** `results/vivado/cdc.txt` header table
(independently confirmed):
`CDC-1` 16 Critical, `CDC-7` 1 Critical, `CDC-12` 4 Critical, `CDC-13` 2
Critical (= 23 Critical total), plus `CDC-2` 15 synchronizers missing
`ASYNC_REG`. Some are inside XDMA/Xilinx-generated structures; others
touch Saturn-level signals (TX_ENABLE, CODEC_MISO, ADC_MISO, board-version
pins, resets, FIFO resets per the review) and need individual fixes or
documented waivers. Confirmed: `FPGA/lab/tcl/reports.tcl`'s
`implementation_quality_gate` only writes WNS/WHS/DRC fields to
`quality-gate.txt` — **it does not check or reject CDC findings at all.**

**4. External timing is incomplete.** `results/vivado/methodology.txt`
names the unconstrained ports explicitly (confirmed by direct read):
`PROM_SPI_io0..3_io`, `PROM_SPI_ss_io[0]`, and `TX_DAC_PWM` have no
input/output delay relative to their clocks. `timing-summary.txt` reports
0 unconstrained *internal* endpoints, but `check_timing` separately
reports 4 input ports with no input delay at all, 7 more with only a
false-path exception, and 32 output ports with only a false-path
exception (11 + 32 lines up with the review's count). **WNS/WHS only
qualify the paths that are actually constrained** — these ports need real
constraints or documented async exceptions.

**5. The manifest's `git_dirty: true` is a real artifact but not a
regression.** Root-caused in `FPGA/lab/tcl/common.tcl`'s `git_dirty` proc:
it runs `git diff`/`git status --porcelain` live, from *inside* the Vivado
Tcl session, while `FPGA/saturn_project/saturn_project.xpr` (and other
guarded files) are still in their Vivado-modified state — `scripts/vivado-windows.sh`
only restores those files in its `EXIT` trap *after* the whole Vivado
process (and thus the manifest write) has finished. So `git_dirty: true`
in `manifest.json` reflects that in-run moment, not the final repo state
(confirmed clean via `git status` right after the build completes). Fix
would be to capture the dirty flag once, before backing up/launching
Vivado, and thread that captured value into the manifest instead of
re-checking live.

**Artifacts from the latest build, independently re-hashed and matching
exactly:**
- `saturn-bcbd9c4c.bit`: `1e451743db68d736dbc9a12b668ae1007a1d540baf38e91bb5cc88358db333fc`
- `saturn-lab.bin`: `18302f4bb33e306b3f481924b516c28dde2c5fe60fc5871cefc2752b3253dc84`
- `saturn-lab.prm`: `33838c90f09bae99604c5f23ff78c3e1d658c0393e955c010d86e9371afecc24`

## 6. Prioritized path back on track

1. **Checkpoint complete.** Commits `114a00a` and `f40132d` protect the
   Phase 0 lab and reviewed RTL/testbench changes.
2. **Project drift resolved.** The `.xpr` was reverted to the canonical
   checked-in form; the remaining IQ-mod work is the simulation migration
   described in §2.
3. ~~Run `make doctor`~~ **Done and independently re-verified 2026-09-11**
   in the `Ubuntu` WSL distro (interactive shell): `LAB_READY`. Golden
   fallback SHA256:
   `543b207750e0aafe1c63ffb377efa4fea4624527a5f6c09744291779b096f037` —
   record this outside the repo if not already done.
4. ~~Run `make check`~~ **Done and independently re-verified 2026-09-11**:
   lint (`SATURN_LAB_LINT_OK`), formal watchdog `prove`+`cover`
   (k-induction pass, cover trace reached), and both Python DSP regression
   tests all pass on this machine.
5. ~~Run `make vivado-build`~~ **Done, current as of commit `bcbd9c4`
   (2026-09-11)**: `saturn-bcbd9c4c.bit`, WNS/WHS/DRC all pass — see §5.5
   for the full picture. **This does not mean Phase 0 acceptance
   criterion 4 is met** — see §5.5 for the 23 CDC Critical findings and
   unconstrained I/O timing that still need resolution or documented
   waivers before this build can be called signed off.
6. ~~Run `make sim-iqmod`~~ **Done** — reaches `$finish` — but see §5.5
   item 1: the testbench itself has a wiring bug, so this does **not**
   validate IQ modulation functionality. Fix the testbench before trusting
   this result.
7. ~~Run `make export-prom`~~ **Done** — `saturn-lab.bin`/`saturn-lab.prm`
   generated and hashed, recorded in §5.5 and `PROM_BIN_EXPORT.md`.
8. **New priority, before calling Phase 0 done**: work through the §5.5
   gaps in order of risk — (a) add real IQ output checks, (b) retain the
   corrected DUC warm-up settings and capture a formal baseline, (c) triage the 23 CDC
   Critical findings (fix or document individual waivers), (d) add real
   timing constraints or documented async exceptions for PROM_SPI/TX_DAC_PWM,
   (e) fix the manifest's `git_dirty` timing so it reflects final repo
   state, not mid-run state.
9. Only after all 5 Phase 0 acceptance criteria in `FPGA/lab/README.md`
   are genuinely met (not just "build completes"), start V28 telemetry
   work — cross-check the wire/register format against §5 so `P2_app`
   doesn't need a second protocol.
10. Expand formal coverage from watchdog-only to `FIFO_Monitor.v` and
    `DDCMux.v`, as already earmarked in their `rtl-tests/*/README.md`
    files.

## 7. Open questions worth resolving with the user before continuing

- The `.xpr` drift is resolved by reverting Vivado's cosmetic rewrite; keep
  the canonical checked-in project path.
- The guarded launcher now has a nested-BD simulation synchronization step;
  verify it with `make sim-iqmod` in the configured Vivado environment.
- G2 hardware is not currently connected. CM4/XDMA programming and
  hardware-in-the-loop tests remain deferred; current work is
  simulation/synthesis-only.
- Should `Ubuntu` be made the default WSL distro (`wsl --set-default
  Ubuntu`) to remove the distro-mismatch trap for future sessions, or is
  `Ubuntu-24.04` needed as default for something else?
