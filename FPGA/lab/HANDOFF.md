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
5. ~~Run `make vivado-build`~~ **Done and independently re-verified
   2026-09-11**: `results/vivado/quality-gate.txt` shows WNS 0.176 ns, WHS
   0.049 ns, 0 DRC errors, 0 DRC critical warnings. Bitstream
   `results/vivado/saturn-a2e84943.bit` SHA256 independently recomputed
   and matches `manifest.json` exactly:
   `8710d9066a96e41babb439d58922f40f4aa2d6a702df4ea13613ca1535212d49`.
   **Caveat**: `manifest.json`'s `git_sha` is `a2e8494` (the commit
   *before* the Phase 0 checkpoint) with `git_dirty: true` — this build
   predates commits `114a00a`/`f40132d`. No build has yet been run and
   manifested against the actual checkpointed HEAD. Re-run
   `make vivado-build` after resolving item 2 so the manifest reflects a
   real commit, not a pre-checkpoint dirty tree.
6. Run `make sim-iqmod` to verify the new nested-wrapper synchronization;
   then perform the permanent source-BD interface normalization if desired.
7. Do the PROM/BIN GUI export once, record the exact settings, then
   automate it in Tcl — explicitly flagged as the last unfinished Phase 0
   automation piece.
8. Only after all 5 Phase 0 acceptance criteria in `FPGA/lab/README.md`
   are met, start V28 telemetry work — cross-check the wire/register
   format against §5 so `P2_app` doesn't need a second protocol.
9. Expand formal coverage from watchdog-only to `FIFO_Monitor.v` and
   `DDCMux.v`, as already earmarked in their `rtl-tests/*/README.md`
   files.

## 7. Open questions worth resolving with the user before continuing

- The `.xpr` drift is resolved by reverting Vivado's cosmetic rewrite; keep
  the canonical checked-in project path.
- The guarded launcher now has a nested-BD simulation synchronization step;
  verify it with `make sim-iqmod` in the configured Vivado environment.
- Is a G2 unit currently connected for the CM4/XDMA programming and
  hardware-in-the-loop steps, or is work still simulation/synthesis-only
  for now?
- Should `Ubuntu` be made the default WSL distro (`wsl --set-default
  Ubuntu`) to remove the distro-mismatch trap for future sessions, or is
  `Ubuntu-24.04` needed as default for something else?
