# Saturn Reliability Remediation Plan

**Plan branch:** `remediation/reliability-hardening`

**Baseline:** `a186532e7fb29815af05d3502645920f2412a8af`

**Created:** July 18, 2026

**Primary goal:** Improve update, restore, state, and job reliability without turning a single-operator radio appliance into an enterprise security platform.

## How to use this plan

- Check an item only after its acceptance criteria and required tests pass.
- Add the implementing commit or pull request beside the item when it is completed.
- Do not combine deployment activation, restore, persistent jobs, and FPGA changes in one change set.
- Keep the previous working behavior available until the replacement has passed appliance testing.
- Record intentional product decisions here so they are not repeatedly treated as accidental defects.

Status markers:

- `[ ]` Not started
- `[~]` In progress
- `[x]` Complete and verified
- `[D]` Deferred
- `[A]` Accepted product risk; no change planned

## Product decisions and threat model

These decisions are approved constraints for this remediation effort:

- `[A]` Saturn is normally operated by one owner on a trusted amateur-radio LAN.
- `[A]` Remote access is supported through Tailscale. Direct Internet port forwarding is unsupported.
- `[A]` The administrator password minimum and generated-password length remain five characters. Longer passwords remain optional.
- `[A]` RF transmission remains enabled by default for normal Saturn Remote operation.
- `[A]` Remembered-device login may remain valid for one year to avoid recurring password burden.
- `[x]` The appliance updater will use the **Deployment model**: stage, build, install, activate, restart, verify the target commit, and automatically roll back on failure.
- `[x]` Browser-controlled SD-card imaging, cloning, and target wiping will be disabled. Disk imaging remains a documented manual maintenance function outside Saturn Go.
- `[x]` Normal application deployment will not build or flash FPGA artifacts. FPGA work requires a firmware maintainer and remains an explicit separate operation.

Existing protections that must not regress:

- Internal bridge and management backends remain loopback-only unless explicitly configured otherwise.
- Tailscale is the recommended remote-access boundary.
- TX watchdog, automatic unkey, controller ownership, and RF-safety behavior remain active.
- Privileged operations continue through root-owned installed helpers rather than writable repository scripts.
- Password changes invalidate remembered-device authentication.

## Scope boundaries

### Normal application release

A normal Saturn application release should contain:

- `saturn-go`
- `saturn-bridge`
- Saturn Remote browser bundle
- Saturn Go templates and static assets
- `p2app` and required user-space radio tools
- Versioned installed maintenance helpers required by that release
- A release manifest containing source commit, build time, component versions, and hashes

### Separate maintenance domains

The following are not silently changed by a normal application deployment:

- FPGA images or FPGA flashing
- XDMA kernel module and kernel/header changes
- Operating-system distribution upgrades
- Boot firmware configuration
- LCD or front-panel hardware configuration
- Whole-disk imaging or cloning

## Milestone 0 — Baseline and operator contract

### REM-0001: Publish the supported exposure model

- [x] Document LAN and Tailscale as supported access methods.
- [x] State that direct Internet port forwarding is unsupported.
- [x] Document that raw Protocol 2 and raw TCI are trusted-LAN interfaces.
- [x] Add firewall/Tailscale guidance without requiring enterprise identity systems.

Acceptance criteria:

- Installation and operations documentation give one consistent network-exposure recommendation.
- No page implies that port 8443 or raw port 50001 should be forwarded from the public Internet.

### REM-0002: Reconcile intentional authentication and RF behavior

- [x] Document the five-character minimum as an intentional usability decision.
- [x] Document RF TX as enabled by default.
- [x] Display the effective RF gate prominently in Saturn Remote.
- [x] Preserve optional longer passwords and the existing password-change function.

Acceptance criteria:

- README, architecture, runbook, UI, installer defaults, and tests agree.
- A clean install clearly tells the operator whether RF transmission is permitted.

### REM-0003: Disable browser whole-disk imaging and cloning

- [x] Remove image/create/download and clone/wipe controls in Saturn Go.
- [x] Reject the corresponding API routes with an explanatory `410 Gone` response.
- [x] Remove unnecessary imaging/clone/wipe sudo permissions and clean up legacy installed helpers.
- [x] Document a manual command-line imaging procedure with device-selection warnings.

Acceptance criteria:

- Saturn Go cannot start an SD-card clone or target-device wipe.
- Existing backup and repository-restore functions remain available.
- The manual process requires explicit device identification and confirmation.

## Milestone 1 — Health and release identity

### REM-0101: Split liveness from readiness

- [x] Add `/livez` for process liveness.
- [x] Add `/readyz` with structured component results.
- [x] Preserve `/healthz` temporarily as a compatibility alias with a documented removal plan.

Minimum readiness checks:

- Running release commit matches the expected release manifest.
- Mandatory state paths exist and are writable.
- Required configuration parses successfully.
- Free disk space exceeds configured safety thresholds.
- `saturn-bridge` is reachable.
- P2 service state is reported.

XDMA/radio hardware state should be reported separately from application readiness so an application deployment does not roll back merely because external radio hardware is temporarily unavailable.

Acceptance criteria:

- `/livez` remains successful during a dependency failure.
- `/readyz` fails with a component-specific reason during a dependency, state, disk, or release-identity failure.
- Readiness includes the full expected Git commit.

### REM-0102: Use readiness consistently

- [x] Installer verifies `/readyz` rather than unconditional `/healthz`.
- [ ] Deployment and rollback require the expected commit from `/readyz`.
- [x] Watchdog distinguishes process death from dependency/readiness failure.
- [x] Saturn Go overview displays readiness and installed commit.

The Saturn Go root deployment broker now requires the staged full commit from
`/readyz` and rolls back a wrong binary. The remaining unchecked deployment
item is the legacy appliance source-worktree updater in `update.rs`; it will be
replaced by the versioned Deployment model in Milestone 2 rather than being
given another source-root health workaround.

Acceptance criteria:

- A healthy old process cannot validate a newly staged commit.
- A wrong-commit response causes deployment rollback.

## Milestone 2 — Versioned Deployment model

Target layout:

```text
/opt/saturn/releases/<full-commit>/
/opt/saturn/current -> /opt/saturn/releases/<full-commit>
/var/lib/saturn-state/
/var/log/saturn/
```

### REM-0201: Define and build a release bundle

- [x] Run the low-memory build preflight before Rust compilation and require the configured disk-backed build swap on memory-constrained appliances.
- [x] Use bounded Rust parallelism (`CARGO_BUILD_JOBS=1` by default on the supported Pi appliance).
- [x] Define the release manifest schema.
- [x] Build all normal-release components from one exact commit.
- [x] Record component hashes and build results.
- [x] Run non-hardware unit, parser, native boundary, and web-bundle tests before installation.
- [x] Fail without changing the active release when any build or test fails.

Completed on the supported 1 GiB Pi appliance at commit
`acf99c35d28da89109a595b3839bd8a8ffaa6ff0`. The builder produced and
independently validated an inactive schema-v1 bundle containing 17 declared
components, 75 files, and 11 passed build/test gates. The installed appliance
remained healthy on its prior commit throughout the build and failed assembly
attempts removed their incomplete staging directories.

Acceptance criteria:

- All installed components report or map to the same source commit.
- A deliberately broken staged build leaves the current release untouched.

### REM-0202: Install into an immutable versioned directory

Completed on the supported appliance and revalidated with inactive release
`3aed204a78d5360fd03610e7f9ad2323afeeef41`. The complete manifest-validated
bundle was installed at `/opt/saturn/releases/<full-commit>` as `root:root`;
no entry is group/world writable and every directory is mode `0755`.
`/opt/saturn/current` remained absent, no service restart occurred during the
inactive install, and `/readyz` continued to report the expected commit with
Saturn Go, Saturn Bridge, P2, state, disk, and XDMA checks healthy.

Appliance validation exposed and fixed two permission-boundary defects: the
canonical installer no longer rewrites completed staging payload modes, and
the builder/manifest/installer now normalize and reject writable or
non-traversable release content.

- [x] Install the release into `/opt/saturn/releases/<full-commit>`.
- [x] Ensure installed release files are not writable by the Saturn Go service user.
- [x] Keep mutable settings, credentials, jobs, and logs outside release directories.
- [x] Validate ownership, permissions, manifest, and required files before activation.

Acceptance criteria:

- The current release remains runnable while the new release is built and installed.
- Saturn Go cannot modify installed release executables or helpers.

### REM-0203: Atomically activate and verify a release

The code-only activation transaction is implemented and covered by an
isolated fixture test. It writes an atomic prepared transaction, installs
stable-pointer systemd overrides, atomically switches `/opt/saturn/current`,
uses dependency-aware stop/start ordering, and commits only after `/readyz`
accepts the exact target commit. Production activation remains deliberately
disabled in root-owned configuration until the REM-0204 rollback transaction
passes a separately approved live appliance test; no live service or active
pointer was changed while implementing this item.

- [x] Persist a prepared deployment transaction before activation.
- [x] Atomically change `/opt/saturn/current` on the same filesystem.
- [x] Restart only affected services in a defined order.
- [x] Require `/readyz` to return the target full commit.
- [x] Mark deployment committed only after readiness succeeds.

Acceptance criteria:

- Activation exposes either the complete old release or complete new release.
- No mixed-release file set is possible.
- Live appliance acceptance is deferred until the REM-0204 implementation is
  installed and exercised in a separately approved rollback test.

### REM-0204: Automatically roll back failed activation

The code-only rollback transaction is implemented and covered by isolated
failure-injection tests. Before activation it records the prior pointer, exact
ready commit, service activity, and byte-for-byte copies of existing systemd
drop-ins. Any post-prepare failure stops affected services, atomically restores
the old pointer (or the original legacy no-pointer state), restores the prior
drop-ins, restarts only services that were previously active, and verifies the
old exact commit before recording success. A rollback that cannot be fully
verified is recorded as `rollback_failed` and blocks another transaction.

The activator performs no release pruning, so the active release and every
installed prior release remain available. Disk-aware pruning may be added
later, but it must never remove the active release or the two newest prior
verified releases.

- [x] Retain the prior active-release pointer identity and configuration snapshot until commit.
- [x] Switch back and restart services when activation or readiness fails.
- [x] Persist the failure reason and rollback result.
- [x] Keep at least the current release and two prior verified releases, subject to disk-space policy.

Acceptance criteria:

- Wrong commit, startup failure, readiness timeout, and invalid configuration all restore the prior verified release.
- The operator can see the failed target and rollback reason after Saturn Go restarts.
- Isolated acceptance tests pass. Production activation remains disabled until
  an explicitly approved live appliance rollback test is completed.

### REM-0205: Define state compatibility

The code-only state contract is implemented and covered by isolated fixtures.
New schema-v2 release manifests declare the state version they write, the
versions they can read, the permitted migration, its one-way status, and the
exact direct files managed below `/var/lib/saturn-state`. Existing schema-v1
release manifests map to explicit legacy state schema 0 so installed rollback
releases remain usable.

Before a migration, the activator stops state-writing services and creates a
checksummed, mode-preserving backup under
`/var/lib/saturn-state/deployments/state-backups/`. It changes the version
marker only after the backup is durable. A failed migration cannot switch the
active pointer; a later activation/readiness failure restores the state backup
before the prior services restart. Undocumented rollback-unsafe migrations
fail closed, while a documented one-way migration additionally requires the
operator-only `--approve-one-way-migration` flag and records that approval.

- [x] Version persistent state schemas.
- [x] Back up state before migration.
- [x] Require a release to read its own and the immediately preceding supported state version.
- [x] Block deployment when rollback would become unsafe unless the operator explicitly approves a documented one-way migration.

Acceptance criteria:

- Upgrade and immediate rollback preserve settings.
- Migration failure does not activate the new release.
- Isolated acceptance tests pass. Production activation remains disabled; no
  live pointer or service was changed for REM-0205.

## Milestone 3 — Transactional backup and restore

### REM-0301: Inventory irreplaceable state

- [x] Inventory administrator credentials, remembered-device data, profiles, radio settings, calibration, network/Tailscale state, LCD/front-panel configuration, custom scripts, boot configuration, and deployment history.
- [x] Identify which data is portable between appliances and which is device-specific.
- [x] Exclude secrets from ordinary support bundles.

The reviewed inventory is maintained in the machine-readable
`update_manager/release/state-inventory-v1.json` contract and the operator
reference `update_manager/docs/STATE_INVENTORY.md`. It separates portable
settings, review-before-transfer radio data, same-device identity/recovery
state, reproducible payloads, external hardware state, and diagnostics. It
also records the exact limits of the current repository, client-source, state
migration, and manual whole-disk backup mechanisms.

REM-0301 is documentation and contract validation only. It does not add a
support-bundle endpoint, change backup contents, change live host state, or
make the current restore transactional. Those changes remain REM-0302 and
REM-0303.

Acceptance criteria:

- The backup documentation states exactly what each backup contains and omits.
- The inventory contract test covers every REM-0205 managed state file,
  requires all reviewed state classes, and prevents secret content from being
  classified for ordinary support-bundle inclusion.

### REM-0302: Separate backup types

- [x] Define a small settings backup.
- [x] Define a source/release backup.
- [x] Document whole-disk images as a manual disaster-recovery procedure.
- [x] Stop referring to a repository archive as a complete appliance backup.

`GET /backup_settings` now exports a schema-v1 settings archive with a
checksummed file manifest and explicit secret/identity exclusions. It includes
managed Saturn settings, registered operator-authored scripts, and direct
piHPSDR/deskHPSDR property files. Regular-file, per-file, and total-size checks
keep the selection bounded and reject redirected content.

`GET /backup_source` accurately names the complete active repository archive;
the historical `/backup_full` route remains a compatibility alias only.
`GET /backup_releases` and `/backup_release?commit=<full-commit>` enumerate and
export individual manifest-bearing immutable releases without mutable state.
The UI presents all three outputs separately and labels existing in-place
repository restore as legacy/nontransactional.

The exact format, content, omissions, sensitivity, and restore availability
are specified in `update_manager/docs/BACKUP_FORMATS.md`. Whole-disk imaging
remains a manual, local-console, same-device disaster-recovery operation.
REM-0302 does not enable settings/release archive import and does not change
live release activation; REM-0303 owns transactional restore.

### REM-0303: Make repository/settings restore transactional

- [x] Restore into a unique sibling staging location.
- [x] Validate content, schema, ownership, permissions, and free space.
- [x] Flush the completed restore before activation.
- [x] Activate through an atomic pointer/rename where supported.
- [x] Retain the prior data until post-restore validation succeeds.

Acceptance criteria:

- Process termination, power interruption simulation, and ENOSPC leave the old or new complete state, never a mixed tree.
- Recovery state remains understandable after reboot.

Settings restore now validates the schema-v1 manifest and exact payload set,
stages checksummed old/new copies, writes a durable transaction journal, and
atomically replaces each bounded managed file. Host-specific repository and
update policy is opt-in. Source restore creates and flushes a unique repository
generation below the state root and atomically switches `repo_root.txt`; the
prior checkout is retained. Saturn Go runs transaction recovery before loading
the pointer or accepting requests. `GET /restore_status` exposes recent durable
records. Fault-injection tests cover ENOSPC, hard process termination during a
partially applied settings transaction, and interruption after a source pointer
switch, proving recovery returns to a complete old state while successful runs
expose a complete new state.

## Milestone 4 — Durable state and job controller

### REM-0401: Centralize atomic state writes

- [x] Write to a same-directory unique temporary file.
- [x] Apply owner and mode before activation.
- [x] Flush file contents, rename atomically, and flush the directory.
- [x] Preserve a last-known-good record for critical policy and deployment state.
- [x] Fail readiness on malformed mandatory state rather than silently replacing it with defaults.

Apply to:

- Release pointer and deployment transaction
- Update policy and update history
- Job records
- Remote settings and profiles
- Custom-script registry
- Deployment status

Acceptance criteria:

- Fault-injection tests expose only the complete old or complete new document.

Saturn Go now routes repository pointers, both update policies, update history,
remote settings/profiles, the custom-script registry, and custom script content
through one Rust atomic state-store implementation. The writer uses a unique
same-directory file, fixes and verifies its mode and owner continuity before
exposure, flushes content, renames, and flushes the parent directory. Validated
critical documents also receive a `<name>.last-good` copy. Shell deployment
status uses the companion bounded JSON writer, while the root release journal
retains the same last-known-good guarantee. Future persistent job records in
REM-0403 are required to use this state-store contract.

`/readyz` now includes a required `state_documents` component. A missing or
malformed repository pointer, custom-script registry, or update policy blocks
readiness; malformed optional settings, history, deployment, or schema files
also block readiness when present. Policy loaders no longer silently replace
malformed JSON with defaults. Rust unit tests and
`tests/test-saturn-atomic-state-write.sh` inject failures on both sides of the
rename and verify that only a complete old or complete new document is visible.

### REM-0402: Add host-level maintenance locking

- [x] Use a host-level lock independent of the Saturn Go process.
- [x] Define resource classes: release, repository, disk, FPGA, package, network, radio, and read-only.
- [x] Prevent conflicting operations before starting a child process.
- [x] Keep FPGA and disk-image operations outside normal application deployment.

Acceptance criteria:

- Restarting Saturn Go does not permit a second conflicting job.

Saturn maintenance now uses fixed-order advisory locks under
`/run/lock/saturn-maintenance`. The trusted lock broker owns the locks and
waits for the maintenance child, so a running script remains exclusive even
if the Saturn Go process that launched it restarts. Saturn Go probes the same
resources before spawning to return an immediate HTTP conflict; the broker
then acquires them authoritatively to close the check/start race. Appliance
updates and rollbacks that execute in-process use a separate broker holder
whose lifetime is tied to the server task.

The resource policy is intentionally narrow: application deployment claims
`release`, `repository`, `package`, and `radio`; it never claims `disk` or
`fpga`. FPGA flashing claims `fpga` plus `radio`, disk imaging remains disabled
in the web appliance, and unrecognized operator scripts conservatively claim
all mutating resource classes. Restore, legacy repository restore, Tailscale,
and every configured/custom script enter through the same broker. Read-only
operations use shared locks. The installer creates the root-owned runtime lock
directory and its service-group-writable lock files through systemd-tmpfiles.
`tests/test-saturn-maintenance-lock.sh` verifies cross-process conflict,
independent resources, shared read-only access, and lock release.

### REM-0403: Persist and reconcile jobs

- [x] Persist job ID, type, state, resources, requester, timestamps, child scope/PID, output path, and result.
- [x] Run maintenance children in named systemd scopes or equivalent process groups.
- [x] Reconcile incomplete jobs on Saturn Go startup.
- [x] Report interrupted jobs and required recovery steps.

Acceptance criteria:

- A Saturn Go restart retains accurate job status and exclusivity.
- Orphaned and interrupted jobs are detected deterministically.

Maintenance records now live under
`/var/lib/saturn-state/maintenance-jobs`. Script children run in a dedicated
process group owned by the REM-0402 lock broker. The broker tees output to a
durable log and atomically writes a completion result even if Saturn Go is
restarted. `/maintenance_jobs` reconciles and reports completed, orphaned, and
interrupted records with operator recovery steps; `/run_log` falls back to the
durable record after a server restart. Appliance updates publish the same
durable identity and terminal result contract, with an in-process child scope
that is deterministically marked interrupted if the controller exits.
`tests/test-saturn-maintenance-jobs.sh` verifies broker output/result durability,
exit propagation, and restrictive file modes; Rust tests cover startup
reconciliation and process identity checks.

### REM-0404: Graceful shutdown and cancellation

- [x] Replace direct `process::exit` with the normal shutdown signal.
- [x] Stop accepting new jobs before shutdown.
- [x] Define which jobs may finish and which must be safely cancelled.
- [x] Terminate the complete process group when cancellation is supported.

Acceptance criteria:

- Shutdown during a maintenance job never silently loses job state.
- No unmanaged child remains after an acknowledged cancellation.

Saturn Go now closes a process-wide maintenance admission gate before handling
an API, SIGINT, or SIGTERM shutdown request. `GET /shutdown_status` reports the
gate and active job policy. Transactional updates, restores, deploys, flashes,
network changes, password changes, Tailscale actions, and unclassified operator
scripts conservatively finish. Only the installed log/backup cleanup helpers
and read-only G2 version report declare cancellation support. Cancelled script
jobs receive SIGTERM and then SIGKILL after a bounded grace period for the
complete REM-0403 process group, and are durably recorded as `cancelled` before
the server exits. The generated systemd unit uses `KillMode=mixed` so SIGTERM
reaches the controller first, with a 15-minute stop ceiling for finish-policy
jobs. Rust tests cover policy selection, duplicate admission identity, and
complete process-group termination.

## Milestone 5 — Resource limits without blocking appliance workflows

### REM-0501: Replace the global 2 GiB request limit

- [x] Apply small limits to JSON and settings routes.
- [x] Apply a separate small limit to custom-script arguments.
- [x] Retain a configurable streaming limit only for routes that genuinely upload restore data.
- [x] Confirm browser clone removal eliminates any clone dependency on the global limit.

Initial limits to validate:

- JSON/configuration: 64 KiB
- Custom-script arguments and metadata: 64 KiB
- Restore upload: configurable, streamed, and checked against disk reserve

Acceptance criteria:

- Oversized ordinary requests fail early with HTTP 413.
- A supported restore upload streams without buffering the whole body in memory.

Completed in REM-0501. Saturn Go and nginx now reject ordinary JSON/configuration
and custom-script requests above 64 KiB with HTTP 413. Only the three restore
archive routes receive the configurable large-body allowance, and nginx disables
request buffering for those routes. Multipart chunks are written directly to a
private temporary file while enforcing both the upload ceiling and the readiness
disk reserve. Staging lives under Saturn's persistent state filesystem instead
of the appliance's small `/tmp` tmpfs; multipart framing errors are no longer swallowed. Browser imaging,
cloning, target enumeration, and wiping remain removed, with only small 410 Gone
compatibility tombstones in the backend. Rust boundary tests cover early 413,
the independent restore allowance, and chunked restore ingestion.

### REM-0502: Bound script output and duration

- [x] Replace unbounded output channels with bounded channels.
- [x] Cap retained output by bytes and lines.
- [x] Emit an explicit truncation/backpressure message.
- [x] Define per-job default and maximum deadlines.

Acceptance criteria:

- A noisy or hung helper cannot cause unbounded Saturn Go memory growth.

Completed in REM-0502. Script and Tailscale SSE streams use bounded 128-entry
channels and report omitted live events when a client cannot keep up. Per-script
resume state retains at most 1 MiB and 5,000 lines; durable broker logs retain at
most 4 MiB and 5,000 lines and end with an explicit truncation marker. Routine
scripts default to 30 minutes, update scripts default to four hours, and an
optional `deadline_seconds` request is clamped to a six-hour maximum. Tailscale
helpers have a ten-minute deadline. Deadline expiry terminates the complete
maintenance process group and records a durable `timed_out` result.

### REM-0503: Bound remote client resources

- [x] Set a documented global authenticated-client limit.
- [x] Preserve one controller/TX owner.
- [x] Bound/coalesce control and display queues.
- [x] Prioritize TX release and safety commands over media/display traffic.
- [x] Export connection, queue, drop, and high-water metrics.

Initial capacity target to validate:

- One normal operator
- Up to four authenticated remote clients
- One controller/TX owner

Acceptance criteria:

- Excess clients are rejected cleanly.
- Flood tests keep memory, threads, and safety-command latency within defined limits.

Completed in REM-0503. Saturn Go admits at most four authenticated logical
remote clients. A split control/media pair with the same session id consumes
one slot and duplicate lanes are rejected; excess clients receive HTTP 429 with
a retry hint. Saturn Bridge independently caps physical TCI sockets at eight,
preserving the four-client split-transport target. The existing single
operator/controller role remains authoritative.

Inbound commands use fixed safety, control, and microphone queues. State
commands coalesce, microphone retention is limited to eight newest frames, and
TX release bypasses both queues through the safety lane. Per-client outbound
control is capped at 256 messages/256 KiB, display remains depth one, audio
retention remains bounded to 250 ms, and safety traffic is scheduled first.
Authenticated connection metrics are available from `/remote_metrics`; bridge
connection, queue, replacement/drop, latency, and high-water counters are
exported through `/bridge_diag` and the `remote_backpressure` TCI telemetry.
Flood tests cover hard queue/socket ceilings and verify that TX release is
dequeued first in under 10 ms after 10,000 control/media submissions.

## Milestone 6 — Low-burden session improvements

### REM-0601: Preserve the one-year remembered-login behavior

- [A] Do not introduce routine 7–30 day password prompts.
- [ ] Continue invalidating remembered login when the administrator password changes.
- [ ] Clearly identify shared-device risk in the operator documentation.

### REM-0602: Add optional per-device revocation

- [D] Issue separate opaque tokens per remembered device.
- [D] Store token hashes and device metadata, not plaintext tokens.
- [D] Let the operator name, view, and revoke an individual device.
- [D] Keep the allowed absolute lifetime at one year.

This item is deferred until the deployment, restore, state, and job milestones are stable.

## Milestone 7 — Reproducible release input

### REM-0701: Deploy exact commits

- [ ] Resolve a requested branch/tag to a full commit before building.
- [ ] Record the exact commit and source remote in the release manifest.
- [ ] Never activate a different commit from the one tested in staging.
- [ ] Preserve branch-following behavior only as a source-selection convenience; deployment itself uses the resolved immutable commit.

### REM-0702: Defer release signing without blocking remediation

- [D] Signed tags or signed arm64 release bundles.
- [D] SBOM and provenance publication.
- [D] Offline artifact installation.

Exact commit identity, manifest hashes, and target-aware readiness are required now. A formal release-signing system can be added later if Saturn becomes a managed fleet or gains additional firmware ownership.

## FPGA artifact policy boundary

No FPGA release policy will be invented by the application-maintenance team without the firmware maintainer.

Until a firmware owner defines that policy:

- Normal Saturn application deployment must not rebuild or flash the FPGA.
- Existing FPGA images remain unchanged by remediation work.
- The UI may report the detected FPGA version and available artifact hash, but must not claim source reproducibility.
- FPGA flashing remains an explicit, separately confirmed maintenance operation.
- A future firmware owner must decide whether checked-in images or reproducible Vivado builds are authoritative, how artifacts are versioned, and what hardware regression evidence is required.

## Validation matrix

### Required without radio hardware

- [ ] Rust formatting, linting, locked tests, audit, and deny checks
- [ ] TypeScript checking, browser tests, template seam checks, and production bundle build
- [ ] Shell syntax, ShellCheck, cloud-init schema, and provisioning contract tests
- [ ] Native P2 boundary/controller tests and sanitizer jobs
- [ ] Deployment broken-build, wrong-commit, readiness-timeout, and rollback tests
- [ ] Atomic state and restore fault-injection tests
- [ ] Job restart/reconciliation and conflict tests
- [ ] Request-size, output-backpressure, and client-limit tests

### Required on a Saturn appliance before release

- [ ] Verify low-memory build preflight, disk-backed swap activation, and single-job Rust compilation on a 1 GiB appliance.
- [ ] Deploy new release and verify expected commit
- [ ] Automatically roll back a deliberately broken release
- [ ] Reboot with the new release active
- [ ] Verify bridge/P2 startup and RX operation
- [ ] Verify controlled TX, PTT release, and watchdog unkey behavior
- [ ] Interrupt update/restore and confirm deterministic recovery
- [ ] Test low-disk and temporary network-loss behavior
- [ ] Verify Tailscale and LAN access remain operator-friendly
- [ ] Verify LCD operation without requiring a front panel

## Finding disposition

| Review finding | Disposition |
|---|---|
| SAT-001 five-character credential | Accepted product decision; document and preserve optional longer values |
| SAT-002 RF TX default | Accepted product decision; reconcile documentation and show effective state |
| SAT-003 target-blind update check | Remediate in Milestones 1–2 |
| SAT-004 nontransactional restore | Remediate in Milestone 3 |
| SAT-005 process-local jobs/locks | Remediate in Milestone 4 |
| SAT-006 update metadata warning | Remediate through deployment transaction/state store |
| SAT-007 arbitrary health URL | Replace with internal target-aware readiness |
| SAT-008 global body/output growth | Remediate in Milestone 5 |
| SAT-009 job orchestration | Remediate in Milestone 4 |
| SAT-010 direct process exit | Remediate in Milestone 4 |
| SAT-011 non-atomic state writes | Remediate in Milestone 4 |
| SAT-012 remembered-device lifetime | One-year lifetime accepted; per-device revocation deferred |
| SAT-013 TLS pair handling | Planned after core deployment/state work |
| SAT-014 bridge resource growth | Remediate limits in Milestone 5 |
| SAT-015 Protocol 2 IP trust | Document trusted-LAN boundary; no protocol redesign in this plan |
| SAT-016 privileged helper breadth | Reduce clone helpers now; broader typed broker deferred |
| SAT-017 mutable branch provisioning | Deployment resolves and records an exact commit |
| SAT-018 mutable CI dependencies | Planned maintenance hardening, not a release blocker for this branch |
| SAT-019 liveness-only health | Remediate in Milestone 1 |
| SAT-020 ignored startup errors | Include in readiness/fail-fast state work |
| SAT-021 signed/A-B releases | Versioned releases now; signing deferred |
| SAT-022 native hardening | Deferred maintenance improvement |
| SAT-023 HTTP/docs mismatch | Planned documentation/HTTP cleanup |
| SAT-024 host-derived redirects | Planned low-risk cleanup |
| SAT-025 module size/template seams | Refactor only where required by remediation; avoid unrelated rewrites |

## Release gates for this remediation branch

This branch is ready to merge only when:

- [x] Product decisions are reflected consistently in documentation and UI.
- [x] Browser SD-card imaging/cloning is disabled and the manual alternative is documented.
- [ ] Deployment builds and verifies the exact target commit.
- [ ] Failed deployment automatically restores the previous verified release.
- [ ] Restore and critical state writes are transaction-safe.
- [ ] Conflicting maintenance jobs remain locked across Saturn Go restarts.
- [ ] Ordinary API routes no longer inherit the 2 GiB body limit.
- [ ] Non-hardware CI passes.
- [ ] Appliance validation passes on supported Trixie arm64 hardware.
- [ ] The tested commit and validation evidence are recorded below.

## Completion evidence

Update this section as milestones complete.

| Milestone | Commit/PR | Test evidence | Appliance evidence | Status |
|---|---|---|---|---|
| 0 — Contract | Milestone 0 implementation commit | Rust 103/103; template seam; password, provisioning, and imaging-disablement contracts | Hardware-independent | Complete |
| 1 — Health | — | — | — | Not started |
| 2 — Deployment | — | — | — | Not started |
| 3 — Restore | — | — | — | Not started |
| 4 — State/jobs | — | — | — | In progress (REM-0401 through REM-0403 complete) |
| 5 — Limits | REM-0501 through REM-0503 implementation commits | Rust boundary/flood tests; durable broker cap contract | REM-0501 and REM-0502 appliance acceptance complete; REM-0503 pending appliance install | Complete pending REM-0503 appliance acceptance |
| 6 — Sessions | Deferred | — | — | Deferred |
| 7 — Reproducibility | — | — | — | Not started |
