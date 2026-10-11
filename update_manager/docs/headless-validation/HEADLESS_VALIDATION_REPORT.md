# Automatic headless Saturn Remote validation in the supported update process

GoldMarsh, 2026-10-10 and 11. Branch `goldmarsh/headless-validation`, built on `f10a087` (Stage A) and the live-driver tools.
FPGA 1.31.002 / build ID `0x53460004` and the XDMA kernel driver are untouched. **The G2 is still on release `329f3109`** with the `rollback_failed` record exactly as the first
activation left it; an earlier build of this branch is installed *inactive* beside it and the new activation helper, validator and guard are installed with activation still
disabled. The first hardware `check` has been run (read-only) and failed, for reasons described next; this report describes the source that answers it.

This file and `g2_headless_steps.sh` live in the source tree (`update_manager/docs/headless-validation/`) so that nothing depends on a scratch directory. The commit
under test is the branch head; the hand-off message gives its full SHA, and the procedure refuses to build or run anything else (`PIN=<that SHA>`).

## What was wrong

The activation gate needs an active receive client at the owner (`state.sdr_active`), which exists only while something is receiving. With every browser closed nothing
attached one, so the readiness check failed, and so did the rollback's identical check. The previous release was in fact restored (proved from the helper's own nine
conditions); the helper recorded `rollback_failed` and exit 70 because "restored" and "receiving" were one check.

## What the first hardware check found (G2, release `329f3109`, all browsers closed)

The check ran the real validator in its locked-down transient unit on the Pi. It did **not** pass, and the evidence (kept under `/var/lib/saturn-state/deployments/validation/`) says why:

* **The kernel cannot apply the memory cap.** `/proc/cmdline` has `cgroup_disable=memory`, `cgroup.controllers` lists only `cpuset cpu io pids`, so systemd accepts `MemoryMax=350M` and
  `MemorySwapMax=0` and enforces neither (`MemoryCurrent` reads `[not set]`). The cap "as designed" does not exist on this host; the validator would have reported exactly that once it
  got as far as reading it.
* **The audio did not play.** Display data and 457 Opus frames arrived, no decode error, the AudioContext ran, but the old page played 1 decoded frame in 45 s (30 needed), dropped
  160 as late (its decoder output is discarded if it arrives more than 250 ms after the packet) and counted 19 sequence gaps (each of which tears down and rebuilds its decoder).
  The Bridge's raw-IQ delivery queue (capacity 4 per client) overflowed during the session. A browser that cannot keep up looks exactly like this.
* **Nothing after the first failed condition had been read** (sandbox, memory, owner, wire, radio all showed "none"): the run stopped at the first failure.

What I did about each, measured on real Chrome with the real page, Bridge and proxy (the x86 numbers are not the Pi's, but the relative effects are what decided it):

| | processes' memory (Pss) | browser CPU (cores x 100%) |
|---|---|---|
| as run on the G2 (page in front, `--alsa-output-device=null`) | about 625 MiB | about 350% |
| page in a background tab | 576 MiB | 153% |
| Chrome's own fake audio output (`--disable-audio-output`) | 607 MiB | 220% |
| **both** (what the validator now does) | 577 MiB | **17.5%** |
| both, plus `--enable-low-end-device-mode --disable-software-rasterizer --num-raster-threads=1` | **509 MiB** | 18.6% |

The CPU was the audio: Chrome's WebAudio render thread and its output thread (about 50% and 30% of a core each) rendering against an ALSA "null" device that is not paced by a timer,
plus painting a page nobody can see. Nothing the validation proves is lost: in every variant the same Opus frames arrive and the same number are played, on the new page and on the old
one. The viewport size, the old page's WAN transport, one renderer process, and site isolation settings did not matter, and `--no-zygote` breaks the sandbox (rejected; a test now forbids
any switch that weakens it for the sake of memory). **Memory does not go below about 500 MiB on this Chrome under any switch**: the page's own JavaScript heap is 7 MiB; the rest is
Chrome's per-process baseline (five renderer processes about 215 MiB, the browser process about 120 MiB, utilities about 95 MiB, the GPU process about 67 MiB, zygotes about 27 MiB).

So, in the source:

1. The validator runs the page in a **background tab** and uses Chrome's **fake audio output**, with the three lean-rendering switches; the evidence records `page_visibility`.
2. Where the kernel has no memory controller the **validator is the cap**: a thread reads the browser's proportional memory four times a second and stops its unit above the cap, writing the
   failed condition `browser.memory_within_cap` *before* it acts so nothing the dying browser causes is reported first. This is named in the evidence (`memory_enforcement: watchdog`, and
   `MEMORY CAP ENFORCED BY THE VALIDATOR'S WATCHDOG, not by the kernel`), never silent. With a memory controller the kernel's `MemoryMax` is used and checked as before. The unit also gets
   `OOMScoreAdjust=1000`, so if the host does run out of memory the browser is the OOM killer's first choice, not the radio.
3. **No browser is started unless the host has the memory free**: `MemAvailable` must cover the cap plus 32 MiB (`host.memory_headroom`), so a validation can never push the radio's
   processes into swap. On the Pi (about 580 MiB available) that bounds a measuring cap to roughly 540 MiB.
4. **Every independent condition is read even after one fails**, so one run reports the sandbox, the memory cap, the owner's view, the wire audit and the radio whatever the audio did. The first
   failure is still the one reported as the failed condition.
5. **The evidence says where the resources went**: once a second, the browser's memory by process type, CPU, major faults, the host's CPU and I/O wait, swap traffic and available memory, the
   page's own main-thread load, the legacy page's decoder counters, and the Bridge's drop counters. `check` prints a summary of it, pass or fail.
6. `check [SIZE]` (for example `check 500M`) measures with another browser memory cap for that one check (`--check-reception --memory-max`, refused with anything else, never stored).

**What this does not settle, and I have not hidden:** whether this Chromium fits the Pi. Its memory could not be driven below about 500 MiB on x86 and the Pi has about 580 MiB available
beside the running radio. The next `check` measures it. If it does not fit, the options are a larger cap with the headroom check (the radio's processes are never the ones that pay), enabling
the memory cgroup controller in the Pi's kernel command line (`cgroup_enable=memory cgroup_memory=1`, which needs a reboot and is the owner's call), or a validation client that is not a full
browser (a design change, also the owner's call).

## What the updater does now

After the target services start, the helper runs a temporary receive-only headless browser session against the existing Bridge, holds it while the helper's own
readiness check runs, and cleans it up on success, failure, timeout or signal. It never starts a Bridge, opens XDMA, retunes, or applies a saved profile. Validation
passes only if all of this holds (a connected socket or an HTTP 200 proves nothing):

* the target release is the one running (`/readyz` commit and every service's binary under that release directory);
* the page loads, the receive-only guard is proven active in it before anything is sent, and there is no fatal error;
* fresh display data and Opus frames advance, **decoded audio frames are played and advancing**, the AudioContext runs, there are no decode errors;
* the owner's telemetry is fresh and from the running service, counts the session, and the helper's own seven-condition gate holds with the session attached;
* the RXC1 status is the intended one (target only); frequency, rate, DDC, ADC and TX flags are unchanged;
* every frame that left the browser is on the allowlist, judged live from outside the page;
* **the viewer role was given by the Bridge, or the release is one of the named legacy releases below;**
* **the host has the memory free for the browser before it starts, and the browser's sandbox is in force and its memory cap is enforced (by the kernel, or by the named watchdog), read from the host (below);**
* afterwards the browser, its profile and the owner connection are gone.

The same validation runs after a rollback, and the transaction record reports `release_restored` separately from `receive_verified`. Exit 70 = the previous release
could not be restored; 71 = restored but reception unproven; anything else = the activation's own failure with a verified rollback.

## The four things asked for in this round

**1. Viewer role.** The validator's default is `--viewer-role required`: an operator-role session is refused (`session.connected_as_viewer`). The only exception is the
named compatibility fallback `--viewer-role legacy-fallback`, which the helper selects **only for a release listed in `VALIDATION_LEGACY_BRIDGE_RELEASES`**, by exact full
commit: `329f3109…` (the release a rollback returns to), `4247ed95…`, `91d97663…`, `f10a087…` — the four installed on the G2, all of which predate the Bridge fix
`dfb0b19`. A new release is not listed, so its Bridge must give the viewer role. When the fallback is used it is recorded as `legacy_fallback_used: true`, in its own
condition (`session.role_legacy_fallback`, never as `session.connected_as_viewer`), in the transaction record, and in a `LEGACY BRIDGE FALLBACK USED` log line that
`--check-reception` and the `check` stage print. The guards are not relaxed: the injected guard must already be proven active before a session is started, the wire
audit judges every frame, and the unchanged-radio check applies. There is no configuration switch that turns the requirement off (`VALIDATION_VIEWER_ROLE` was removed).

Finding about the old Bridge, from running it: the page opens a control and a media connection, and the old Bridge gives the operator role to whichever registers first.
So an old-Bridge session is *sometimes* operator and sometimes viewer, and the rehearsal now judges each run by what the Bridge actually did in it (below).

**2. Sandbox and memory limit are verified, not assumed.** While the session runs the validator reads, from the host: no browser process runs with `--no-sandbox`; every
renderer is confined by a seccomp filter (`Seccomp: 2` in `/proc/<pid>/status`; a renderer without the sandbox shows `0`, which I confirmed on a real Chrome both ways);
and the transient unit has the configured `MemoryMax` and `MemorySwapMax=0` (read with `systemctl show`). Peak memory is kept. A browser that cannot start its sandbox
inside the unit fails the validation — `VALIDATION_NO_SANDBOX` was removed rather than left as a way round it. The `check` stage of the procedure then also asserts, from
the host and independently of the validator's own cleanup conditions: no `saturn-remote-validator-*` unit, no `--remote-debugging-pipe` process, the Bridge's connection
count back to what it was, and no leftover profile or private-temp directory.

A real Chrome found a defect the 100+ fake-browser tests could not: processes the browser forks retitle their command line into one space-separated string, so splitting
`/proc/<pid>/cmdline` on NUL never saw `--type=renderer` and the check reported "no renderer". It now splits on NUL and whitespace, and the test fake uses the observed shape.

**3. Reconciliation preserves the failed transaction.** `--reconcile-rollback` no longer rewrites anything the failed activation recorded. `activation_failure`, `rollback`
(still `failed`, `release_restored: false`, `receive_verified: not_attempted` in the test fixture) and any `validation` evidence are byte-for-byte what they were; the status
becomes `rolled_back` (the previous release is running; the target was never committed — never `committed`); and the verified recovery is recorded **beside** them under
`reconciliation`: `release_restored`, `receive_verified`, the validation evidence, the SHA-256 of the preserved file `current.json.rollback-failed.<time>`, and a complete copy of the
original record. The message says the activation did not succeed. It still refuses, leaving the record byte-identical, unless the previous release is verifiably running
(pointer, drop-ins identical to the rollback snapshot, services as they were, exact running commit, healthy owner) and receiving.

**4. Nothing depends on scratch.** Report, procedure, their test and the CI steps that run them are in the source. The procedure cannot name the commit it is stored in, so
the commit is given once (`PIN=<full sha> bash g2_headless_steps.sh build`), checked against what the branch resolves to, remembered in `~/.g2_headless_pin`, and every later
stage refuses a different one. The four hashes it pins (the helper's installer, the helper, the validator, the guard) are those of the committed blobs, and `tests/test-g2-headless-steps.sh` fails if they drift.
The release manifest is held to the same commit: the `validate` stage (staged bundle) and the `install` stage (installed copy) both require `release-manifest.json` to name exactly
the pinned commit as a clean tree, with every component built from it, and `status` prints the same finding. So the script, the build and the release manifest are demonstrably one commit.

## Evidence (all run on this machine; the G2 has run none of it)

| what | result |
|---|---|
| validator tests (fake DevTools browser + fake owner + fake unit/cgroup/proc/meminfo, a fake `systemctl kill` that really kills) | **146 pass.** 30 mutations of the rules (sandbox, role, memory by cgroup and by watchdog, headroom, evaluate-all, background tab, audio sink, lean switches, process accounting, evidence): all killed, after closing six gaps they exposed (a tripped watchdog could still be reported "ok" at the end; a browser stopped on purpose left unrelated failures; the cgroup mode and the watchdog mode were not distinguishable; a DevTools failure caused by the memory stop was reported first; and two assertions) |
| activation helper tests (incident control; success with nobody connected; stalled audio; rollback with and without verified reception; policy by release; isolation failure; interrupted helper; reconcile; `--check-reception --memory-max`) | pass; 28 mutations earlier and 6 for the measuring cap and the record: all killed |
| installer for the helper/validator/guard | pass |
| staged procedure (`tests/test-g2-headless-steps.sh`): pins, commit lock, manifest-names-the-pin, the build, helper and check stages with the host's commands stubbed (including `check 500M` and bad sizes), verdict on about 70 variants of evidence | pass; every mutation tried over the rounds (24, then 9 for the manifest check, 8 for the build stage, 8 for the helper stage, 6 for the check size and the verdict rules) killed |
| real components on loopback: replay Bridge + saturn-go proxy (TLS, basic auth, pinned certificate) + the real page + **real Chrome 153 with its sandbox ON** | **101/101 checks** at the head before the last two edits (a rerun at the final head is in the hand-off): new page and old page, each with the page hidden and the browser averaging 12-15% of a core (350% before); display data stopped mid-measurement; Chrome started with `--no-sandbox` refused; a real Chrome stopped by the memory watchdog at a 60M cap (the cause is the first failure, Chrome and connection gone) and left alone under a 4G cap; old page + **old Bridge** (operator role refused under `required`, accepted as a recorded fallback). In every sandboxed run the real Chrome showed its renderers all `Seccomp: 2`. |
| shellcheck on every shell file CI checks | clean (see "Found in my own earlier push") |
| Bridge (unchanged since `dfb0b19`): 462 tests; web: 761 tests, typecheck, seam check | pass at the earlier head; no Bridge or web file changed since |

## Found in my own earlier work

* `tests/test-saturn-release-activate.sh` as pushed would have failed the repository's shellcheck gate (SC2012/SC2034/SC2123/SC2030/SC2031). Fixed; the new files are in the
  CI lists and the validator, installer-guard and procedure tests now run in CI.
* `cargo fmt -- --check` failed on the Stage-A sources already at `f10a087` (it passes at `main` `861f0fb`), which would fail the CI format step for this branch: nine files of the
  Bridge crate (`src/tci/{client,command_queue,tests}.rs`, `src/tx_thread.rs`, `src/xdma_backend.rs`, `tests/{common/mod,replay_e2e,replay_harness,replay_transport_screen}.rs`)
  and one assertion in `rust-server/src/main.rs` (from `a8fcad2`; my earlier note that `main` was clean covered only the Bridge crate). Two formatting-only commits fix them, with the
  toolchain CI pins (1.98.1): re-wrapping, re-indentation, and the braces rustfmt puts around closure bodies it wraps. Every changed file was compared with its previous text: the same
  characters once whitespace and commas are ignored, except those closure braces. No other file was touched; `cargo fmt -- --check` is clean in both crates.
* **The `build` stage had never been run, and its first run on the G2 stopped.** Two defects, both mine: (1) a non-interactive shell (`ssh pi@g2 'command'`) has no `~/.cargo/bin` on
  its `PATH`, so the builder reported `required command not found: cargo`; the procedure now puts that directory on the `PATH`. (2) Behind it, the stage looked for `@ <commit>` in the
  builder's `--resolve-only` output, which prints `resolved_commit=<commit>` (the `@` text is only in a real build's log line), so it would have refused the right commit and blamed the push;
  it now requires the exact line `resolved_commit=<pinned commit>` and shows the builder's own message when it cannot run. The test drives the stage with a builder that reports each case
  (the pinned commit, another commit, a longer string, a look-alike line, a builder that fails) and checks the real builder's source still prints that line. Nothing was built and nothing
  on the G2 changed in the failed run beyond `~/g2_headless_steps.sh` and `~/.g2_headless_pin`.
* **The `helper` stage ran the installer from a path where it does not exist.** The first run on the G2 stopped at `sudo: /usr/local/lib/saturn-go/scripts/saturn-release-install-activator-root.sh:
  command not found` before changing anything: only the *helper* is installed under `/usr/local/lib/saturn-go/scripts`; its installer is run from the validated, immutable release
  (`/opt/saturn/releases/<commit>/scripts/`, as `update_manager/release/rxc1/README.md` already says). My runbook had the same wrong path. Both now use the release's copy, the
  procedure pins the installer's SHA-256 with the other three before running it as root, and the test runs the stage with `sudo` behaving like the real one (a missing command is
  not found), corrupts each of the four files in turn, and makes the installer fail.

## Sequence on the G2 (you run the sudo stages; I read the evidence; ~nothing is restarted before `activate`)

0. Get the procedure: `curl -fsSL https://raw.githubusercontent.com/kd4yal2024/Saturn/<SHA>/update_manager/docs/headless-validation/g2_headless_steps.sh -o ~/g2_headless_steps.sh`
   (or I copy it from the pushed source), then `PIN=<SHA> bash ~/g2_headless_steps.sh build`.
1. `validate`, `install` (inactive), `helper` (activator + validator + guard, each pinned by SHA-256; activation stays disabled).
2. **`check [SIZE]`**, all browser tabs closed: the production launcher on the running `329f3109`; changes nothing. `check 500M` measures with that browser memory cap for that one check. Success prints `OK: reception, sandbox, memory cap and cleanup are all verified`.
   Because `329f3109` is a legacy release it is expected to print `LEGACY BRIDGE FALLBACK USED` or a plain viewer session (the old Bridge's race); either is acceptable, and it says which.
3. `reconcile`, then optionally `failure` (a real activation meant to fail validation, rolling back to `329f3109`), then `activate` (the normal update, RXC1 arm written last, all browsers closed).
   Per the instruction, 3 happens only after the source review and a successful `check`.

## Not verified / residual risk

* **Whether this Chromium fits the Pi is not known.** On x86 it needs about 500 MiB (see above); the Pi has about 580 MiB available next to the running radio. The next `check`, with the
  per-process-type memory and the swap/I-O figures it prints, decides it; nothing is guessed. The sandbox inside the locked-down unit did start on the Pi (the browser attached and loaded the
  page); whether every renderer reports `Seccomp: 2` there was not reached by the first check, because the run stopped at its first failure, and is read by the next.
* The kernel offers no pressure-stall information on this Pi (`/proc/pressure` is absent), so the diagnostics use CPU, I/O wait, swap and fault counters instead.
* `MemoryPeak` needs systemd >= 255 and a memory controller; the peak is otherwise the validator's own measurement (proportional set size), and the evidence names which.
* On the old Bridge the operator slot may be held by the media connection of a validation session for ~15 s; a person connecting in that window would be a viewer until it ends.
  Nothing in the new release has this: its viewer-declaring session releases the slot (`dfb0b19`).
* `valid` RXC1 has never been observed against a real owner, only the replay's `disabled`; the `failure` stage deliberately uses an RXC1 expectation the target cannot meet.
* ARM binaries come only from the G2's own native build of the pushed commit; none were built here.
