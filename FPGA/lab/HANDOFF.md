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
`ASYNC_REG`. Confirmed: `FPGA/lab/tcl/reports.tcl`'s
`implementation_quality_gate` only writes WNS/WHS/DRC fields to
`quality-gate.txt` — **it does not check or reject CDC findings at all.**

**CDC triage — every one of the 23 Critical findings traced to its actual
source/destination (not just counted):**

- **15 waiver/documentation candidates, no RTL work:**
  - 6 pure vendor IP: 3× `pcie_reset_n` (CDC-1) + 1× `pcie_reset_n` (CDC-7)
    + 2× CDC-13, all inside Xilinx's XDMA/PCIe2-to-PCIe3 wrapper
    (`saturn_top_i/PCIe/xdma_0/inst/...pcie2_ip_i/...`) — pre-verified
    vendor hard IP.
  - 4 intentional clock-monitor sampling: `ref_in_10`, `EMC_CLK`, and the
    `clk_wiz_0` MMCM `CLKOUT0` (×2), all landing in `clock_monitor_0`,
    whose entire job is sampling foreign clocks — single-register
    sampling is the designed mechanism, not an oversight.
  - 5 static hardware straps: `pcb_version_id[0..3]` (5 rows, one bit
    feeds two destination bits) into `c_addsub_0` — board-version ID
    pins, stable at power-up, no runtime CDC hazard.
- **4 straightforward-looking real-signal crossings — NOT yet proven
  safe, do not blindly add synchronizers:**
  - `TX_ENABLE` → `AXIL_ReadReg_64_0/rdatareg_reg[31]/D` (status readback
    mirror) — fix if the destination is confirmed to be a genuinely
    different clock domain.
  - `CODEC_MISO` → `AXIL_SPIWriter_0/shiftinreg_reg[0]/D` and `ADC_MISO`
    → `AXI_SPI_ADC_0/ADCData_reg[0]/D` — **both modules
    (`FPGA/sources/verilogmodules/axil_SPIWriter.v`,
    `axi_spi_adc.v`) generate their own SPI clock from `aclk` and sample
    MISO at a specific FSM state timed against that self-generated
    clock** (confirmed by direct RTL read: `axil_SPIWriter.v` samples
    `SPIMISO` on entry to state 4, "deassert SCK; shift in input bit";
    `axi_spi_adc.v` samples `MISO` at a specific `clk_phase`/`BitCnt`).
    Inserting synchronizer flops before the sample point shifts the
    effective sample instant without adjusting the FSM's phase count —
    could sample a stale or wrong bit. Relevant timing budgets, read
    directly from the documentation PDFs:
    - Codec SPI (`FPGA/documentation/Codec SPI Write Timing Diag.pdf`):
      SPICk ≈ 10 MHz, 4 aclk-driven FSM states per bit cell, MISO
      sampled on SCK deassert (entry to state 4); codec gets ~100 ns
      setup time since last shift.
    - ADC SPI (`FPGA/documentation/SPI ADC timing diagram.pdf`): SCK
      period 128 ns (7.8125 MHz), each FSM clock phase 32 ns, MISO
      sampled on SCK rising edge.
    Focused Vivado cross-probing confirmed both FSM instances use
    `clk_122_1`, the 122.88 MHz `aclk`. The generated SPI clocks are
    synchronous derivatives of that domain, but synchronizer insertion still
    requires checking the FSM sample phase against the peripheral setup/hold
    budget.
  - `TX_ENABLE` → `TX_DUC_0/regmux_2_1_0/dout_reg[15]/D` — the most
    suspicious finding: this is the *same* 16-bit register as 15 sibling
    bits that already carry a 2-flop path (`dout_reg[0..14]`, flagged only
    as CDC-2 Warning, missing `ASYNC_REG`) — bit 15 alone shows zero
    depth (Critical). `regmux_2_1` itself
    (`FPGA/sources/verilogmodules/regmux_2_1.v`) is a plain synchronous
    2:1 mux with no per-bit logic, so the asymmetry originates in how the
    `TX_DUC` block design
    (`FPGA/IP/DUCIP/DUCIP.srcs/sources_1/bd/TX_DUC/TX_DUC.bd`) wires that
    instance's `din0`/`din1`/`sel` — needs Vivado's IP Integrator canvas
    or CDC cross-probing to see the actual per-bit connections; not
    traceable from RTL/grep alone.
    The focused top-level probe shows `/Transmitter/TX_DUC_0/regmux_2_1_0`
    clocked by `Net5`/`clk122`, with `din0` tied to the zero constant and
    `din1` tied to the full `mult_gen_0/P` bus; no bit-15-specific BD
    connection is present. The remaining asymmetry is therefore
    post-synthesis net behavior or boundary optimization, not an obvious BD
    wiring typo. The apparent two-flop depth on bits 0–14 is likewise not a
    deliberate synchronizer; it is an optimization artifact and cannot be
    treated as CDC protection. The robust fix is one explicit,
    `ASYNC_REG`-tagged two-flop synchronizer on `TX_ENABLE` upstream of this
    mute mux, replacing the accidental per-bit behavior and addressing all
    16 related findings.
- **4 CDC-12 "multi-clock fan-in" findings — need Vivado tracing:**
  `AXIL_ConfigReg_64_1/config_reg0_reg[1]/[3]` and
  `Double_D_register_syncareset1/Intermediate2_reg[0]` both fan into the
    same `xpm_cdc_sync_rst` reset-synchronizer inputs for
    `axis_data_fifo_DUC` and `axis_data_fifo_codecspk`. Vivado cross-probing
    shows the explicit downstream fan-ins `PCIe/DUCFIFORstn →
    FIFO_Interfaces/TX_FIFO_aresetn` and `PCIe/CodecSkpRstn →
    FIFO_Interfaces/CodecSpkResetn`. The remaining question is the upstream
    convergence inside `saturn_top.bd`: where the two different-clock
    register sources merge into `DUCFIFORstn`/`CodecSkpRstn`. Two different-
    clock registers driving one synchronizer input is a real hazard (the
    synchronizer only guards one source domain).

**Agreed triage order** (Codex's sequencing, endorsed): (1) document all
23 in this section — done above; (2) add one explicit `ASYNC_REG`
two-flop synchronizer for `TX_ENABLE` upstream of the DUC mute mux and fix
the readback crossing if its destination clock is confirmed different; (3)
trace the 4 FIFO reset fan-ins and synchronize each source
independently; (4) for `CODEC_MISO`/`ADC_MISO`, confirm the exact `aclk`
frequency driving each FSM against the timing budgets above before adding
any synchronizer; (5) add formal CDC waivers only for the vendor/monitor/
strap buckets (15 findings), each with path-specific rationale, not a
blanket waiver.

**Important, and not obvious from the CDC report alone**: a `set_false_path`
does not resolve a CDC finding. `TX_ENABLE` already has
`set_false_path -from [get_ports TX_ENABLE]`
(`FPGA/constraints/timingconstraints.xdc:198`), and `pcb_version_id[0..3]`
("board-version pins") already have their own `set_false_path -from`
entries (lines 218–221) — yet **both still appear in the CDC-critical
list**. Timing exceptions and CDC signoff are separate dimensions: a
false-path only tells the STA to ignore setup/hold on that path; CDC-1
still wants either a real 2+-flop synchronizer tagged `ASYNC_REG`, or an
explicit, documented CDC waiver. So the CDC triage for these signals is a
genuine synchronizer-or-waiver decision, not something the existing
timing constraints already cover.

**4. External timing — fixed in commits `f82dc67` + `2613c2f`, one build
short of full empirical confirmation.** `results/vivado/methodology.txt` flags `PROM_SPI_io0..3_io`,
`PROM_SPI_ss_io[0]`, and `TX_DAC_PWM` as missing a delay relative to
`VIRTUAL_clk_125mhz` / `clock_122_in_p` respectively (confirmed by direct
read). But `FPGA/constraints/timingconstraints.xdc` shows these ports
already have real, deliberate constraints — just against a *different*,
slower functional clock:
- `TX_DAC_PWM`: `set_output_delay -clock [get_clocks VIRTUAL_clk_out2_saturn_top_clk_wiz_0_0] -min/-max -add_delay 0.000` (lines 89–90) — a virtual 81.38 ns (÷10 of the 122.88 MHz sample clock) PWM-update clock.
- `PROM_SPI_io0..3_io`/`ss_io[0]`: `set_output_delay -clock [get_clocks clk_sck] -max 1.700 -min -2.200` (lines 176–185), where `clk_sck` is a real `create_generated_clock` sourced from the AXI Quad SPI IP's own `ext_spi_clk` (line 166), plus existing `set_multicycle_path` exceptions between `clk_sck` and that IP's internal `ext_spi_clk` domain (lines 172–173, 186–187).

So these aren't unconstrained-and-ignored — they're constrained for their
real functional clock, but `check_timing` was separately flagging a path
from the *other*, faster reference clock domain that wasn't yet excepted.
**Fixed, matching the design's own established pattern** (`RF_SPI_*`,
ADC SPI, `CODEC_SPI_*` all use plain `set_false_path` for exactly this
kind of slow control interface — lines 48–51, 93–96, 201–205):
- `f82dc67` added `set_false_path -from [get_clocks clock_122_in_p] -to
  [get_ports TX_DAC_PWM]` and `-from [get_clocks VIRTUAL_clk_125mhz] -to
  [get_ports PROM_SPI_io0..3_io / ss_io[0]]` (output direction) —
  verified by direct diff read.
- The same build's `methodology.txt` still showed the *input*-direction
  half of the report (`PROM_SPI_io*_io` are bidirectional `_io` ports, so
  `check_timing` wants both directions excepted) — `2613c2f` added the
  mirrored `set_false_path -from [get_ports PROM_SPI_io*_io] -to
  [get_clocks VIRTUAL_clk_125mhz]` entries. Also verified by direct diff
  read.

**Not yet empirically re-confirmed**: the `methodology.txt`/
`timing-summary.txt` currently on disk are from the build at git SHA
`f82dc67` (confirmed via `manifest.json`) — i.e. *before* `2613c2f`'s
input-direction fix. They still show the 4 `PROM_SPI_io0..3_io`
"input delay... missing" (HIGH) lines from `TIMING-18`. That's expected
given the timing, not a sign the fix failed — but it means one more
`make vivado-build` is needed to get a fresh methodology report and
confirm the input-side gap is actually closed, not just logically sound.

(Note: `FPGA/documentation/Generating Configuration PROM file.docx` was
checked — it's a GUI click-through for the PROM export step only
(Spansion S25FL256S part, SPIx1, BIN format); it contains no SPI-timing
rationale, so it doesn't bear on this constraint gap.)

**5. The manifest's `git_dirty: true` mistiming is now fixed (commit
`eacce67`).** Root cause was confirmed in `FPGA/lab/tcl/common.tcl`'s
`git_dirty` proc: it ran `git diff`/`git status --porcelain` live, from
*inside* the Vivado Tcl session, while guarded project files were still in
their Vivado-modified state — `scripts/vivado-windows.sh` only restores
those files in its `EXIT` trap *after* the Vivado process (and thus the
manifest write) finishes. The fix: `vivado-windows.sh` now computes
`git_dirty` once, before backing up/launching Vivado, and exports it as
`SATURN_GIT_DIRTY`; `common.tcl`'s `git_dirty` proc reads that env var
first and only falls back to a live check if it's unset. Verified by
direct read of both files. **Empirically confirmed 2026-09-11**: the
build at git SHA `f82dc67` produced `manifest.json` with `git_dirty:
false` on a tree that had just been committed — independently verified by
reading the current manifest. Fully closed.

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
8. **Remaining priority, before calling Phase 0 done**: work through the
   §5.5 gaps — (a) add real IQ output checks (wiring bug itself is fixed
   as of `eacce67`, but there's still no output assertion), (b) retain the
   corrected DUC warm-up settings and capture a formal 262,144-sample
   baseline, (c) triage the 23 CDC Critical findings — note `TX_ENABLE`
   and `pcb_version_id[*]` already have timing false-paths yet are still
   CDC-critical, so each needs an explicit synchronizer-or-waiver
   decision, not just a timing fix. ~~(d) PROM_SPI/TX_DAC_PWM timing
   exceptions~~ **done in `f82dc67` + `2613c2f`** — one more
   `make vivado-build` needed to regenerate `methodology.txt` and confirm
   the input-direction fix empirically (§5.5 item 4). ~~(e) fix the
   manifest's `git_dirty` timing~~ **done in `eacce67`, empirically
   confirmed** (§5.5 item 5) — `git_dirty: false` now shows correctly.
   **CDC triage (c) is the only remaining substantive Phase 0 blocker.**
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
