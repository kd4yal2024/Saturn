# Architecture

## System Overview

Saturn Update Manager is deployed as a small appliance-style web stack:

1. NGINX handles HTTP entry, Basic Auth, and reverse proxy.
2. Rust backend (`saturn-go`, Axum) serves pages and API endpoints.
3. Backend launches shell/Python scripts for maintenance tasks.
4. Systemd keeps the backend running and uses `/livez` for process-liveness restart decisions.

The Saturn Remote real-time backend also lives in the repo:

- `saturn-bridge`
  - separate from `saturn-go`
  - provides the direct G2 remote/session bridge used by Saturn Remote
  - owns the downstream Protocol 2 client role to `p2app`
  - provides the TCI/WebSocket boundary and native WDSP integration
  - is gaining an opt-in direct XDMA backend in staged, fail-safe phases; see
    `SATURN_BRIDGE_XDMA.md` for the ownership and client-switching contract

## Request Flow

1. Browser requests `http://<host>/saturn/`.
2. NGINX enforces Basic Auth (`/etc/nginx/.htpasswd`).
3. NGINX proxies to Rust backend (`SATURN_ADDR`, default `127.0.0.1:8080`).
4. Backend returns HTML from `/var/lib/saturn-web` and JSON/SSE API responses.
5. For script execution, backend spawns scripts from `/opt/saturn-go/scripts` and streams output via SSE.

## UI Page Responsibilities

- `overview.html`
  - Default landing page (`/`) for appliance, radio, network, service, and last-deployment health with quick workflow links.
  - Uses an isolated system-rate sampling scope so its polling does not alter Monitor chart baselines.
- `index.html`
  - Custom Scripts page (browser-managed script catalog + runner) and password change.
  - `update-G2.py`, `update-pihpsdr.py`, `update-deskhpsdr.py`, and `restore-backup.sh` are intentionally hidden from this page dropdown.
  - Supports browser file upload for custom script content and includes backend-seeded default custom cleanup scripts.
- `update.html`
  - G2 Update page that combines:
    - Update G2 terminal workflow (`POST /run` with `update-G2.py`)
    - Appliance Update policy/start/status/rollback controls (repo URL + branch/ref + healthcheck inputs in current UI).
    - G2 run button is gated by valid Appliance repo URL input.
- `saturngo.html`
  - Dedicated Saturn Go self-update terminal workflow (`POST /run` with `update-saturn-go.sh`) plus repo/ref policy and last-deploy-status panels.
- `pihpsdr.html`
  - Dedicated piHPSDR update terminal workflow (`POST /run` with `update-pihpsdr.py`).
- `deskhpsdr.html`
  - Dedicated deskHPSDR update terminal workflow (`POST /run` with `update-deskhpsdr.py`).
  - The script clones/pulls `~/github/deskhpsdr` and delegates build/install-dependency behavior to helper scripts from the active Saturn repo root.
- `saturn-remote-next.html` + `saturn-remote-next.js`
  - The Saturn Remote UI, served at `/remote-next` (legacy `/remote` paths redirect here since 2026-07-14).
  - Uses DOM for connection/session/radio controls and WebGL2 for high-DPI panadapter and waterfall rendering.
  - Targets `saturn-bridge` as the remote protocol boundary instead of talking to `p2app` directly.
  - The page loads the Vite IIFE bundle `saturn-remote-next.js` via `<script src="/remote-assets/remote-next.js">`; the bundle exposes `globalThis.SaturnRemoteNext` for the inline page script.
  - The bundle is built from the `update_manager/remote-web` TypeScript project (`vite build` -> `dist/saturn-remote-next.js`) by both the installer and `update-saturn-go.sh` self-update.
- `fpga.html`
  - Dedicated FPGA flash workflow (`POST /run` with `flash_fpga.sh`) plus image discovery (`GET /get_fpga_images`).
- `backup.html`
  - Repo-root selection, repository backup/restore, repair pack, and configuration verification.
  - Whole-disk imaging, cloning, and target wiping are intentionally local-console maintenance functions and are not exposed by Saturn Go.
- `monitor.html`
  - Real-time system monitoring and process controls.
- `p23test.html`
  - Radio Telemetry & Diagnostics page served canonically at `/telemetry`.
  - Combines live `p2app`, FPGA, network, and XDMA telemetry with advanced build/deploy/restart and override controls.
  - Includes the persistent Performance Lab for fixed-window, workload-locked baseline/candidate comparisons; see `PERFORMANCE_LAB.md`.
  - Legacy `/p23test` routes remain available for compatibility.
- `tailscale.html`
  - Tailscale VPN status, enrollment, service, and Saturn Remote access controls.

All HTTP management pages use the shared offline appliance shell under
`templates/assets/`, which supplies grouped navigation, responsive behavior,
design tokens, local fonts, and vendored browser libraries.

## Runtime Layout

### Deployed Paths

- `/opt/saturn-go/bin/saturn-go`
  - Rust server binary.
- `/opt/saturn-go/scripts/`
  - Executable maintenance scripts (also target directory for browser-managed custom scripts).
- `/var/lib/saturn-web/`
  - Web assets include `overview.html`, management/telemetry/application HTML
    pages, Saturn Remote HTML and bundle files, `assets/` (shared shell, local
    fonts, and vendored libraries), `config.json`, and `themes.json`. Optional
    Saturn Remote helper scripts are also synced when present.
- `/var/lib/saturn-state/`
  - Mutable settings, update/deployment records, release transaction state,
    remote TLS/remembered-device identity, snapshots, and staged worktrees.
  - `STATE_INVENTORY.md` classifies each item by portability, sensitivity,
    recovery importance, and support-bundle handling; not all files under this
    root belong in a portable settings backup.
- `/etc/systemd/system/saturn-go.service`
  - Main backend service.
- `/etc/systemd/system/saturn-go-watchdog.service`
- `/etc/systemd/system/saturn-go-watchdog.timer`
  - Periodic process-liveness check and restart logic.
- `/usr/local/lib/saturn-go/saturn-health-watchdog.sh`
  - Root-owned watchdog script executed by the watchdog service unit.
- `/etc/nginx/sites-available/saturn`
- `/etc/nginx/conf.d/saturn_sse_map.conf`
  - NGINX proxy config and SSE behavior.

### Source Paths

- `update_manager/rust-server/`
  - Rust backend source code.
- `update_manager/saturn-bridge/`
  - Standalone direct-G2 bridge source code and native WDSP integration.
- `update_manager/templates/`
  - UI templates copied to web root during install, including the shared
    offline shell in `templates/assets/` and both Saturn Remote pages.
- `update_manager/remote-web/`
  - Vite TypeScript project that builds the `saturn-remote-next.js` IIFE bundle consumed by `/remote-next`.
- `update_manager/docs/SATURN_REMOTE_ARCHITECTURE.md`
  - Detailed frontend/rendering contract for `saturn-remote` and `/remote-next`.
- `update_manager/scripts/`
  - Runtime scripts copied to `/opt/saturn-go/scripts`. The shared web asset manifest `saturn-go-web-assets.sh` lives here and is sourced by both the installer and `update-saturn-go.sh`.

Browser-managed custom scripts:

- Metadata persisted at `SATURN_CUSTOM_SCRIPTS_FILE` (default `/var/lib/saturn-state/custom_scripts.json`).
- Optional script content writes to `SATURN_SCRIPTS_DIR` (default `/opt/saturn-go/scripts`).
- The loader enforces bounded metadata size:
  - `custom_scripts.json` is rejected if it exceeds the backend size cap
  - script count, flag count, and per-flag length are clamped on load/save
- Backend seeds default custom entries (and missing files) on startup:
  - `cleanup-saturn-logs.sh`
  - `cleanup-saturn-backups.sh`

## Saturn Remote Display Model

`saturn-remote` is intentionally split into two UI layers:

- DOM layer
  - session controls
  - VFO entry
  - mode/filter controls
  - textual telemetry and diagnostics
- GPU canvas layer
  - panadapter
  - waterfall
  - dense, per-frame visual overlays

Current transport posture:

- LAN-first
  - consume raw IQ from `saturn-bridge` over the first TCI/WebSocket path
- WAN-later
  - move toward display-oriented FFT row transport and compressed audio once the remote stack grows beyond same-LAN operation

See `SATURN_REMOTE_ARCHITECTURE.md` for the concrete bridge/display contract.

## Core State Model

- Active repo root is held in memory and persisted to `repo_root.txt`.
- Startup canonicalizes the default repo root and any saved `repo_root.txt`
  value before Saturn repo validation.
- Update policy is persisted in `update_policy.json`.
- Last successful update/rollback metadata is persisted in `update_state.json`.
- Snapshot archives are stored in `snapshots/`.
- Transactional update worktrees are stored in `repo-staging/`.
- A process-local update activity lock coordinates mutually exclusive update operations (appliance update, appliance rollback, and Update G2 runs).
- Process-local per-script run-log buffers (in memory) back `/run_log` resume behavior for terminal pages.

## Health Model

- `GET /livez` reports process liveness and the full Git commit embedded in the
  running binary. The systemd watchdog uses this endpoint so a temporary radio
  or bridge dependency failure does not create a restart loop.
- `GET /readyz` returns structured required and optional component results.
  Release identity, state writability, configuration parsing, free disk space,
  and Saturn Bridge reachability are required. P2 and XDMA are reported
  separately so absent hardware does not roll back an otherwise valid
  application deployment.
- The installer and Saturn Go root deployment broker call
  `/readyz?expected_commit=<full-sha>`. A still-running old binary or a staged
  binary built from the wrong commit therefore cannot validate the target.
- Canonical appliance provisioning intentionally installs the manager and
  Bridge in separate steps. The nested manager install defers its final probe;
  the provisioning orchestrator performs the exact-commit `/readyz` check only
  after Bridge installation. Standalone manager installation continues to
  verify readiness before it returns.
- `GET /healthz` is a temporary compatibility alias for `/livez`; it is not a
  deployment readiness signal and is scheduled for removal after the 2026
  transition.

## Security Model

### Access Control

- External access is protected by NGINX Basic Auth.
- Backend is intended to bind loopback by default (`127.0.0.1:8080`).

### CSRF Protection

All mutating (`POST`) routes require:

- Header `X-Saturn-CSRF: 1`
- `Host` header present
- If `Origin` or `Referer` exists, its host must match request host

### Path/Target Safety

- Repo root switching and restore target must pass Saturn repo validation:
  - directory exists
  - `.git` exists
  - `update_manager/` exists
- Restore archive is rejected if any tar entry is absolute or includes `..`.
- Restore also rejects unsafe symlink targets, oversized expansion ratios, and
  archives that would exceed `/tmp` free space before extraction.
- Restore requires explicit `confirm=RESTORE` unless dry-run mode is used.
- Script runner rejects script names containing path traversal or separators.

## Transactional Appliance Update Flow

`POST /update_start` starts an async update job.

1. Load and normalize update policy.
2. Probe expected public policy URL (`https://github.com/<owner>/<repo>.git`) and selected ref with `git ls-remote`.
3. `git fetch --prune <policy-url> <target-ref>` on active repo, leaving local remotes unchanged.
4. Resolve target commit from `FETCH_HEAD`.
5. If unchanged, mark job `no_change`.
6. Optional snapshot creation (`tar`) with retention pruning.
7. Create detached staged worktree in `repo-staging`.
8. Switch active repo root to staged worktree.
9. Run health check URL.
10. On failure: revert repo root and remove staged worktree.
11. On success: persist last update state and prune older staged worktrees.

Rollback (`POST /update_rollback`) re-points active repo root to `previous_repo_root` from last update state and re-runs health check.

Concurrency guard:

- Appliance update and rollback acquire the shared update-activity lock.
- If another update activity is already running, these routes return `409 Conflict`.

## Script Execution Model

- `POST /run` accepts multipart form:
  - `script=<filename>`
  - zero or more `flags=<flag>` values
- `GET /run_log` returns buffered output for a script by offset:
  - `?script=<filename>&from=<offset>&limit=<n>`
- Backend starts script from `SATURN_SCRIPTS_DIR` (default `/opt/saturn-go/scripts`).
- `.py` scripts are rejected if their resolved path is inside active `SATURN_REPO_ROOT`.
- Output from stdout/stderr is streamed as SSE messages.
- Output is also copied into an in-memory per-script ring buffer so UI pages can resume output after tab/page switches.
- `stdbuf` + unbuffered Python mode are used when available to reduce output latency.
- Backend injects active repo-root context into child processes:
  - `SATURN_REPO_ROOT`
  - `SATURN_DIR`
  - `SATURN_ACTIVE_REPO_ROOT`
- Python child runs also set:
  - `PYTHONDONTWRITEBYTECODE=1`
  - `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`
- For `update-G2.py`/`update-G2.sh`, `/run` also acquires the shared update-activity lock; conflicting update activity returns `409 Conflict`.

## Backup/Restore Model

- Settings backup (`GET /backup_settings`) stages a bounded allowlist of
  regular portable/review-before-transfer files in a private temporary tree,
  writes per-file SHA-256 metadata, and streams a schema-v1 `tar.gz`.
- Source backup (`GET /backup_source`) streams a `tar.gz` of active repo root.
  Historical `GET /backup_full` is a compatibility alias for this source
  archive, not a complete appliance backup.
- Installed release listing/download (`GET /backup_releases`,
  `GET /backup_release?commit=...`) exports one full-commit, manifest-bearing,
  real directory directly below the configured immutable releases root.
- Exact contents and omissions are defined in `STATE_INVENTORY.md` and
  `BACKUP_FORMATS.md`.
- Settings restore (`POST /restore_settings`) validates the schema-v1 manifest,
  exact file set, hashes, ownership, permissions, semantics, and capacity. It
  stages durable old/new copies and atomically replaces bounded files under a
  journal; host-specific policy is opt-in.
- Source restore (`POST /restore_source`, compatibility alias
  `POST /restore_full`) validates and extracts one Saturn repository, copies it
  to a flushed generation, then atomically switches `repo_root.txt` while
  retaining the prior checkout.
- Saturn Go runs restore recovery before reading `repo_root.txt`; incomplete
  settings transactions and incomplete source-pointer switches roll back.
- Dry-run restore performs the same content and capacity preflight without
  creating or activating a transaction. Apply requires `confirm=RESTORE` and
  the shared update-activity lock.
- Update G2 directory backups (`GET /g2_backups`, `POST /g2_restore`): lists `saturn-backup-*` directories under backend `$HOME` and restores selected backup through the transactional source-generation path.
- piHPSDR directory backups (`GET /pihpsdr_backups`, `POST /pihpsdr_restore`): lists `pihpsdr-backup-*` directories under backend `$HOME` and restores selected backup into configured piHPSDR checkout.

## Monitor Model

- `GET /get_system_data` returns CPU, memory, swap, disk, network, load, uptime, temperature, and process list.
- `POST /kill_process/:pid` supports controlled process termination with safeguards for protected/root-owned processes.
- Monitor UI polls every 1 second for near-real-time display.
