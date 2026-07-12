# Saturn Update Manager (saturn-go)

Saturn Update Manager provides a web UI for Saturn maintenance tasks.
The current backend is implemented in Rust (Axum), while deployment paths and
service names still use `saturn-go` for compatibility with existing installs.

## Documentation Map

- Architecture and internal flow: `docs/ARCHITECTURE.md`
- Full feature inventory (what was added and where): `docs/FEATURE_MATRIX.md`
- Backend endpoint reference: `docs/API_REFERENCE.md`
- Script inventory and usage map: `docs/SCRIPT_CATALOG.md`
- Build/install/operate/troubleshoot runbook: `docs/OPERATIONS_RUNBOOK.md`
- Rust supply-chain policy: `docs/RUST_SUPPLY_CHAIN.md`
- XDMA DKMS release policy: `docs/XDMA_RELEASE_POLICY.md`
- Docs index: `docs/README.md`

## Features

- Web UI for script execution, update coordination, monitoring, and backup/restore workflows
- Server-Sent Events (SSE) script output streaming
- Backend run-log buffering endpoint (`/run_log`) so terminal output can resume after page switches
- Full repository backup download and restore with archive validation
- Full restore preflight now validates tar paths and symlink targets, checks
  uncompressed expansion ratio, and verifies `/tmp` free space before
  extraction
- Script backup integration: list and restore `saturn-backup-*` and `pihpsdr-backup-*` directories from Backup / Restore page
- Runtime repo-root switching via API/UI (`/list_repo_roots`, `/set_repo_root`)
- Startup repo-root selection canonicalizes configured and saved paths before
  Saturn repo validation
- Overview is the default landing page (`/`, `overview.html`) with live system,
  radio, network, deployment, p2app, bridge, and Tailscale status plus quick
  links to common workflows
- Appliance Update policy panel (right side on desktop, below G2 terminal on narrow screens) stores GitHub repo URL + branch/ref + health-check values used by both Appliance Update and Run Update G2
- Appliance Update health policy also supports retry count and initial startup
  delay so a good staged switch is less likely to roll back on a slow first
  probe
- G2 Update page includes a read-only `Show App / Firmware Info` action that reports the active runtime app identity, current binary family, app version, and any current or retained `p2app.service` startup banner lines with FPGA firmware/build/date-code details when they were captured
- G2 Update page also includes `Update Web Manager Too`; when enabled, `Run Update G2` watches for repo changes under `update_manager/` and automatically launches `update-saturn-go.sh --skip-git` as a detached final step after the G2 run completes, reusing the just-updated active repo root instead of requiring a separate Saturn Go repo pull
- Dedicated piHPSDR Update page (`pihpsdr.html`) for `update-pihpsdr.py` terminal execution
- Dedicated deskHPSDR Update page (`deskhpsdr.html`) for `update-deskhpsdr.py` terminal execution using the active Saturn repo-root helper scripts, including the local `deskHPSDR` libgpiod v2 compatibility patch flow
- Dedicated FPGA Flash page (`fpga.html`) for `flash_fpga.sh` using `load-FPGA` (`-b`, `-v`, `-f`) with explicit confirmation guard
- Dedicated Custom Scripts page (`custom.html` / `index.html`) to add/update/delete runnable scripts from browser with file upload + flag metadata
- Browser-managed custom script metadata is bounded by server-side limits
  (script count, flag count, flag length), and oversized `custom_scripts.json`
  files are rejected before load
- Default browser-managed custom scripts are auto-seeded on startup:
  - `cleanup-saturn-logs.sh`
  - `cleanup-saturn-backups.sh`
  - `fix-LED-power-button.sh`
  - `setup-eth-fallback.sh`
- Dedicated Backup / Restore page (`backup.html`) for repo-root management, backup/restore, Pi imaging, clone, and repair tools
- The shared appliance shell provides grouped desktop sidebar navigation, a
  mobile drawer, common dark/light theming, consistent controls/status tokens,
  compact Saturn G2 page-title branding, and local browser assets. Navigation
  groups Overview, Monitor, Radio Telemetry, Saturn Remote, maintenance,
  applications, and system tools.
- Tailwind compatibility runtime, Chart.js, ansi_up, and Inter are vendored
  under `templates/assets/`; maintenance and Remote pages do not require public
  CDNs to render or operate.
- Pi image creation workflow with progress, validation, cancel, and download
- SD-to-removable/USB-device cloning workflow with auto-detected targets, optional quick target wipe, progress, and cancel
- Repair Pack download and system config verification tools
- Built-in monitor for CPU, memory, disk, network, and process data
- Basic auth via NGINX
- CSRF protection for mutating API calls (`X-Saturn-CSRF` + same-host Origin/Referer validation when present)
- Low-latency script streaming: line-buffered subprocess launch (`stdbuf` when available), `\r`/`\n` output boundary handling, anti-buffer SSE headers, plus in-memory run-log buffering for reconnect
- Appliance update workflow: transactional repo staging, pre-update snapshot, policy-driven channels (`stable`/`beta`/`custom`), and rollback endpoint
- Shared update-activity lock prevents overlapping `update-G2` runs with appliance update/rollback operations
- Script runner injects active repo-root environment (`SATURN_REPO_ROOT`, `SATURN_DIR`, `SATURN_ACTIVE_REPO_ROOT`) so update scripts target the same checkout as backend state
- Shutdown waiter migration tooling: updater/provisioning now install `saturn-shutdown-waiter.service` with `/etc/default/saturn-shutdown-waiter` defaults and retire legacy desktop autostart (`g2-shutdown.desktop`)
- `scripts/shutdown-waiter.sh` is now compatible with both older and newer libgpiod `gpioget` CLI variants, including Trixie/libgpiod v2 syntax (`-c/--chip`, `--numeric`).
- Health watchdog timer for self-heal restart if `/healthz` fails
- Repo-root safety checks for manual root switching and restore operations

## Runtime Layout

Typical deployed paths:

```text
/opt/saturn-go/
  bin/saturn-go                 # Rust server binary (legacy name retained)
  scripts/                      # runnable shell/python scripts

/var/lib/saturn-web/
  index.html
  update.html
  saturngo.html
  pihpsdr.html
  deskhpsdr.html
  fpga.html
  monitor.html
  backup.html
  config.json
  themes.json

/var/lib/saturn-state/
  repo_root.txt
  custom_scripts.json
  update_policy.json
  update_state.json
  snapshots/
  repo-staging/
```

Repository source paths:

```text
update_manager/rust-server/     # Rust API server source
update_manager/templates/        # HTML templates copied to web root
update_manager/scripts/          # script and UI config assets
```

Shared UI assets are stored under `update_manager/templates/assets/` and
deployed to `/var/lib/saturn-web/assets/`. Maintenance pages use prefix-safe
relative URLs so both direct backend routes and the production `/saturn/`
Nginx prefix work offline. See `templates/assets/README.md` for pinned versions
and third-party licenses.

## Script Metadata and Versions

Script definitions come from `config.json` plus browser-managed custom entries in `custom_scripts.json`.

- UI script list: `/get_scripts`
- Flag list: `/get_flags`
- Version list ("Show versions above"): `/get_versions`
- Overview page: `/` (also `/overview`, `/overview.html`)
- G2 Update page: `/update` (also `/update.html`)
- Saturn Go page: `/saturngo` (also `/saturngo.html`, `/saturn-go`, `/saturn-go.html`)
- Radio Telemetry & Diagnostics page: `/telemetry` (legacy `/p23test` aliases remain available)
- piHPSDR update page: `/pihpsdr` (also `/pihpsdr.html`)
- deskHPSDR update page: `/deskhpsdr` (also `/deskhpsdr.html`)
- FPGA flash page: `/fpga` (also `/fpga.html`)
- Custom scripts page: `/custom` (also `/custom.html`, `/index.html`)
- Discover FPGA image candidates: `GET /get_fpga_images`
- Active repo root: `/get_repo_root`
- Discover repo roots: `/list_repo_roots`
- Switch active repo root: `POST /set_repo_root` with JSON `{ "repo_root": "/path/to/tree" }`
- Get appliance update policy: `GET /update_policy`
- Set appliance update policy: `POST /update_policy`
- Get Saturn Go self-update policy: `GET /saturngo_policy`
- Set Saturn Go self-update policy: `POST /saturngo_policy`
- Get Saturn Go deploy status: `GET /saturngo_deploy_status`
- Get Radio Telemetry service-lab status (service/source/deploy/symlink/override): `GET /p23_status`
- Get Radio Telemetry performance snapshot (host metrics + workload tags + optional app telemetry): `GET /p23_perf`
- Start transactional update: `POST /update_start` with JSON `{ "channel":"stable|beta|custom", "custom_ref":"..." }`
- Get update status + last state: `GET /update_status`
- Roll back to previous repo root: `POST /update_rollback`
- List Update G2 backups: `GET /g2_backups`
- Validate/restore Update G2 backup directory: `POST /g2_restore` with JSON `{ "backup_name":"saturn-backup-...", "dry_run":true|false, "confirm":"RESTORE" }`
- List piHPSDR backups: `GET /pihpsdr_backups`
- Validate/restore piHPSDR backup directory: `POST /pihpsdr_restore` with JSON `{ "backup_name":"pihpsdr-backup-...", "dry_run":true|false, "confirm":"RESTORE" }`
- List clone target devices for SD-to-device copy: `GET /pi_devices`
- Quick-wipe clone target metadata (signatures/partition tables): `POST /pi_wipe_target` with JSON `{ "target": "/dev/sdX" }`
- Start/cancel clone job and poll status: `POST /pi_clone_start`, `POST /pi_clone_cancel`, `GET /pi_clone_status`
- List browser-managed custom scripts: `GET /custom_scripts`
- Add/update browser-managed custom script entry: `POST /custom_scripts`
- Delete browser-managed custom script entry: `POST /custom_scripts_delete`
- Fetch buffered run output for a script: `GET /run_log?script=<filename>&from=<offset>&limit=<n>`

For mutating API requests (`POST` routes), include header:

- `X-Saturn-CSRF: 1`

The backend also validates `Origin`/`Referer` host against request `Host` when those headers are present.

If a script entry does not define `version`, `/get_versions` now returns
`unknown` instead of a hard-coded default.

## Privilege Behavior

### Script execution (`update-G2.py`)

`update-G2.py` is designed to run from both terminal and web service contexts:

- In verbose mode, commands that require captured output still return output
  (fixes `Size: ?` and `Commit: ?` in status sections).
- APT packages are checked first; installs are only attempted for missing
  packages.
- Privileged steps use:
  - direct execution when already root
  - `sudo` when interactive TTY is available
  - `sudo -n` for non-interactive service execution
- Required privileged steps exit with a clear actionable message when
  elevation is unavailable.
- Optional privileged steps (currently udev rules install in web/non-TTY mode)
  are skipped with a warning instead of failing the whole run.
- `update-G2.py` now invokes `scripts/install-shutdown-waiter-service.sh` to
  install/migrate `saturn-shutdown-waiter.service`, remove legacy
  `~/.config/autostart/g2-shutdown.desktop`, and initialize
  `/etc/default/saturn-shutdown-waiter` when missing.
- `update-G2.py` now also runs `fix-LED-power-button.sh` from the installed
  runtime script set. That helper script self-handoffs to a trusted root-owned
  copy under `/usr/local/lib/saturn-go/scripts`, so Update G2 refreshes the
  front-panel LED/power-button helper in the same maintenance flow.
- `setup-eth-fallback.sh` remains packaged and available from Custom Scripts,
  but it no longer auto-runs during Update G2.
- Shutdown waiter installer default mode is controlled by
  `SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT` (default `auto`), so image builds
  can opt in to `auto`/`true` per hardware profile.
- Installer now writes a narrow sudoers policy for the service user so the web
  UI can execute the root-owned `install-shutdown-waiter-service.sh`,
  `setup-eth-fallback.sh`, `fix-LED-power-button.sh`, and
  `saturn-xdma-doctor.sh` helpers via `sudo -n`.
- When launched by `/run`, the backend sets `SATURN_REPO_ROOT`, `SATURN_DIR`,
  and `SATURN_ACTIVE_REPO_ROOT` to the current active repo root before spawning
  the script.
- When a policy repo URL is provided, `update-G2.py` verifies that the repo is
  publicly reachable over HTTPS and then pulls directly from that policy URL
  instead of depending on the local repo's fetch remote. This keeps anonymous
  G2 update flows from failing with ambiguous credential prompts when a private
  or mistyped GitHub repo is configured.
- Appliance Update uses the same public-repo policy: it probes the configured
  repo/ref with `git ls-remote`, fetches the selected ref directly from the
  policy URL into `FETCH_HEAD`, and does not rewrite the local checkout's
  `origin` remote. Anonymous update users therefore get an explicit "public
  repo required" error for private or misspelled GitHub repos.
- `/run` refuses Python script execution when the resolved script path is inside
  the active repo tree; use installed scripts under `/opt/saturn-go/scripts`.
- Python script runs from `/run` set `PYTHONDONTWRITEBYTECODE=1` and
  `PYTHONPYCACHEPREFIX=/var/cache/saturn-python` to keep source trees clean.

### Change Password

`/change_password` calls the privileged `saturn-admin-password.sh set` helper
via `sudo -n` (password piped over stdin).

- The helper updates `/etc/nginx/.htpasswd` **and** the Saturn Remote TLS
  auth drop-in (`10-remote-auth.conf`) together, all-or-nothing, so LAN and
  remote passwords cannot drift
- A deferred `saturn-go` restart (~2s) applies the TLS-side change
- Returns explicit guidance when the helper or its sudoers entry is missing
- Console recovery: `sudo saturn-admin-password.sh reset` prints a fresh
  generated passphrase (see `docs/ADMIN_AUTH_SIMPLIFICATION.md`)

### Saturn Go Self-Update (`update-saturn-go.sh`)

- Dedicated page `/saturngo` runs the Saturn Go self-update workflow with:
  - live terminal output (`/run`)
  - separate Saturn Go repo/ref policy (`/saturngo_policy`)
  - last deploy status panel (`/saturngo_deploy_status`)
  - an `XDMA Doctor` action for privileged read-only PCIe/XDMA diagnostics
  - a `Stage Running Kernel` action for safe XDMA pre-staging on the currently
    running kernel without restarting `p2app.service`
- The page runs `/opt/saturn-go/scripts/update-saturn-go.sh` to:
  - update the repo (optional)
  - verify/activate the 2 GiB Saturn Go build swapfile
  - rebuild the Rust backend with guarded Pi defaults
    (`CARGO_BUILD_JOBS=1`, `cargo build --release -j1`, `nice -n 15`,
    `ionice -c3`)
  - sync deployed web assets (`*.html`, `config.json`, `themes.json`)
  - sync packaged scripts into `/opt/saturn-go/scripts` without removing browser-managed extras
  - refresh trusted helper copies in `/usr/local/lib/saturn-go/scripts` and rewrite the narrow sudoers policy
  - install `/usr/local/bin/saturn-fix-xdma.sh`,
    `/usr/local/bin/saturn-xdma-kernel-postinst.sh`, and
    `/etc/kernel/postinst.d/saturn-xdma` for future kernel package installs
  - dispatch a detached root helper to stop/copy/start `saturn-go.service`
- When a Saturn Go policy repo URL is provided, `update-saturn-go.sh` now
  verifies that the repo/ref is publicly reachable over HTTPS and fetches
  directly from that policy URL instead of rewriting the local git remote. This
  keeps anonymous/self-update flows from failing with ambiguous GitHub
  credential prompts when the configured repo is private or mistyped.
- The page also runs `/opt/saturn-go/scripts/xdma-doctor.sh` for a classified
  read-only XDMA/PCIe report. That wrapper escalates to the root-owned helper
  copy at `/usr/local/lib/saturn-go/scripts/saturn-xdma-doctor.sh`. When the
  helper is launched from `saturn-go.service`, it re-execs through a transient
  `systemd-run --pipe` unit so the check runs outside the service's
  `ProtectKernelModules=true` sandbox.
- The page also runs `/opt/saturn-go/scripts/xdma-stage-current.sh` when XDMA
  is working but not installed on disk for the running kernel. That wrapper
  escalates to `/usr/local/lib/saturn-go/scripts/saturn-xdma-stage-current.sh`,
  which calls `saturn-fix-xdma.sh --stage-kernel "$(uname -r)"` without
  touching the live module/service state. The privileged helper uses the same
  transient-unit escape so Saturn Go can stage the module on disk without
  inheriting the web service's mount protections.
- UI run options map to script flags:
  - `--verbose`, `--dry-run`, `--skip-git`, `--skip-build`, `--skip-deploy`
- Deploy status is written to:
  - `/var/lib/saturn-state/saturngo_deploy_status.json` (default)
- The web terminal may disconnect near the end when `saturn-go.service`
  restarts; reload the page after ~10-20 seconds.

### deskHPSDR Update (`update-deskhpsdr.py`)

- Dedicated page `/deskhpsdr` runs the deskHPSDR update/build workflow with live terminal output and buffered resume via `/run` + `/run_log`.
- If `~/github/deskhpsdr` does not exist and `--skip-git` is not selected, the updater clones `https://github.com/dl1bz/deskhpsdr.git` before building.
- If the checkout already exists and `--skip-git` is not selected, the updater pulls `origin/<current-branch>` using `--ff-only` and auto-stashes local changes first when needed.
- The build step delegates to the active Saturn repo-root helper script:
  - `scripts/deskhpsdr-test-build-on-current-image.sh --repo ~/github/deskhpsdr`
- When dependency installation is enabled, the build helper installs Debian
  prerequisites through the provisioned privileged helper
  `/usr/local/lib/saturn-go/scripts/deskhpsdr-install-deps-on-current-image.sh`
  using passwordless `sudo -n`; the updater checks this helper before building.
- The helper script now applies `scripts/patches/deskhpsdr-libgpiod-v2.patch` with `git apply` when needed and treats an already-applied patch as success.
- The helper build probe now forces `GPIO=ON` and `SATURN=ON`, which keeps the Trixie/libgpiod v2 compatibility fix active in web-driven updates.
- UI run options map to script flags:
  - `--skip-git`, `-y`, `-n`, `--no-install-deps`, `--no-clean`, `--no-desktop-shortcut`, `--dry-run`, `--verbose`
- The updater resolves helper scripts from the active backend repo root (`SATURN_REPO_ROOT` / `SATURN_ACTIVE_REPO_ROOT`), so it stays aligned with the currently selected Saturn checkout.
- On a fresh image, do not select `--skip-git`; otherwise the updater will fail because there is no local `~/github/deskhpsdr` checkout to build.

### Radio Telemetry & Diagnostics

- The primary-navigation page `/telemetry` provides live radio/performance
  telemetry and an advanced terminal workflow for:
  - building the converged `P2_app`
  - deploying the managed `p2app` binary under `/opt/saturn-go/p23-apps`
  - refreshing `p2app.service` override metadata, current symlink, and restart behavior
  - reverting to the original unit `ExecStart`
- Uses `/opt/saturn-go/scripts/p23-app-manager.sh` via `/run`.
- Status panel polls `GET /p23_status`.
- Performance panel polls `GET /p23_perf` and combines host metrics with workload tags and app-emitted counters for:
  - `p2app.service` main process CPU/RSS/context switches/page faults
  - `eth0` throughput/packet rate/errors+drops
  - XDMA interrupt rate (`/proc/interrupts`) and PCIe link speed/width (`/sys/class/xdma/...`)
  - current workload shape (runtime app identity, binary family, startup mode, panel mode, DDC enable/interleave state, wideband mode)
  - app packet/DMA/error counters for high-priority, DDC, wideband, mic, DUC, and speaker paths
  - DDC/DUC/speaker DMA operation rates and average transfer size
  - DUC/speaker queue depth, FIFO depth, queue age, write/underrun mode, and active-gap state
  - FPGA firmware, clock/fallback state, FPGA die temperature, Pi SoC temperature, and CPU frequency
- Performance panel uses an appliance-style layout with primary CPU/DDC/DUC/XDMA KPIs, compact Radio Data Path, System Health, and Reliability sections, plus explicit active, idle, degraded, and fault states. Idle radios display as healthy/waiting and are excluded from XDMA-drop comparisons.
- Per-task scheduler runtime is retained in raw diagnostics but omitted from the primary UI when it is not reasonably comparable to `/proc/<pid>/stat` CPU time on the running kernel.
- Performance panel includes a `Capture Snapshot` action that packages:
  - the current `/p23_perf` payload
  - the page's derived runtime counters
  - the current baseline summary/sample count
  - the current status/workload summaries
  - the effective `p2app.service` runtime environment subset used by the lab (`SATURN_FRONT_PANEL_MODE`, `SATURN_P3_RT_AUDIO_*`) when available
- Captured snapshots can be copied as JSON or downloaded from the page for later review/comparison; browsers without `navigator.clipboard` automatically fall back to a legacy in-page copy method.
- `fifo_duc_under_events` and `fifo_speaker_under_events` now represent underrun episodes rather than repeated observations of the same active underflow bit during polling recovery, so cumulative totals are more meaningful across long runs.
- When `/proc/<pid>/io` is unreadable for the active `p2app.service` process,
  `/p23_perf` now reports `process.io.source = "eth0_netdev_proxy"` and uses
  `eth0` RX/TX byte counters as the char-I/O proxy source so the Radio Telemetry
  `IRQ/MiB` metric remains available.
- Performance baseline resets automatically when the running `p2app.service` workload identity or active/idle radio state changes, so unrelated samples are not mixed together.
- Status/performance panes are always reloaded fresh and are no longer restored from browser session storage.
- Restart/deploy actions support startup profiles (`panel`, `panel-debug`, `headless`) and front-panel mode overrides (`auto`, `g2`, `g2v2`, `prefer-g2`, `prefer-g2v2`, `off`), written as `SATURN_FRONT_PANEL_MODE` in the systemd drop-in.
- Status/dashboard views also report the effective service runtime environment seen by `p2app.service`, including optional `SATURN_P3_RT_AUDIO_ENABLE`, `SATURN_P3_RT_AUDIO_POLICY`, `SATURN_P3_RT_AUDIO_PRIORITY`, and `SATURN_P3_RT_AUDIO_CPUS`.
- `/telemetry` includes a `Restore Unit Default` action for clearing the Saturn override and returning `p2app.service` to the base unit launch path.
- `/telemetry` includes an optional ADC peak telemetry panel:
  - toggle via `POST /p23_adc_telemetry`
  - explicit `disabled`, `waiting_for_radio`, `live`, `stale`, `stale_process`, `unreadable`, and `invalid` state reported by `GET /p23_status`
  - latest snapshot stored in `/dev/shm/saturn_p23_adc_peak_telemetry.json`
  - publisher runs only while the Protocol 2 outgoing high-priority path is active and atomically publishes mode `0644` for cross-service reads
  - UI reports raw peaks, calculated dBFS, overflow bits, age, and service-PID match
  - uses `/dev/shm` and overwrites a single file, so it does not create persistent disk churn
  - disabling telemetry stops updates and requests removal of the latest snapshot
- Recommended snapshot timing when comparing `p2app` builds or driver changes:
  - `2 minutes` for a quick smoke test after a build/deploy change
  - `10-15 minutes` for a normal baseline capture under a steady workload
  - `30-60 minutes` for longer stability/jitter investigations
  - `5 minutes` each for mode transitions such as idle RX, active RX, TX, and reconnect recovery
- Advanced build/deploy controls are intended for local service testing; the
  page itself is linked from the main navigation for telemetry and diagnostics.

## Build and Deploy (Rust Server)

Build from the repository:

```bash
cd /home/pi/github/Saturn/update_manager/rust-server
cargo check
SATURN_SATURNGO_BUILD_SWAP_FILE=/home/pi/saturn-build.swap \
  /home/pi/github/Saturn/update_manager/scripts/saturn-go-build-preflight.sh ensure-swap
CARGO_BUILD_JOBS=1 \
TMPDIR=/home/pi/github/Saturn/update_manager/rust-server/.tmp \
CARGO_TARGET_DIR=/home/pi/github/Saturn/update_manager/rust-server/target-local \
  nice -n 15 ionice -c3 cargo build --release -j1
```

Quick manual redeploy:

```bash
sudo cp target-local/release/saturn-go /opt/saturn-go/bin/saturn-go
sudo cp ../templates/*.html /var/lib/saturn-web/
sudo cp ../scripts/config.json ../scripts/themes.json /var/lib/saturn-web/
sudo cp ../scripts/* /opt/saturn-go/scripts/
sudo systemctl restart saturn-go.service
```

## Commit and Push to GitHub

Basic local commit flow from the Saturn repo root:

```bash
cd /home/pi/github/Saturn
git status --short
git add <paths>
git commit -m "Describe the change"
git push origin HEAD
```

If you want a branch on GitHub first:

```bash
cd /home/pi/github/Saturn
git checkout -b <branch-name>
git add <paths>
git commit -m "Describe the change"
git push -u origin <branch-name>
```

Notes:

- Use `git status --short` before commit so you can see unrelated local changes.
- Prefer `git push origin HEAD` when working on a branch; it pushes the current branch without retyping the branch name.
- If you amend or rebase commits that were already pushed, update GitHub with `git push --force-with-lease`.
- If a commit message picks up an unwanted trailer such as `Co-Authored-By`, remove it with `git commit --amend` before pushing.

## Installation

Installer (deploy paths, service, web assets, scripts):

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/install_saturn_go_nginx.sh
```

Auth bootstrap:

- If `SATURN_ADMIN_PASSWORD` is set, installer uses it for `admin`
- Otherwise installer prompts for a password when run interactively
- In non-interactive mode, installer generates a random password and prints it once

Installer behavior (current):

- Deploys Rust backend only (no legacy Go source generation)
- Removes legacy distro `cargo`/`rustc` packages (if installed) and bootstraps a current Rust toolchain via `rustup` for the build user before compiling (fixes old-Cargo `Cargo.lock` v4 parse errors on Bookworm)
- Installs `nodejs` and `npm` and runs lockfile-only `npm ci && npm run build` in `update_manager/remote-web` to produce the `saturn-remote-next.js` Vite bundle before staging web assets
- Uses `update_manager/scripts/saturn-go-web-assets.sh` as the shared web asset manifest for install and self-update deploys (fails fast if any required asset, including `saturn-remote-next.html`/`saturn-remote-next.js`, is missing)
- Builds and installs `saturn-bridge.service` by default. The bridge installer
  provisions pinned sparse WDSP 2.00 and piHPSDR Linux-port sources, builds the
  native archive, verifies WFM/phase-rotator/PureSignal symbols, and links the
  bridge without a prebuilt piHPSDR tree or cloud-init provisioning.
- Copies `saturn-remote-next.js` only from the Vite `dist/` output and verifies the deployed bundle with `saturn-remote-next.js.sha256`, preventing stale template/root files from shadowing a fresh build
- Proxies all `/saturn/*` routes through NGINX to the Rust backend
- Redirects plain HTTP remote entry points such as `/remote` and `/saturn/remote` to `https://<host>:8443/remote`
- Creates/updates `saturn-go.service` using a non-root service user
- Enables `saturn-go-watchdog.timer` to auto-restart service when health check fails
- Applies systemd hardening defaults (restricted kernel/control-group access, syscall architecture/address-family restrictions)
- Leaves `NoNewPrivileges` disabled so controlled `sudo -n` paths (for example password update) can work when sudoers permits them
- Sets `/opt/saturn-go/scripts` ownership to the service user/group so browser-managed custom script content can be saved
- Syncs packaged scripts from `update_manager/scripts` plus selected repo-root helper scripts without deleting browser-managed custom scripts; packaged copies update only when source files are newer
- Writes `/etc/sudoers.d/saturn-go-maintenance` so the service user can run the
  narrow set of privileged maintenance helpers non-interactively
- Installs matching root-owned privileged helper copies under `/usr/local/lib/saturn-go/scripts`
- Installs root-owned watchdog script at `/usr/local/lib/saturn-go/saturn-health-watchdog.sh` (outside writable custom script path)
- Saturn Go self-update builds and stages a matching WDSP 2.00 bridge binary,
  service unit, Remote UI HTML, bundle, and checksum before dispatching the
  detached root deployment helper.
- Run `update_manager/scripts/update-saturn-go.sh --skip-git --stage-only` to
  build and validate that complete payload without changing running services.
  A real deployment keeps the previous Saturn Go and bridge binaries and
  restores them if service startup or the bridge TCI health check fails.

Remote state notes:

- The stable remote UI is served from `https://<host>:8443/remote` (rendered from `saturn-remote.html`).
- The next-generation remote UI is served from `https://<host>:8443/remote-next` (rendered from `saturn-remote-next.html` plus the `saturn-remote-next.js` Vite bundle loaded via `<script src="/remote-assets/remote-next.js">`). Both pages share the same basic-auth gate, settings/profile state, and `saturn-bridge` TCI websocket; `/remote-next` carries the active extraction work while `/remote` remains the stable fallback.
- Current Saturn Remote beta URL: `https://<host>:8443/remote-next?phase42_split=1&phase44_tx_opus=1&phase44_tx_cfc=1&client_bust=bridgeprefill240-cfcessb3`. This enables Phase 42 split transport, Phase 44 Opus wideband TX, and the conservative ESSB CFC profile validated for clear Chrome Android TX audio.
- Shared remote settings persist in `/var/lib/saturn-state/remote_settings.json`.
- Named remote Setup profiles persist in `/var/lib/saturn-state/remote_profiles.json`.
- The remote `Setup` menu now supports:
  - saving the current radio and display state as a named server-side profile
  - loading and deleting saved profiles
  - assigning one saved profile as the startup profile
  - advanced panadapter/waterfall display presets such as spectrum floor/ceiling, waterfall floor/ceiling, averaging, speed, palette, peak hold, and grid/marker toggles
- The remote `Setup -> DSP` panel now supports:
  - `NR` controls for mode, amount, taps, delay, gain, and leakage
  - `ANF` controls for enable, taps, delay, gain, and leakage
- When a startup profile is set, the remote applies it on page load before connecting.
- The live remote meter panel now uses smoothed operator-facing analog behavior for S-meter, TX power, and SWR instead of raw stepwise updates.
- The TX power meter now supports `Peak` and `Avg` display modes.
- Browser disconnect/pagehide now forces an explicit TX release so the bridge does not stay keyed after a client vanishes mid-transmit.
- Two-tone test now persists server-side and supports configurable `freq1`, `freq2`, `level`, `invert LSB-family`, and `tone 2 delay` settings.
- USB/LSB panadapter and waterfall alignment now keep the signed passband box on the correct side of center when switching modes.

## Uninstall

Uninstaller aligned to the current installer:

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/uninstall_saturn_go_nginx.sh [--purge] [--no-purge] [--keep-auth] [--remove-packages] [--dry-run] [--yes]
```

Flags:

- Default behavior keeps `/opt/saturn-go`, `/var/lib/saturn-web`, and `/var/lib/saturn-state` (including custom scripts/state) for safer reinstalls
- `--purge`: remove `/opt/saturn-go`, `/var/lib/saturn-web`, and `/var/lib/saturn-state` for a clean slate
- `--no-purge`: explicit keep mode (same as default)
- `--keep-auth`: keep `/etc/nginx/.htpasswd`
- `--remove-packages`: best-effort removal of install-time packages
- `--dry-run`: print actions without making changes
- `--yes`: non-interactive confirmation

Default URL:

- `http://<host>/saturn/`

## Environment Variables

- `SATURN_ADDR` (default `127.0.0.1:8080`)
- `SATURN_WEBROOT` (default `/var/lib/saturn-web`)
- `SATURN_CONFIG` (default `$SATURN_WEBROOT/config.json`)
- `SATURN_SCRIPTS_DIR` (default `/opt/saturn-go/scripts`)
- `SATURN_REPO_ROOT` (default `$HOME/github/Saturn`)
- `SATURN_STATE_DIR` (installer default `/var/lib/saturn-state`)
- `SATURN_REPO_ROOT_FILE` (default `$SATURN_STATE_DIR/repo_root.txt`)
- `SATURN_MAX_BODY_BYTES` (default `2147483648`)
- `SATURN_RESTORE_MAX_UPLOAD_BYTES` (default `2147483648`)
- `SATURN_UPDATE_POLICY_FILE` (default `$SATURN_STATE_DIR/update_policy.json`)
- `SATURN_SATURNGO_UPDATE_POLICY_FILE` (default `$SATURN_STATE_DIR/saturngo_update_policy.json`)
- `SATURN_SATURNGO_DEPLOY_STATUS_FILE` (default `$SATURN_STATE_DIR/saturngo_deploy_status.json`)
- `SATURN_UPDATE_STATE_FILE` (default `$SATURN_STATE_DIR/update_state.json`)
- `SATURN_SNAPSHOT_DIR` (default `$SATURN_STATE_DIR/snapshots`)
- `SATURN_STAGING_DIR` (default `$SATURN_STATE_DIR/repo-staging`)
- `SATURN_CUSTOM_SCRIPTS_FILE` (default `$SATURN_STATE_DIR/custom_scripts.json`)
- `SATURN_PIHPSDR_ROOT` (default `$HOME/github/pihpsdr`)
- `SATURN_NGINX_CLIENT_MAX_BODY_SIZE` (installer default `2G`)
- `SATURN_WATCHDOG_URL` (installer default `http://$SATURN_ADDR/healthz`)
- `SATURN_WATCHDOG_INTERVAL` (installer default `30s`)
- `SATURN_ADMIN_PASSWORD` (optional non-interactive initial admin password)
- `SATURN_SERVICE_USER` (installer override for service user)
- `SATURN_SERVICE_GROUP` (installer override for service group)
- `SATURN_INSTALL_BRIDGE` defaults to `1`; set it to `0` to opt out of building
  and installing `saturn-bridge.service`
- `SATURN_REQUIRE_BRIDGE` defaults to `SATURN_INSTALL_BRIDGE`; when enabled,
  native source/build failures fail the full installation
- `SATURN_BRIDGE_WDSP_FLAVOR` defaults to `wdsp2`; `pihpsdr` is the explicit
  legacy-library mode
- `SATURN_WDSP2_REPO_URL` / `SATURN_WDSP2_REF` override the pinned TAPR WDSP
  source (default ref `584e8aca5ba1c4c6bc66fc0cc164ce567c8ba1e3`)
- `SATURN_PIHPSDR_PORT_REPO_URL` / `SATURN_PIHPSDR_PORT_REF` override the pinned
  Linux port shim source (default ref `974acbac07fe7dd3e24f28f3956a9ffb3a1ebaf1`)
- `SATURN_SATURNGO_BUILD_BRIDGE` defaults to `1` for Saturn Go self-update;
  set it to `0` only when intentionally retaining the installed bridge
- `SATURN_PIHPSDR_DIR` is used only by the explicit `pihpsdr` legacy mode

## Troubleshooting

- UI loads but script output fails:
  - Check `systemctl status saturn-go.service`
  - Verify script exists and is executable in `/opt/saturn-go/scripts`
- Saturn Go page shows `ERR:` lines during a successful run:
  - `cargo` and `systemd-run` may print normal informational lines to stderr
  - check the progress terminal output and `Last Deploy Status` panel before treating as failure
- Change Password fails:
  - Ensure `/usr/local/lib/saturn-go/scripts/saturn-admin-password.sh` exists and
    the service user's sudoers policy allows `set`/`status` (rerun the installer
    if missing); `htpasswd` comes from `apache2-utils`
  - Diagnose with `sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status`
- Versions panel is blank or `unknown`:
  - Verify `version` keys in `/var/lib/saturn-web/config.json`
- Clone target dropdown is empty:
  - Use `lsblk` (not `df`) to verify Linux sees the reader/card as a block device
  - Unmounted USB/SD readers will not appear in `df`
  - Insert the card before connecting some USB readers, then check `dmesg -w`, `lsusb`, and `lsblk`
- System powers off soon after boot:
  - Check `systemctl status saturn-shutdown-waiter.service`
  - Verify `/etc/default/saturn-shutdown-waiter` mode (`SATURN_SHUTDOWN_WAITER_ENABLED=false|auto|true`) for the hardware profile
  - Review recent waiter logs: `journalctl -u saturn-shutdown-waiter.service -n 100 --no-pager`
- `update-pihpsdr.py` fails with `UnicodeEncodeError` on `latin-1` output:
  - Update the deployed `/opt/saturn-go/scripts/update-pihpsdr.py` from this repo; current script degrades unsupported symbols on non-UTF-8 streams and writes logs as UTF-8
- Installer fails building Rust server with `lock file version '4'` / old Cargo:
  - Current installer now removes legacy apt `cargo`/`rustc` and installs a modern stable toolchain via `rustup`
  - If rerunning after a failed older installer, run `sudo bash update_manager/install_saturn_go_nginx.sh` again

## Credits

- Original Saturn Update Manager by Jerry DeLong, KD4YAL
- Saturn Update Manager Rust backend and UI workflow extensions in this repo
