# Changelog

All notable changes to the Saturn Update Manager (Rust) are documented here.

## [Unreleased]
### Added
- Hidden `/p23test` lab and `p23-app-manager.sh` now document and drive the
  converged `p2app` service path directly instead of presenting `P2_app` and
  `P3_app` as separate active deploy targets. Legacy `p3` action arguments are
  still accepted as compatibility aliases.
- G2/app-status wording now distinguishes runtime app identity from deployed
  binary family, which makes the transition state readable when an older `p3`
  deploy family is still running the converged `p2` identity.
- P2/P3 runtime telemetry (`/p23_perf`) now includes a structured `fpga`
  section with live product, firmware name/version, bit-file date code, clock
  state, fallback-image state, and die-temperature data exported directly from
  the running app instead of relying only on startup-banner text.
- FPGA flashing now uses a dedicated root-owned helper (`saturn-flash-fpga.sh`)
  under `/usr/local/lib/saturn-go/scripts`, with the web-facing
  `flash_fpga.sh` reduced to a narrow `sudo -n` wrapper. This fixes the FPGA
  Flash page when the service user does not have general passwordless sudo.
- Saturn Go XDMA tools now include a `Stage Running Kernel` maintenance action
  in `saturngo.html`, backed by a narrow privileged helper that runs
  `saturn-fix-xdma.sh --stage-kernel "$(uname -r)"` without restarting
  `p2app.service`.
- `saturngo.html` now includes a compact XDMA status card that surfaces the
  parsed doctor stage/advisory state without requiring users to read the full
  terminal dump.
- Rust server test coverage for the extracted handler/helper modules (`auth`,
  `middleware`, `pages`, `update`, `util`, `monitor`, `clone`, `image`,
  `repair`), including endpoint-level checks for CSRF enforcement, update
  policy/status handlers, the shared `begin_update_activity` conflict gate, Pi
  image request validation, and repair-pack/verify-system-config response
  shape.
- Graceful shutdown: server now handles SIGINT/SIGTERM via `axum::serve().with_graceful_shutdown()`, allowing in-flight requests to complete before exit.
- Runtime repo-tree discovery/switching API: `GET /list_repo_roots`, `POST /set_repo_root`, with persisted active root (`repo_root.txt`).
- Backup UI controls for selecting and applying the active repo root.
- Rust backend health endpoint: `GET /healthz`.
- CSRF request guard for mutating (`POST`) API routes using `X-Saturn-CSRF`, plus same-host Origin/Referer checks when present.
- Appliance update API: `GET/POST /update_policy`, `POST /update_start`, `GET /update_status`, `POST /update_rollback`.
- Transactional repo update flow with staged git worktree switch and rollback on health-check failure.
- Pre-update snapshot archives with retention policy in `/var/lib/saturn-state/snapshots`.
- Backup UI "Appliance Update" panel for channel policy, start/status, and rollback controls.
- `saturn-go-watchdog.timer` + `saturn-go-watchdog.service` for periodic health checks and self-heal restart.
- Backup / Restore clone UI `Wipe Target` action plus `POST /pi_wipe_target` endpoint for quick pre-clone metadata wipe (best-effort unmount, signature/partition-table cleanup, first/last 16 MiB zeroing).
- Dedicated Saturn Go self-update page (`/saturngo`, `/saturn-go`) with separate repo/ref policy form, live terminal output, and rebuild/redeploy workflow for `saturn-go.service`.
- Saturn Go self-update policy API (`GET/POST /saturngo_policy`) with a separate persisted policy file from the G2 Appliance Update policy.
- Saturn Go deploy status endpoint (`GET /saturngo_deploy_status`) backed by a persisted status JSON file for last-run/deploy visibility.
- New `update-saturn-go.sh` script to update repo/build/redeploy the Rust backend from the web UI via `/run`.
- Hidden P2/P3 App Test Lab page (`/p23test`) for build/deploy/switch testing of `P2_app` and `P3_app` without adding navigation links.
- P2/P3 test-lab status endpoint (`GET /p23_status`) reporting service state, source/deployed binaries, symlink selection, and systemd override state.
- P2/P3 test-lab performance endpoint (`GET /p23_perf`) for lightweight process/network/XDMA/PCIe snapshots used to baseline P3/P2 runtime behavior and investigate lag.
- P2/P3 ADC peak telemetry toggle endpoint (`POST /p23_adc_telemetry`) and hidden `/p23test` panel for enabling/disabling runtime ADC peak export without persistent disk writes.
- New `p23-app-manager.sh` helper script for P2/P3 build/deploy/switch/revert actions via `/run`.
- New `scripts/install-shutdown-waiter-service.sh` migration installer to deploy `saturn-shutdown-waiter.service`, initialize `/etc/default/saturn-shutdown-waiter`, and remove legacy `~/.config/autostart/g2-shutdown.desktop`.
- Hidden P2/P3 Test Lab now includes a `Capture Snapshot` action that packages the current `/p23_perf` payload, derived runtime counters, and baseline sample summary into a copyable/downloadable JSON snapshot for lab review.
- Hidden P2/P3 Test Lab snapshot capture now also carries the effective `p2app.service` runtime-tuning state from `systemctl show -p Environment`, so lab snapshots can record live `SATURN_P3_RT_AUDIO_*` settings alongside host/app metrics.
- G2 Update page now includes a read-only `Show App / Firmware Info` action that prints the active P2/P3 app/version from `/p23_perf` plus the latest `p2app.service` startup banner lines for FPGA firmware, build date, bit-file date code, clocks, and startup die temperature.
- G2 Update now includes `Update Web Manager Too`; when `update-G2.py` detects pulled changes under `update_manager/`, the page can automatically launch `update-saturn-go.sh --skip-git --verbose` as a separate final post-step after the G2 run completes, rebuilding from the already-updated active repo root.
- Dedicated deskHPSDR Update page (`/deskhpsdr`) and `update-deskhpsdr.py` runner for live clone/update/build terminal workflow, including helper-script-driven dependency/build flags and fresh-image clone support.
- Default Custom Scripts startup seeding now also includes `setup-eth-fallback.sh`, exposing the Ethernet APIPA fallback repair helper in the web manager.
- Default Custom Scripts startup seeding now also includes `fix-LED-power-button.sh`, exposing the front-panel LED/power-button repair helper in the web manager.
- Saturn Go page now includes an `XDMA Doctor` action that runs a classified read-only PCIe/XDMA report through the existing privileged helper lane.

### Changed
- `update-G2.py` and `update-saturn-go.sh` now verify that configured policy
  repos/refs are publicly reachable over HTTPS before pulling, use the
  validated policy URL directly instead of rewriting the local git remote, and
  surface a clear "public repo required" error when a private or mistyped
  GitHub repo is saved for unattended/anonymous update flows.
- Appliance Update now follows the same anonymous-safe policy URL behavior:
  the Rust backend probes the configured public repo/ref with `git ls-remote`,
  fetches the selected ref directly from the policy URL into `FETCH_HEAD`, and
  leaves the local checkout's `origin` remote untouched.
- `update.html` and `saturngo.html` now warn that saved GitHub repos must be
  public for unattended or anonymous appliance/self-update runs.
- `g2-version-info.sh` now prefers live `/p23_perf` FPGA/runtime metadata for
  firmware version, product/version, bit-file date code, fallback state, and
  die temperature, using the `p2app.service` startup-banner journal scrape only
  as a fallback when runtime telemetry is unavailable.
- `g2-version-info.sh` now labels the selected deployed slot separately from
  the app's live runtime protocol identity so the current transition state
  (`p3` slot running as `p2`) reads clearly instead of looking contradictory.
- `g2-version-info.sh` now emits an explicit warning when `/p23_perf` fetches
  fail or return no data instead of silently falling back to the non-telemetry
  path.
- The FPGA flash helper now resolves `--latest` with a safe per-file mtime scan
  instead of relying on `ls -t "$FPGA_DIR"/*.bin`.
- `g2-version-info.sh` now reports the deployment slot separately from the
  live app identity, avoids printing two competing `Runtime` blocks, and scopes
  the FPGA startup-banner scrape to the current `p2app.service` instance
  instead of grepping the full unit journal.
- `saturn-xdma-doctor.sh` now emits an explicit advisory when XDMA is working
  only because the module is already loaded but the module is not installed on
  disk for the running kernel, so the doctor output distinguishes "runtime OK"
  from "reboot/recovery safe".
- The privileged XDMA doctor/staging helpers now re-exec through
  `systemd-run --pipe` when launched from `saturn-go.service`, so Saturn Go
  can inspect or stage `/lib/modules/.../xdma.ko` without inheriting the
  service's kernel-module protection sandbox.
- `scripts/fix-xdma.sh` now falls back to `/usr/src/linux-headers-<kernel>`
  and repairs missing `/lib/modules/<kernel>/{build,source}` links before
  failing header detection, which fixes false "headers missing" failures on
  kernels whose header package is already installed.
- `update_manager/rust-server/src/main.rs` is no longer carrying shadow copies
  of the server subsystems. The extracted modules (`state`, `util`, `update`,
  `auth`, `middleware`, `pages`, `monitor`, `clone`, `image`, `repair`) are
  now wired into the live binary, and `main.rs` has been reduced to router and
  remaining shared runtime logic instead of a parallel monolith plus orphaned
  code paths.
- Backup/restore test tooling now carries forward the active clone/image
  request behavior: clone start supports `verify_compare`, stderr
  classification is normalized in the clone job log, and `pi_wipe_target`
  lives beside the rest of the clone-device workflow instead of remaining
  inline in `main.rs`.
- Hidden P2/P3 Test Lab `/p23test` now honors the shared light/dark browser
  theme preference and includes an in-page theme toggle instead of forcing a
  dark-only palette.
- Saturn Go installer/self-update now also deploys the root-owned `saturn-xdma-doctor.sh` helper into `/usr/local/lib/saturn-go/scripts` and extends the narrow sudoers policy so the non-root web service can invoke it via `/opt/saturn-go/scripts/xdma-doctor.sh`.
- Appliance update policy now includes `healthcheck_retries` and
  `healthcheck_initial_delay_secs`, allowing staged repo switches and rollback
  probes to tolerate slower local service startup before declaring health-check
  failure.
- Startup repo-root resolution now canonicalizes both the default root and any
  saved `repo_root.txt` path before Saturn repo validation, closing symlink
  bypasses in the early startup path.
- Browser-managed custom script metadata is now bounded at load and write time:
  script count, flag count, and flag length are capped, and oversized
  `custom_scripts.json` files are rejected before deserialization.
- Full restore tar preflight now parses verbose tar listings so restore can
  validate archive paths and symlink targets, measure uncompressed size, reject
  extreme expansion ratios, and check `/tmp` free space before extraction.
- Update G2 now runs the shutdown waiter installer plus
  `fix-LED-power-button.sh` as part of the maintenance flow.
- `setup-eth-fallback.sh` remains available in Custom Scripts, but no longer
  auto-runs during Update G2.
- Installer now deploys `install-shutdown-waiter-service.sh` and
  `shutdown-waiter.sh` into the runtime script set, installs matching
  root-owned helper copies under `/usr/local/lib/saturn-go/scripts`, and
  writes a narrow sudoers policy so the web UI can execute the three G2 repair
  helpers with `sudo -n`.
- Shutdown waiter install/provision defaults now use `SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT=auto`, so fresh images arm the post-boot power-button watcher unless hardware auto-detect says not to.
- Saturn Go installer/self-update script sync now includes the repo-root `scripts/setup-eth-fallback.sh` helper in the managed `/opt/saturn-go/scripts` set.
- Saturn Go installer/self-update script sync now also includes the repo-root `scripts/fix-LED-power-button.sh` helper in the managed `/opt/saturn-go/scripts` set.
- deskHPSDR update/build flow now applies a Saturn-managed `deskhpsdr` libgpiod v2 compatibility patch before build and probes with `GPIO=ON` instead of forcing `GPIO=OFF`, preserving GPIO support on Trixie during update-manager driven builds.
- P2/P3 app speaker and DUC underrun telemetry now counts underflow episodes on edge transitions instead of incrementing on every FIFO-monitor poll while the same starvation condition is still active. This makes `/p23_perf` cumulative underrun history comparable across runs and prevents one recovery period from inflating the totals.
- P2/P3 app high-priority telemetry now preserves the live ADC peak-hold values when exporting `/dev/shm/saturn_p23_perf_stats.json`, so the lab page ADC gauges match runtime packet content instead of being zeroed before snapshot.
- P2/P3 app Makefiles now emit and include header dependency files, preventing stale object reuse after telemetry enum/header changes; this fixed shifted `/p23_perf` counters such as bogus wideband send errors and corrupted DUC/speaker totals after incremental rebuilds.
- `update-saturn-go.sh` now deploys the same web/script asset set as provisioning: release binary, HTML pages, `config.json`, `themes.json`, and packaged runtime scripts, while preserving browser-managed extra scripts in `/opt/saturn-go/scripts`.
- Hidden P2/P3 Test Lab `/p23test` now uses a workload-first dashboard layout instead of a single dense text block, making host pressure and app behavior easier to read together.
- Hidden P2/P3 Test Lab status and dashboard panes now surface the effective service runtime/audio-RT configuration, and `Copy JSON` falls back to a legacy textarea copy path when the browser Clipboard API is unavailable.
- `/p23_perf` now includes workload tags plus optional app-exported telemetry (`/dev/shm/saturn_p23_perf_stats.json`) so the lab can correlate host metrics with DDC/wideband/mic/DUC/speaker activity.
- ADC peak telemetry documentation now matches runtime behavior: disabling the
  feature removes the enable flag and stops updates, but the last `/dev/shm`
  snapshot is retained until overwritten or removed.
- CSRF middleware now rejects POST requests that are missing both `Origin` and `Referer` headers, closing a bypass when neither header was sent.
- `/get_system_data`: `proc_regex` query parameter now compiled with a 64 KB size limit via `RegexBuilder` to prevent regex-based CPU exhaustion.
- `/exit` endpoint now logs the remote IP at `warn` level before initiating shutdown.
- Pi image download cleanup delay increased from 30 seconds to 10 minutes, preventing file deletion while large downloads are still in progress.
- Completed Pi image and clone job maps are now pruned to a maximum of 20 entries, preventing unbounded memory growth over long uptimes.
- Default custom script constants replaced with `include_str!()` referencing `scripts/cleanup-saturn-logs.sh` and `scripts/cleanup-saturn-backups.sh`, eliminating source duplication.
- Updated `README.md` to document the current Rust/Axum backend, deployment layout, and compatibility naming (`saturn-go` service/binary).
- Documented version panel behavior and the non-interactive privilege model used by `update-G2.py` and `/change_password`.
- Replaced installer implementation with a Rust-only deployment flow (no embedded Go source generation/build).
- Installer now configures NGINX as a path-prefix reverse proxy for `/saturn/*` plus a dedicated SSE route for `/saturn/run`.
- Installer now enforces non-default admin bootstrap credentials (prompt/env/random generation), instead of shipping `admin/admin`.
- Re-aligned uninstall script to remove the exact artifacts created by current install flow (service, NGINX site, SSE map, optional auth/runtime purge).
- Uninstall now defaults to keeping runtime directories/custom state; use `--purge` for full cleanup.
- Installer script sync now preserves browser-managed custom scripts and only updates packaged scripts when source files are newer.
- Request body handling now uses explicit limits (`SATURN_MAX_BODY_BYTES`, `SATURN_RESTORE_MAX_UPLOAD_BYTES`) instead of unlimited bodies.
- Main, backup, and monitor web UIs now attach `X-Saturn-CSRF: 1` to all mutating API calls.
- `run` SSE path now streams with lower latency: line-buffered subprocess invocation (`stdbuf` when available), `\r` + `\n` boundary handling, and no-cache/anti-buffer response headers.
- NGINX `/saturn/run` now disables request buffering and adds explicit no-cache header to reduce end-to-end stream latency.
- Monitor refresh interval reduced from 3s to 1s for more appliance-like real-time visibility.
- Installer now writes update-policy/snapshot/staging env configuration into `saturn-go.service`.
- Installer now uses dedicated writable state path (`/var/lib/saturn-state`) for repo-root and appliance update state files.
- Installer now applies additional systemd hardening (`RestrictSUIDSGID`, `ProtectKernel*`, `ProtectControlGroups`, syscall/address-family restrictions).
- Installer now omits `NoNewPrivileges` in `saturn-go.service` so allowed `sudo -n` maintenance paths remain functional.
- Repo-root switching and restore now require Saturn-style git checkout paths (`.git` + `update_manager`), preventing destructive restore targets.
- Appliance update now prunes older staged worktrees under `/var/lib/saturn-state/repo-staging` to limit disk growth.
- Uninstaller now removes watchdog units and watchdog script to stay aligned with installer artifacts.
- `/run` now blocks Python script execution when the resolved script path is inside the active Saturn repo tree; only installed script copies are allowed.
- Python scripts launched by `/run` now set `PYTHONDONTWRITEBYTECODE=1` and `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`.
- Clone target detection now includes USB-attached block devices that report `removable=0` (common for USB SD card readers), while still excluding internal/virtual devices.
- Saturn Go self-update page now uses explicit run-option checkboxes (`verbose`, `dry-run`, `skip-git`, `skip-build`, `skip-deploy`) and shows a polling "Last Deploy Status" panel.
- `/run` now injects Saturn Go self-update policy env vars and a deploy-status-file path when launching `update-saturn-go.sh`, and treats Saturn Go self-update as a shared update activity (conflict-guarded like G2/appliance update).
- Installer now copies `saturngo.html` and hidden `p23test.html`, writes Saturn Go policy/deploy-status service env vars, and auto-bootstrap builds with a rustup-managed stable toolchain (removing legacy apt `cargo`/`rustc` when present).
- Installer now warns before `apt-get update` when `/tmp` is too full (helps diagnose misleading APT signature-split errors caused by tmpfs exhaustion).
- Hidden P2/P3 Test Lab now supports explicit startup profiles (`panel`, `panel-debug`, `headless`) and front-panel mode overrides (`SATURN_FRONT_PANEL_MODE`) for switch/deploy actions, plus an emergency web revert button.
- `p23_status` now reports parsed override metadata (`panel_mode`, `saturn_meta`) from the generated systemd drop-in for easier troubleshooting.
- `p23_status` now also reports optional ADC peak telemetry state and the latest exported snapshot from `/dev/shm/saturn_p23_adc_peak_telemetry.json`.
- Hidden P2/P3 Test Lab now includes a Phase 1 performance panel with resettable baselines (process CPU/RSS/scheduler wait plus `eth0` throughput and XDMA interrupt-rate/PCIe link telemetry).
- Hidden P2/P3 Test Lab performance panel now highlights threshold alerts for scheduler wait / CPU spikes and XDMA interrupt spikes or drops relative to baseline.
- `P2_app` / `P3_app` ADC peak telemetry export now writes only a single latest-snapshot JSON file in `/dev/shm` while enabled, capped to roughly once per second, instead of continuously writing persistent logs.
- Hidden P2/P3 Test Lab status/performance panes no longer restore stale cached content across reloads, browser polling is serialized, and the performance baseline resets automatically when `p2app.service` restarts into a new P2/P3 process identity.
- `/p23_perf` CPU total-tick parsing now excludes Linux guest accounting fields so process CPU percentages are not slightly under-reported when guest time is present.
- `/p23_perf` now includes process `rchar`/`wchar` counters, and the hidden P2/P3 Test Lab derives an XDMA efficiency proxy (`IRQ/MiB` of process char I/O) so high raw XDMA interrupt rates can be compared against actual app traffic.
- `update-G2.sh` and `update_manager/scripts/update-G2.py` now run the shutdown-waiter migration installer as part of update flow.
- Provisioning (`provision/cloud-init/provision-saturn.sh`) now installs and runs the shutdown-waiter installer by default (`SATURN_INSTALL_SHUTDOWN_WAITER=1`) and adds `gpiod` to required packages.
- `scripts/copy-autostart.sh` no longer installs `g2-shutdown.desktop`; shutdown waiting is now service-managed.
- `autostart-files/g2-shutdown.desktop` is now marked deprecated/hidden to prevent new desktop-session activation.
- FPGA flash page (`templates/fpga.html`) now uses `/run_log` as the primary source of truth, labels write/verify phases separately, keeps polling until buffered output is fully drained, removes stale session-state replay, and limits rendered output to a smaller visible tail to reduce browser lockups during long flash/verify runs.
- FPGA flash live log polling now serializes in the browser and batches DOM updates to avoid overlapping fetch/render cycles during high-volume `load-FPGA` output.

### Fixed
- Appliance update and rollback health checks no longer fail on the first
  transient timeout by default; the local health probe can now wait and retry
  according to normalized policy values.
- Full restore now rejects unsafe symlink targets and archives that would
  over-expand or exceed current `/tmp` free space before extraction begins.
- `update-G2.py`: verbose mode now preserves captured command output used by status sections (fixes `Size: ?` and `Commit: ?` cases).
- `update-G2.py`: library install now checks which APT packages are actually missing before attempting privileged installs.
- `update-G2.py`: privileged steps now use adaptive sudo behavior (`sudo` with TTY, `sudo -n` without TTY) with clearer failure messages.
- `/change_password`: backend now retries with `sudo -n htpasswd` and returns actionable permission errors when sudo rules are missing.
- `/get_versions`: missing script `version` metadata now returns `unknown` instead of a misleading hard-coded version.
- Added explicit `version` metadata for `update-G2.py` (`2.14`) and `update-pihpsdr.py` (`1.10`) in `scripts/config.json`.
- `/change_password`: switched to `htpasswd -i` with stdin input so passwords are not exposed in process arguments.
- `restore-backup.sh`: fixed `--list --json` runtime error and malformed JSON output.
- `restore-backup.sh`: added `--backup-name` support used by web UI restore flow.
- `/run_log` now tracks absolute line offsets for rolling script logs so long FPGA flash runs continue streaming correctly after the in-memory buffer begins truncating older lines.
- `monitor.html`: replaced process-row `innerHTML` injection with DOM `textContent` rendering for safer output handling.
- `scripts/config.json`: corrected restore script `directory` path metadata.
- Installer health check now fails loudly (with `systemctl`/`journalctl` diagnostics) if backend `/healthz` does not come up.
- Installer now always restarts `saturn-go.service` after writing the unit, preventing stale in-memory env (e.g., old bind port) on reinstall.
- Password minimum reduced to 5 characters across installer prompt, backend `/change_password`, and web UI validation.
- Light-theme terminal output now keeps compile/build lines readable (ANSI white mapped for light backgrounds), including the dedicated `pihpsdr.html` terminal view.
- `update-G2.py`: added an automatic CAT signature compatibility patch for `sw_projects/P2_app/g2panel_libgpiodv2.c` after pulls, fixing recent `MakeProductVersionCAT` build breaks.
- `update-G2.py`: `install_udev_rules` now skips with warning (instead of hard-failing the full update) in non-interactive mode when passwordless sudo is unavailable.
- G2 terminal runner now enforces a configured Appliance Update repo URL; if not configured, `/run` returns a clear error instead of silently using defaults.
- G2 terminal runner now passes Appliance Update policy repo/remote/ref into `update-G2.py`, and `update-G2.py` now applies that policy by setting the git remote URL before pulling.
- `update-G2.py` and `update-pihpsdr.py` now refuse execution when run from inside the Saturn repo tree, preventing accidental repo-local Python runs.
- `update-pihpsdr.py`: prevented startup crash on non-UTF-8 (`latin-1`) stdout/stderr/log streams by adding per-stream Unicode fallback output and explicit UTF-8 log file writes.
- `update-saturn-go.sh`: fixed `--dry-run` staging-helper generation error when the staged directory is intentionally not created.
- `update-saturn-go.sh`: fixed detached root-helper status-file error handling/JSON quoting so `/saturngo_deploy_status` always returns valid JSON after deploy completion.
- `p23-app-manager.sh`: dry-run deploy/switch no longer writes a temp override file (avoids failing when `/tmp` is full).
- `index.html`: custom scripts/version/flags/password UI now reports clearer errors when an API returns HTML (login page / backend error page) instead of JSON.
- `scripts/shutdown-waiter.sh`: added config-gated modes (`auto|true|false`), pull-up pin reads, high-before-arm guard, and consecutive-low confirmation to reduce false shutdown triggers on mixed hardware variants.
- `/p23_perf` now falls back to `eth0` byte counters as a documented char-I/O proxy source (`process.io.source = "eth0_netdev_proxy"`) when `/proc/<pid>/io` is unreadable for the running `p2app.service` process, keeping the hidden P23 `IRQ/MiB` metric usable under service-user permission constraints.
- `scripts/shutdown-waiter.sh` now tolerates both libgpiod CLI styles when reading the shutdown GPIO, trying the newer `gpioget -c/--chip ... --numeric` form first and falling back to the older positional-chip syntax so Bookworm/Trixie and older images both work.

## [2026-02-13]
### Added
- Full repo backup download (`/backup_full`) and restore (`/restore_full`) with validation and RESTORE confirmation.
- Dedicated **Backup / Restore** page (`backup.html`) linked from the main UI.
- Pi image creation workflow with progress, validation (size + SHA256), and download cleanup.
- Output directory selection for Pi image creation (default `/mnt/usb`).
- Pi image cancel support and live log panel.
- Clone SD card to removable device workflow with auto-detected targets, progress, and cancel.
- Repair Pack download and system config verification tools.
- New script `clone_pi_to_device.sh`.

### Changed
- Disabled default request body limits to support large uploads.
- Removed **Create Pi Image** from the main script list (moved to backup page).
- Added `SATURN_REPO_ROOT` env var for configurable repo root.

### Fixed
- Restores now accept `dry_run=1` and similar boolean query values.
