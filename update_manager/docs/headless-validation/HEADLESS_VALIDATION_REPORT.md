# Automatic headless Saturn Remote validation in the supported update process

GoldMarsh, 2026-10-10 and 11. Branch `goldmarsh/headless-validation`, built on `f10a087` (Stage A) and the live-driver tools.
FPGA 1.31.002 / build ID `0x53460004` and the XDMA kernel driver are untouched. **The G2 now runs release `cc09c61f91030d4c5a38d6c570a20c28b5d97e03`**, activated on 2026-10-11 at 16:25 EDT
by the staged procedure: the receive validation ran as the viewer, the record is `committed`, RXC1 reads `valid` (see "Activation of `cc09c61` on the G2"). The route here: the first hardware `check` failed, for reasons
described next, and the source answered it (see "Hardware results with release `50898660`"); the first `activate`, of release `50898660`, then rolled back with exit 71 for two timing assumptions of the validator
(see "First activation attempt"), which `cc09c61` corrects. **That was the bootstrap through the staged shell procedure.** The normal user-facing update (the Update action of the Saturn Go page, and the Saturn-Go installer) has since been
connected to the same machinery on this branch for *existing installations that run releases* (see "Managed updates for existing installations"). That work is implemented and tested on a development
machine; **it has not yet been run on the G2** (see "What is still open").

This file and `g2_headless_steps.sh` live in the source tree (`update_manager/docs/headless-validation/`) so that nothing depends on a scratch directory. The commit
under test is the **release** `cc09c61f91030d4c5a38d6c570a20c28b5d97e03` (the earlier `50898660388fba554e0cbfc63844cbac7c87326b` was installed and rolled back, and stays installed inactive): it is what was built, is
installed and running on the G2, and is what the procedure pins (`PIN=`; it refuses to build or run anything else). The branch head is a separate identity, reported in the hand-off message: commits after the release that
change only this report, the procedure and its test are not release components and need no rebuild, and `git diff cc09c61f91030d4c5a38d6c570a20c28b5d97e03 <head> --stat` shows exactly what differs.

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

## Hardware results with release `50898660` (G2, 2026-10-11; the running release is still `329f3109`)

* `validate`, `install` and `helper` ran as designed: the manifest names the pinned commit, the four pinned files match, the release is installed inactive, activation stays disabled.
* **`check` at the default 350M cap stopped at 8.6 s**, before the page had loaded: the watchdog read 366 MiB (proportional set size) and stopped the browser. Every later condition is
  reported NOT PROVEN, not failed; cleanup was complete and no listed Bridge drop counter rose. It shows the browser had reached 366 MiB by then. It is not a general figure for
  "startup": the per-process-type numbers are each type's own maximum, reached at different moments, and do not add up to one instant.
* **`check 500M` passed** (peak 448 MiB, 52 MiB under the cap) and a **plain `check` with `VALIDATION_MEMORY_MAX="500M"` set in the activation configuration passed** (peak 413 MiB): display
  data, Opus frames, decoded audio, AudioContext, the owner's readiness gate, an unchanged radio, `Seccomp: 2` on all four renderers and no `--no-sandbox`, the validator's watchdog
  as the memory enforcement (this kernel has no memory controller), and complete cleanup checked from the host. The running release is a legacy one, so the operator-role fallback
  was used and recorded as such.
* **What it cost, and what it does not prove.** The browser averaged about 1.6 cores and the host about 72% busy for roughly 13 s; swap rose by about 70 MiB and fell back; the Bridge's
  IQ delivery-queue overflow counters rose by 13 and by 20 (the only listed counters that moved; they rose after the browser, the only client, attached); the page counted 20 underruns and
  74 to 115 late drops among about 320 Opus frames. This proves functional reception, not smooth playback or zero impact. The longer matched audio comparison is still to come.
* **500M is this host's setting, not a new default.** The repository default stays 350M. A browser is not started unless `MemAvailable` is at least the cap plus 32 MiB (532 MiB for
  500M); this G2 showed 551 to 586 MiB. The cap is persisted for this G2 in `/etc/default/saturn-release-activate`, so `check`, `reconcile`, activation and the rollback validation use it.
* **`reconcile` was refused** by the helper: it will not reconcile while the root-owned `ACTIVATION_ENABLED` policy is `"0"`. My procedure's `reconcile` stage did not switch the policy
  on for its run the way `failure` and `activate` do, and no test had ever run those three stages. Nothing was changed. See "Found in my own earlier push".

## First activation attempt on the G2 (2026-10-11 14:17, release `50898660`): rolled back, exit 71

`activate` (arm written last, every browser closed) restarted the Bridge and Go on the new release and failed twice at its very first look at the host, before any browser ran:

* **The target's validation was refused for memory**, one second after the new release started: 529 MiB available, 532 MiB needed (the 500M cap plus the 32 MiB reserve); the same host had shown 551 to 586 MiB
  at rest. No browser was started.
* **The rollback's validation raced the restarted Bridge.** The helper stopped the target, restored `329f3109` and restarted its services at 14:17:43; the validator began one second later, before that Bridge had
  written `/run/saturn-bridge/perf.json` (`owner.fresh_telemetry`: no such file). The helper therefore ended with exit 71: the previous release restored, reception on it not verified.
* **What was fine.** The radio ended receiving on `329f3109` (pointer, `/readyz`, fresh telemetry, `rf_safe`, no client, nothing left behind), and `check` runs before and after prove reception there. The validator's
  baseline of the new Bridge read RXC1 `valid` (the one-use arm was spent as designed): a confirmation for this candidate release; the October 9 trial had already seen `valid` on a live owner. The policy was switched on for the run and restored byte-identical,
  even on exit 71. The restarts reset the tuning to the Bridge's default of 14.2 MHz (it had been 3.902 MHz). The validator's "radio unchanged" condition compares the radio before and
  after its own browser session; it does not show, and cannot show, that settings survive the service restart before it. Keeping tuning across updates belongs to the normal-updater work that follows. The new activation record replaced the reconciled one; the original failed transaction stays at
  `current.json.rollback-failed.<time>` and the reconciliation's evidence in the validation directory.
* **Both are assumptions of the validator about a host that has just been restarted, not defects of the release.** The validator now waits, within a bound (`owner_wait_s` and `headroom_wait_s`, 30 s each), for the
  owner's first telemetry (present, fresh, the running service's own) and for MemAvailable to reach what the browser needs; it says how long it waited and what it saw first and at best, stops waiting when told to
  stop, and still fails, with the reason, when the bound passes. A correction to the validator is a release component change, so it needs a new build; the installed `50898660` was not retried: another
  attempt with the same validator would have gambled on the same restart timing.

## Activation of `cc09c61` on the G2 (2026-10-11 16:25 EDT)

`validate`, `install` and `helper` ran as designed; `activate` was run next (no separate `check` with the new validator was run first; the activation's own validation was its first use on hardware).
The helper exited 0. Read independently over ssh afterwards, not from the helper's output:

* **Running identity.** `/opt/saturn/current` is `/opt/saturn/releases/cc09c61f91030d4c5a38d6c570a20c28b5d97e03`; `saturn-bridge` (since 16:25:46) and `saturn-go` (since 16:25:47) run from that release's `bin/`, and their
  SHA-256 equal the manifest's (`saturn-bridge` `21bed70518acdf80...`, `saturn-go` `baa1b154a51b4718...`). `/readyz` reports `cc09c61...` as both `build_commit` and `expected_commit`; the Bridge's telemetry is fresh,
  `dirty False`, `rf_safe True`, FPGA `0x53460004`, backend `xdma`, no client. The configuration is untouched (`ACTIVATION_ENABLED="0"`, the single `VALIDATION_MEMORY_MAX="500M"` line).
* **The record** is `committed` / `commit` (`target cc09c61...`, previous `329f3109...`), with no `activation_failure` or `rollback`; `validation.target` is `done`, driver `native`, role `viewer` (enforced by the Bridge,
  policy `required`), `legacy_fallback_used` false.
* **All 25 conditions of the validation passed**: display data (36 rows), Opus frames (144), decoded audio (77 played in 3.5 s), AudioContext running, no decode errors, the owner's readiness gate with the session
  attached, the receive-only guard before any page script, 63 frames left the browser all inside the allowlist, the radio unchanged, `Seccomp: 2` on all four renderers with no `--no-sandbox`, the memory cap enforced by the
  validator's watchdog (peak **437 MiB** of 500M), and a complete cleanup (no unit, process, connection or leftover directory; the owner counts 0 connections).
* **The new page behaves very differently from the old one** (in these short validation runs; they are not the matched audio comparison, which is still to come). 297 Opus frames, 230 played, 1 late drop, 2 underruns (the old release's page: about 320 frames, 100 to 220 played, 74 to 115 late drops, 20 to 28 underruns),
  the browser averaged 96% of a core (the old page's 140 to 165%), and none of the Bridge's drop counters moved (the old page raised the IQ-queue overflow counters by 6 to 22 per session).
* **RXC1 reads `valid`** at the validator's baseline and in the Bridge's telemetry afterwards; the one-use arm was spent as designed.
* **The launch check passed by 1 MiB**: 533 MiB available against the 532 MiB the browser needs (cap plus reserve), on the first reading (no wait was needed). That is the margin of the launch check at that instant, not
  the radio's remaining memory. One reading just below it (529 MiB) refused the first attempt. The bounded wait lets a validation recover from a temporary dip; it cannot guarantee that enough memory is there. The reserve
  stays as it is while this work is completed; a leaner browser, or a smaller reserve, is the first thing to revisit if validation is refused again.
* **Radio settings.** The Bridge restarted at its default tuning of 14.2 MHz, as the previous restarts did. Settings do not survive an update yet.

The evidence (the `committed` record, the target validation directory, the journal and the state) is kept off the G2 in the review folder `activation_cc09c61_20261011/`, hashes verified. The earlier exit-71 attempt is kept
in `exit71_attempt_20261011/`.

## Managed updates for existing installations (implemented and tested here; not yet run on the G2)

* **Entry point.** The Update action of the Saturn Go page runs `update-saturn-go.sh`. It now decides on the *layout* (is a release what runs: `/opt/saturn/current` is a link to `/opt/saturn/releases/<commit>`)
  separately from the *authorization* (`ACTIVATION_ENABLED`). A release layout with managed updates enabled takes the release flow; a release layout with updates disabled **refuses** (phase `managed_updates_disabled`,
  naming the setting) and never falls to the legacy `/opt/saturn-go` deployment. A legacy layout with updates disabled keeps the legacy deployment; with updates enabled the update is refused until it is run
  with the explicit `--migrate-to-releases` (an ordinary update never changes the layout).
* **The flow** (`saturn-release-update-flow.sh`): preflight, resolve the selected GitHub ref to one commit, stop if it is already the active release, build that commit into a bundle on the device
  (`saturn-release-build.sh --expect-commit <that commit>`: if the ref has moved the build stops before any work), validate the manifest, then hand the bundle to the root broker
  (`saturn-release-update-root.py --dispatch`). The broker runs the existing root installer and activator in a transient unit (activating restarts the page's own service): install inactive, switch, restart,
  prove reception with the temporary headless receive-only session, restore the previous release if that fails. It reports success only when the transaction record is `committed` for this commit.
  Prebuilt ARM releases can replace the local compilation later through the same bundle and manifest.
* **Authorization.** `SATURN_MANAGED_UPDATES=keep|enable|disable` is an explicit choice made when the Saturn-Go installer runs (default `keep`). The sudoers rule that lets the service user call the broker is written from the
  resulting policy on every installer run, so disabling removes it. An existing configuration is preserved: an explicitly disabled policy stays disabled unless `enable` is chosen, and the administrator's
  `VALIDATION_MEMORY_MAX` (500M on this G2), timeout, expected RXC1 state and the six validation overrides survive, while unknown or invalid keys are dropped with a note. The 32 MiB reserve is unchanged.
* **Installer on a release layout.** The Saturn-Go installer (`update_manager/install_saturn_go_nginx.sh`) refreshes only what it owns, builds and restarts nothing of the application, and checks readiness against the
  running release. `SATURN_INSTALL_PLAN=1` prints what it would do without root and without changing anything. **The whole-appliance `sudo ./install.sh` is not release-aware; on a release layout it refuses before changing anything and names the Saturn-Go installer** (see below).
* **Receive settings.** The Bridge keeps the receive settings it has accepted in `/var/lib/saturn-bridge/rx-state-v1.txt` (outside every release, versioned, atomic, debounced), restores them before reception starts, and never
  stores or restores PTT, MOX or a transmit session. The Bridge unit must name that state directory (the installer refreshes the unit); until it has, the Bridge runs without persistence.
* **What was verified, and where.** Unit and integration tests on a development machine: the configuration merge, the broker (with stand-ins for the installer and activator and the real lock tool), the flow and the routing,
  the builder's pin, the installer's plan mode for both layouts, and the Bridge's persistence including three end-to-end tests against the real Bridge with WDSP and replayed hardware. Mutation runs over each of these
  (a no-op control must survive; every mutant is killed by a named test). **Nothing of this has run on the G2.**

## What is still open

* **Scope of the managed-update work.** What exists is *managed updates for existing installations*: an installation that already runs releases (this G2) updates through the Update action, with the activation
  policy an explicit installer choice. A fresh installation still deploys the legacy layout, and moving it onto releases is done with the explicit `--migrate-to-releases`; converting the installer's fresh-install path to
  the same release machinery is separate, not yet done work.
* **Not yet run on the G2:** the installer refresh on the release layout (with the choice to enable managed updates), an update through the normal page with no browser attached, the saved receive settings coming
  back after the restart, and the reruns of the installer that must leave the activation policy and the 500M allowance as they are. These are the completion test; `main` stays unchanged until it passes and is reviewed.
* **The whole-appliance installer is not release-aware, and is now safe on a release installation.** `sudo ./install.sh` runs the shared provisioner (packages, build, XDMA driver through DKMS, P2, then Saturn Go and the Bridge); its
  Saturn-Go phase would run `install-saturn-bridge.sh` without unit-only mode (building the Bridge, switching the backend and restarting it) and verify readiness against the checkout's commit rather than the running release.
  `scripts/install-saturn-appliance.sh` therefore refuses on a release layout, with the same layout rule as the Saturn-Go installer, **before** packages, drivers, builds or services are touched (also with `--dry-run`), says what it found and names
  `update_manager/install_saturn_go_nginx.sh`; `SATURN_INSTALL_APPLICATION=force` is the explicit override, and it fails closed when the layout cannot be determined. Delegating to, or converting, the provisioner's Saturn-Go
  phase (the fresh-install conversion) remains separate work.
* A controlled rollback has not yet been exercised on the appliance (the `failure` stage exists for it); the unplanned one, on 14:17, ended in exit 71 and is the reason for the restart-timing waits.

## Evidence (all run on this machine, except where the hardware results above say otherwise)

| what | result |
|---|---|
| validator tests (fake DevTools browser + fake owner + fake unit/cgroup/proc/meminfo, a fake `systemctl kill` that really kills) | **161 pass.** 30 mutations of the rules (sandbox, role, memory by cgroup and by watchdog, headroom, evaluate-all, background tab, audio sink, lean switches, process accounting, evidence): all killed, after closing six gaps they exposed (a tripped watchdog could still be reported "ok" at the end; a browser stopped on purpose left unrelated failures; the cgroup mode and the watchdog mode were not distinguishable; a DevTools failure caused by the memory stop was reported first; and two assertions). 15 tests and 22 more mutations cover the restart-timing waits (the owner's telemetry present, fresh and the service's own; the memory headroom): no wait, an unbounded wait, a wait that ignores a stop request, stale or another process's telemetry accepted after the wait, the wait not reported or always reported, the interruption or the needed amount left out of the detail, passing after the wait, the best reading judged instead of the last, an unreadable reading ignored, defaults of zero: all killed, after closing five gaps they exposed (the two stop tests timed the wrong moment; no test had telemetry of another process, or a minute old, that is right later; none had memory that rises and falls back, or becomes unreadable while waiting; none checked the defaults) |
| activation helper tests (incident control; success with nobody connected; stalled audio; rollback with and without verified reception; policy by release; isolation failure; interrupted helper; reconcile; `--check-reception --memory-max`) | pass; 28 mutations earlier and 6 for the measuring cap and the record: all killed |
| installer for the helper/validator/guard | pass |
| staged procedure (`tests/test-g2-headless-steps.sh`): pins, commit lock, manifest-names-the-pin, the build, helper and check stages with the host's commands stubbed (including `check 500M` and bad sizes), verdict on about 70 variants of evidence | pass; every mutation tried over the rounds (24, then 9 for the manifest check, 8 for the build stage, 8 for the helper stage, 6 for the check size and the verdict rules) killed |
| the same test, `reconcile`, `failure` and `activate` against a stand-in for the root helper that applies the real policy gates (the REAL helper's reconcile gate is run too, in its own non-root test mode on temporary paths: policy off refuses, policy on gets past): policy off / already on, restored byte-identical after success, failure, a stage killed by TERM, INT or HUP, and a restoration that fails; every condition of the `failure` and `activate` verdicts; the next-step hint of `check` (`activate`, or `reconcile` beside a `rollback_failed` record; 3 mutations killed); refusals leave no arm and no change | pass; 45 mutations of the policy handling, the restoration and every verdict condition (policy not switched on in each stage, not restored, hash ignored, a failed or half-done command ignored, a signal trap or the exit trap removed, each `failure` and `activation` condition dropped, `Done` printed anyway, RXC1 wait skipped, arm not published or with the wrong text): 43 killed. The 2 survivors (dropping the explicit TERM or HUP trap) are equivalent on bash 5.1, which runs the EXIT trap on those signals anyway; the explicit traps stay so the behaviour does not depend on the bash version. The no-op control survives. Two earlier passes were discarded because their control failed (a runner started with `nohup` makes SIGHUP ignored, which no stage can trap; a race on a shared lint shim); the test now resets the signal dispositions it needs |
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
* **The `reconcile` stage did not switch the root-owned policy on; `failure` and `activate` had never been run even against a stand-in; the reviewer found two more reporting defects.**
  The first hardware `reconcile` was refused (`reconciliation is disabled by root-owned policy (ACTIVATION_ENABLED)`). Corrected and tested (evidence table): all three stages switch the
  policy on for their one helper run only, hash the configuration first and require it byte-identical afterwards, restore it on success, failure, `die`, INT, TERM and HUP, and report a
  restoration they cannot confirm loudly, with the repair command, and fail. `failure` no longer returns success whatever happened: it requires an exit status that is neither 0 nor 70/71, the
  previous release active, a record of its own (`rolled_back`, failed in the receive validation, the rollback restored and receiving) and the policy restored. `activate` no longer prints
  "Done" when the RXC1 wait expires: after the helper succeeds it requires the new release active, `/readyz` reporting it, a `committed` record whose validation finished `done` as the viewer without
  the legacy fallback, no browser and no connection left, and the policy restored; if all that holds but RXC1 never reads `valid` it says so and exits 3. The new test caught one regression of mine
  on the way: an over-strict pattern for the exercise's extra setting line (no digits allowed, and the name contains `RXC1`) would have stopped `failure` before it started.
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
   Per the instruction, 3 happens only after the source review and a successful `check`. Each of the three switches the root-owned `ACTIVATION_ENABLED` policy on for its one helper run only,
   says so, and restores the configuration byte-identical (or says loudly that it could not). A stage exits 0 only when the outcome is the expected one.

## Completion test of the managed updates on the G2 (not yet run; you run the sudo steps)

State read from the G2 when this was written: it runs `cc09c61` (committed); `ACTIVATION_ENABLED="0"`, `VALIDATION_MEMORY_MAX="500M"`; the working checkout `/home/pi/github/Saturn` is at `d29c5d88` (old, with modified tracked build
outputs; it is only a place the builder fetches into, and is not touched); the Update page's policy resolves `stable` to `main` of `kd4yal2024/Saturn`, and **`main` must not move**, so the test types the review branch into the page's Branch / Ref field (the page saves that field as both `stable_ref` and `custom_ref` and switches the channel to `custom`; afterwards type `main` back and save); the Bridge
unit names only `saturn-rxc1-bridge` as a state directory; RXC1 reads `valid` (left from the bootstrap activation's arm); `saturn-go` runs with `~/.cargo/bin` on its `PATH`.

1. **Refresh what the installer owns** (application and radio untouched; no radio service restarts). From a clean worktree of the commit under test, created beside the checkout, never inside it:
   `git -C ~/github/Saturn fetch https://github.com/kd4yal2024/Saturn.git goldmarsh/headless-validation`, then `git -C ~/github/Saturn worktree add --detach ~/saturn-installer-<sha12> <sha>`.
   Look first (no root, changes nothing): `SATURN_INSTALL_PLAN=1 SATURN_MANAGED_UPDATES=enable bash ~/saturn-installer-<sha12>/update_manager/install_saturn_go_nginx.sh`. It must say: layout *release*
   (active release `cc09c61f…`), application payload NOT deployed, `ACTIVATION_ENABLED` 0 -> 1, `VALIDATION_MEMORY_MAX=500M` kept. Then, as you:
   `cd ~/saturn-installer-<sha12> && sudo env HOME=/home/pi SUDO_USER=pi SATURN_SERVICE_USER=pi SATURN_SERVICE_GROUP=pi SATURN_REPO_ROOT=/home/pi/github/Saturn SATURN_INSTALL_PACKAGES=0 SATURN_INSTALL_LEGACY_XDMA_HOOK=0 SATURN_MANAGED_UPDATES=enable bash update_manager/install_saturn_go_nginx.sh`.
   (Those variables are what the whole-appliance provisioner gives this installer: no apt, no legacy XDMA hook. `SATURN_P2APP_PANEL_ENABLED` defaults to 0, which is this G2's value.) I then check, read-only: the same PIDs for
   `saturn-go` and `saturn-bridge`, the configuration (`ACTIVATION_ENABLED="1"`, `500M`), the broker's sudoers rule, `/opt/saturn-go/scripts/update-saturn-go.sh` and the builder beside it, and the Bridge unit's state directory.
   Also, as you or I without root: from the same worktree `bash ./install.sh --dry-run` must refuse with "this appliance runs managed releases (active release `cc09c61f…`)" and name the Saturn-Go installer, having changed nothing.
2. **Update 1, through the page.** Close every Saturn Remote tab. On the Saturn Go page (`/saturngo`) type `goldmarsh/headless-validation` into **Branch / Ref** in the Saturn Go policy panel (the page has no channel or custom-ref selector; it saves that field as the policy's ref), tick **Dry run (no changes)** and press **Run Saturn Go Update** first: it
   prints the preflight and the one commit the ref resolves to, and changes nothing. Then untick it and press **Run Saturn Go Update**. The build runs on the G2 (about 1.5 hours; the radio keeps running). Expected:
   the ref resolved once to the branch tip, that commit built, the broker's unit installs, switches and restarts, the headless check runs as the viewer, the browser and profile are gone, the record is `committed` for that commit,
   and the page shows success. RXC1 is recorded, and not required (`VALIDATION_EXPECT_RXC1` is `any`); it is not expected to read `valid` again without an arm. **The tuning resets once here**: the running `cc09c61` Bridge has no
   persistence, so it saved nothing to restore.
3. **Set a recognisable tuning** in Saturn Remote and wait 25 seconds. I check `/var/lib/saturn-bridge/rx-state-v1.txt` (private, `version=1`, receive fields only, no transmit field).
4. **One controlled failure.** I push a later commit to the branch (the first update already built the commit it resolved, so this also shows the pin). You add `VALIDATION_EXPECT_RXC1="valid"` to `/etc/default/saturn-release-activate` with `sudoedit`
   (after a backup; only if step 2 left RXC1 reading something else) and press Update. Expected: that commit is built, installed and started, the target validation fails at `rxc1.intended_state`, the previous release is restored
   automatically and validated, the record is `rolled_back` with `release_restored` and `receive_verified` both true, the page shows the rollback and its reason, and the Bridge comes back **with the tuning from step 3**. Then you remove the line
   (the installer keeps administrator keys, so a forgotten test line would stay) and I compare the file with the backup.
5. **Update 2.** Press Update again with the override gone: the bundle from step 4 is reused (no build), the update succeeds, and the tuning is still the one from step 3.
6. **Rerun the installer** with the default choice (`SATURN_MANAGED_UPDATES` unset, so `keep`): the policy, the 500M allowance and the broker's rule are as they were, and no radio service restarts. Then, optionally, with `SATURN_MANAGED_UPDATES=disable`: the broker's rule is gone
   and the Update action refuses at once (phase `managed_updates_disabled`, nothing built or changed); `enable` restores it.

Only after this, and a review of the evidence, is the branch merged to `main`. The matched audio comparison is separate and still to come.

### What the first page-driven attempt found (2026-10-11 19:20 EDT)

The first press of **Run Saturn Go Update** stopped at the very first step, resolving the ref: `required command not found: cargo`. Nothing was built, installed or changed (the record, the configuration, the services and the
release pointer were as before, and the status panel said so: phase `resolve`, mode `release`). The cause: the page starts every script with a fixed system-only `PATH` (`apply_build_subprocess_env` in the server), and
`cargo` lives in the build user's `~/.cargo/bin`; the legacy deployment finds it itself, and the release branch left before that code. I had checked the `saturn-go` service's own `PATH` (which includes `~/.cargo/bin`) and not the
environment the page gives its children. Fixed by `prepare_build_toolchain_env`, shared by both deployments, and by running the flow's tests under the page's exact `PATH`. Also read on the G2 under that `PATH` plus `~/.cargo/bin`:
every command the builder needs is present (`cmake` is not, and is not needed), the installed builder resolves the branch to the right commit and honours `--expect-commit`, the service has no write restrictions on the paths the build uses, and the
sudo-to-broker boundary refuses a non-existent bundle cleanly (`--dispatch` of a bundle without a manifest: exit 2, recorded).

### What the second page-driven attempt found (2026-10-11 19:46 to 20:04 EDT)

With the toolchain fixed, the real run resolved `b35bde58c871…` and built for 18 minutes, then stopped in the Rust server's test gate: six `remote_tls` route tests expected 200, 307 or 404 and got 401. Nothing was installed or changed (release
pointer, configuration hash, services and the transaction record as before). Cause: the build inherited the `saturn-go` service's environment, which holds `SATURN_REMOTE_BASIC_AUTH`; the remote router reads that variable and then requires
credentials, and the tests send none. The earlier builds on the G2 ran from an ssh shell, where it is not set. Reproduced here: the same six tests pass without the variable and fail with it. Beyond the tests, the build scripts and every npm package
could have read that credential. The flow now starts the builder with `env -i` and an allow-list (the toolchain, locale, `PKG_CONFIG_*`, line-buffering for the live output, `SATURN_RELEASE_*`, `SATURN_SATURNGO_BUILD_*`, `SATURN_WDSP_*`; git gets
`GIT_TERMINAL_PROMPT=0` from the flow), so a release is built in the environment the earlier successful builds had, not in the service's. The Rust server's tests are still sensitive to that variable when run by hand in a shell that has it; that is
noted, not changed here (it is in the release payload).

### What the third page-driven attempt found (2026-10-11 23:12 EDT): the activation itself, and a rollback_failed record

The build passed every gate this time (the server's 155 tests, 761 web tests, the Bridge's 492 including the `rx_state` ones) and the bundle was installed inactive. The activation then failed at *readiness* and the rollback was recorded as
failed (exit 70, `rollback_failed`, `receive_verified: not_attempted`), yet the radio was running `cc09c61` healthy afterwards. **Cause (mine):** the broker held the `radio` maintenance lock for the whole run. `saturn-go`'s own startup runs its
restore recovery under the `repository` and `radio` locks and exits with status 1 when either is busy, so the target's `saturn-go` could not start (journal: `failed to recover incomplete restore transaction: ... resource is busy: radio`, then a restart
loop) and neither could the previous release's during the rollback; both started within a second of the broker exiting (23:13:28), which is why the radio is fine now. The target's Bridge itself started cleanly (`no saved receive settings ... starting at the defaults`).
The staged procedure never held that lock, so the bootstrap never met it. Fixed: the broker holds `release` only, and checks `radio` and `repository` are free at the start and just before the activation. The test activator now does what `saturn-go`'s startup does
(takes `repository` and `radio` with the real lock tool and exits 70 if busy), so every broker test guards this. What it leaves unchecked: another maintenance job that starts between the last check and the restart; the page is the only
client of those locks and is down while the services restart.

State left on the G2: release `42ff0919…` installed inactive (its bundle is staged), `cc09c61` active and ready, and the transaction record `rollback_failed`. A record in that state blocks further activations until it is reconciled
(`saturn-release-activate-root.sh --reconcile-rollback`, which verifies the previous release is running and receiving and preserves the original record beside the new one).

## Not verified / residual risk

* **The restart-timing waits are bounded, not removed.** If the host's memory never climbs back to cap + 32 MiB within 30 s, or the owner never writes telemetry, the validation still fails (and an activation rolls back);
  the reason is in the evidence.
* **The Pi's Chromium fits, with little room.** It fits under a 500M cap (peaks 413 and 448 MiB) with 551 to 586 MiB available, and does not under 350M. The peak varies from run to run (the
  aborted 350M run read 366 MiB at 8.6 s, still starting), the margin is about 50 MiB, and a launch needs the cap plus 32 MiB free, so an activation at a busier moment can be refused
  at launch; that counts as unproven and rolls back. The sandbox does hold inside the locked-down unit (`Seccomp: 2` on every renderer).
* The kernel offers no pressure-stall information on this Pi (`/proc/pressure` is absent), so the diagnostics use CPU, I/O wait, swap and fault counters instead.
* `MemoryPeak` needs systemd >= 255 and a memory controller; the peak is otherwise the validator's own measurement (proportional set size), and the evidence names which.
* On the old Bridge the operator slot may be held by the media connection of a validation session for ~15 s; a person connecting in that window would be a viewer until it ends.
  Nothing in the new release has this: its viewer-declaring session releases the slot (`dfb0b19`).
* The validator's own rehearsal sees only the replay's `disabled` RXC1. `valid` was observed on a live owner in the October 9 trial and again, at the validator's baseline, in the first activation attempt
  of the candidate release; the `failure` stage deliberately uses an RXC1 expectation the target cannot meet.
* ARM binaries come only from the G2's own native build of the pushed commit; none were built here.
