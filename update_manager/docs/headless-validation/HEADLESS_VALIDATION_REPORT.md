# Automatic headless Saturn Remote validation in the supported update process

GoldMarsh, 2026-10-10. Branch `goldmarsh/headless-validation`, built on `f10a087` (Stage A) and the live-driver tools.
FPGA 1.31.002 / build ID `0x53460004` and the XDMA kernel driver are untouched. **Nothing on the G2 has been changed by this work**: it is still on release
`329f3109`, with the `rollback_failed` record exactly as the first activation left it.

This file and `g2_headless_steps.sh` live in the source tree (`update_manager/docs/headless-validation/`) so that nothing depends on a scratch directory. The commit
under test is the branch head; the hand-off message gives its full SHA, and the procedure refuses to build or run anything else (`PIN=<that SHA>`).

## What was wrong

The activation gate needs an active receive client at the owner (`state.sdr_active`), which exists only while something is receiving. With every browser closed nothing
attached one, so the readiness check failed, and so did the rollback's identical check. The previous release was in fact restored (proved from the helper's own nine
conditions); the helper recorded `rollback_failed` and exit 70 because "restored" and "receiving" were one check.

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
* **the browser's sandbox is in force and its unit carries the memory cap, read from the host (below);**
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
stage refuses a different one. The three hashes it pins (helper, validator, guard) are those of the committed blobs, and `tests/test-g2-headless-steps.sh` fails if they drift.

## Evidence (all run on this machine; the G2 has run none of it)

| what | result |
|---|---|
| validator tests (fake DevTools browser + fake owner + fake unit/cgroup/proc) | 110 pass; 27 mutations of the new rules (sandbox, memory, role policy, cmdline reading, peak, defaults): all killed after closing two gaps (an unreachable guard re-check was deleted; the fake systemctl now records which unit was queried) |
| activation helper tests (incident control; success with nobody connected; stalled audio; rollback with and without verified reception; policy by release; isolation failure; interrupted helper; reconcile) | pass; 28 mutations of the new logic: all killed after adding a near-miss-commit case and a reconcile-summary assertion |
| installer for the helper/validator/guard | pass |
| staged procedure (`tests/test-g2-headless-steps.sh`): pins, commit lock, verdict on about 50 variants of evidence, the whole `check` stage with stubbed host commands | pass; 24 mutations: all killed |
| real components on loopback: replay Bridge + saturn-go proxy (TLS, basic auth, pinned certificate) + the real page + **real Chrome 153 with its sandbox ON** | **59/59 checks.** New page + fixed Bridge; old page + fixed Bridge; display data stopped mid-measurement (the exact failed condition is kept, the browser and connection are gone); a real Chrome started with `--no-sandbox` is refused; old page + **old Bridge** (the rollback target's situation): the operator role was seen once and refused under `required`, and seen once and accepted as a recorded fallback with the guard proven active, 107 frames all on the allowlist and the radio unchanged. In every sandboxed run the real Chrome showed 5 renderers, all `Seccomp: 2`. |
| shellcheck on every shell file CI checks | clean (see "Found in my own earlier push") |
| Bridge (unchanged since `dfb0b19`): 462 tests; web: 761 tests, typecheck, seam check | pass at the earlier head; no Bridge or web file changed since |

## Found in my own earlier push (`4762a81`)

* `tests/test-saturn-release-activate.sh` as pushed would have failed the repository's shellcheck gate (SC2012/SC2034/SC2123/SC2030/SC2031). Fixed; the new files are in the
  CI lists and the validator, installer-guard and procedure tests now run in CI.
* `cargo fmt --check` fails on the Stage-A Bridge sources (`client.rs`, `command_queue.rs`, `tci/tests.rs`, `tx_thread.rs`, `xdma_backend.rs`, `tests/*.rs`) at `f10a087` already;
  `main` is clean. I have **not** touched them (a formatting-only commit would change Bridge source on the way to the appliance). It would fail the CI format step if this branch
  were opened as a pull request; say so if you want that commit.

## Sequence on the G2 (you run the sudo stages; I read the evidence; ~nothing is restarted before `activate`)

0. Get the procedure: `curl -fsSL https://raw.githubusercontent.com/kd4yal2024/Saturn/<SHA>/update_manager/docs/headless-validation/g2_headless_steps.sh -o ~/g2_headless_steps.sh`
   (or I copy it from the pushed source), then `PIN=<SHA> bash ~/g2_headless_steps.sh build`.
1. `validate`, `install` (inactive), `helper` (activator + validator + guard, each pinned by SHA-256; activation stays disabled).
2. **`check`**, all browser tabs closed: the production launcher on the running `329f3109`; changes nothing. Success prints `OK: reception, sandbox, memory cap and cleanup are all verified`.
   Because `329f3109` is a legacy release it is expected to print `LEGACY BRIDGE FALLBACK USED` or a plain viewer session (the old Bridge's race); either is acceptable, and it says which.
3. `reconcile`, then optionally `failure` (a real activation meant to fail validation, rolling back to `329f3109`), then `activate` (the normal update, RXC1 arm written last, all browsers closed).
   Per the instruction, 3 happens only after the source review and a successful `check`.

## Not verified / residual risk

* **Nothing has run on the G2.** Chromium's sandbox inside the locked-down transient unit (dynamic user, `NoNewPrivileges`, no `/home`) and the 350 MB cap on a 905 MB Pi are exactly what
  `check` settles. If the sandbox cannot start there, the check fails and I will change the unit's properties, not switch the sandbox off.
* The memory peak will be known only from the G2. `MemoryPeak` needs systemd ≥ 255; if absent the peak is reported as unavailable and nothing else is affected.
* On the old Bridge the operator slot may be held by the media connection of a validation session for ~15 s; a person connecting in that window would be a viewer until it ends.
  Nothing in the new release has this: its viewer-declaring session releases the slot (`dfb0b19`).
* `valid` RXC1 has never been observed against a real owner, only the replay's `disabled`; the `failure` stage deliberately uses an RXC1 expectation the target cannot meet.
* ARM binaries come only from the G2's own native build of the pushed commit; none were built here.
