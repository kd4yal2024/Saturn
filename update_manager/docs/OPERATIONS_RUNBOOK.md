# Operations Runbook

## Scope

This runbook covers day-2 operations for the Rust-based Saturn Update Manager deployment.

## Build and Install

### Build Backend Locally

```bash
cd /home/pi/github/Saturn/update_manager/rust-server
cargo check
cargo build --release
```

### Full Install (recommended)

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/install_saturn_go_nginx.sh
```

Installer actions include:

- installs dependencies (`nginx`, `apache2-utils`, build tools, Python tools, etc.)
- removes legacy distro `cargo`/`rustc` packages (if present)
- bootstraps/updates a current stable Rust toolchain via `rustup` for the build user, then validates Cargo can read the repo lockfile
- builds and deploys Rust binary to `/opt/saturn-go/bin/saturn-go`
- copies web assets to `/var/lib/saturn-web`
- copies scripts to `/opt/saturn-go/scripts`
- grants service-user ownership of `/opt/saturn-go/scripts` so browser-managed custom script edits can persist
- writes NGINX config for `/saturn/*` and SSE route `/saturn/run`
- writes `saturn-go.service`, watchdog service, and watchdog timer
- waits for backend health at `/healthz`

## Update Existing Deployment

After pulling repo changes, run installer again:

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/install_saturn_go_nginx.sh
```

Installer is designed to refresh service, web assets, scripts, and config.

If an older install attempt failed with a Cargo lockfile parse error (for example
`lock file version '4'` on Bookworm using distro `cargo`), rerun the installer.
Current installer versions self-bootstrap a newer Rust toolchain via `rustup`.

## Uninstall

```bash
cd /home/pi/github/Saturn
sudo bash update_manager/uninstall_saturn_go_nginx.sh [--purge] [--no-purge] [--keep-auth] [--remove-packages] [--dry-run] [--yes]
```

Default behavior keeps runtime directories (including custom scripts and state):

- `/opt/saturn-go`
- `/var/lib/saturn-web`
- `/var/lib/saturn-state`

Use `--purge` for a full cleanup.

## Service Operations

### Status and Logs

```bash
sudo systemctl status saturn-go.service
sudo journalctl -u saturn-go.service -n 200 --no-pager
sudo systemctl status saturn-go-watchdog.timer
```

### Restart

```bash
sudo systemctl restart saturn-go.service
sudo systemctl restart saturn-go-watchdog.timer
```

### NGINX Validation

```bash
sudo nginx -t
sudo systemctl reload nginx
```

### API Quick Checks

Through NGINX (authenticated session in browser) or locally against backend:

```bash
curl -fsS http://127.0.0.1:8080/healthz
curl -fsS http://127.0.0.1:8080/update_status
curl -fsS http://127.0.0.1:8080/list_repo_roots
curl -fsS "http://127.0.0.1:8080/run_log?script=update-G2.py&from=0&limit=20"
```

## Key Workflows

### Navigation Layout

- `/saturn/` opens G2 Update (default landing page).
- `/saturn/pihpsdr` opens dedicated piHPSDR update page.
- `/saturn/fpga` opens dedicated FPGA flash page.
- `/saturn/backup` opens Backup / Restore.
- `/saturn/custom` (and `/saturn/index`) opens Custom Scripts page.
- `/saturn/monitor` opens Monitor.
- Navigation order in current UI: `G2 Update` -> `piHPSDR Update` -> `FPGA Flash` -> `Backup / Restore` -> `Custom Scripts` -> `Monitor`.

### Repo Root Management

- Use `backup.html` repo root controls, or call:

```bash
curl -sS -X POST http://127.0.0.1:8080/set_repo_root \
  -H 'Content-Type: application/json' \
  -H 'X-Saturn-CSRF: 1' \
  -d '{"repo_root":"/home/pi/github/Saturn"}'
```

Validation requires `.git` and `update_manager/` in the target path.

### Backup and Restore

- Download full backup from `backup.html` (or `GET /backup_full`).
- Validate archive first with restore dry-run (`POST /restore_full?dry_run=1`).
- Apply restore only after confirmation (`confirm=RESTORE`).
- For script-created directory backups, use Backup page "Script Backups" controls:
  - Saturn backups from `update-G2.py`: `GET /g2_backups`, `POST /g2_restore`
  - piHPSDR backups from `update-pihpsdr.py`: `GET /pihpsdr_backups`, `POST /pihpsdr_restore`

Important:

- restore overwrites active repo root using `rsync --delete`
- upload size is limited by `SATURN_RESTORE_MAX_UPLOAD_BYTES`
- non-dry-run full restore acquires the shared update lock; concurrent update actions return `409 Conflict`

### Clone SD Card to USB/SD Reader

Recommended workflow from `backup.html`:

1. Click `Refresh` and select the target device (for example `/dev/sdX`).
2. Optional: click `Wipe Target` to clear partition/signature metadata before cloning.
3. Click `Start Clone` and monitor progress/status/log output in the clone panel.

Notes:

- `Wipe Target` is a quick pre-clone wipe (metadata/signatures), not a full secure erase.
- The clone UI progress bar updates from `clone_pi_to_device.sh` progress messages (best results when `pv` is installed).
- Device detection uses block devices, not mounted filesystems.
- Use `lsblk` to verify the reader/card is detected; `df` will not show unmounted targets.
- Some USB readers only enumerate when the SD card is inserted before plugging in.
- If the dropdown is empty, check `dmesg -w`, `lsusb`, and `lsblk`.

### Appliance Update

1. Open G2 Update page (`/saturn/update`).
2. Enter GitHub repo URL and branch/ref.
3. Configure health-check URL and timeout.
4. Save settings (optional; Start also persists current values).
5. Start update.
6. Monitor `update_status` job until complete.
7. If needed, run rollback.

Current UI behavior:

- UI persists policy using `channel=custom` and `custom_ref=<branch/ref>`.
- Appliance policy panel stores repo/ref/health settings consumed by both transactional Appliance Update and `Run Update G2`.
- `Run Update G2` requires valid Appliance repo URL before run can start.
- `Run Update G2` auto-saves current Appliance settings before spawning script.
- Terminal output is resumable after tab/page changes using buffered `/run_log` polling.

G2 Update coordination notes:

- Update G2 terminal and Appliance Update now live on the same page.
- If Appliance Update already moved Git to target commit, run Update G2 with `--skip-git`.
- G2 runs and appliance update/rollback are mutually exclusive; overlapping requests return `409 Conflict`.

Update behavior:

- updates Git remote to expected GitHub URL from policy
- fetches target ref
- snapshots active repo (if enabled)
- stages update in `repo-staging` worktree
- switches active repo root only after staging
- health-check gates completion; failed checks auto-revert root

### Update G2 (Dedicated Terminal)

- Run `update-G2.py` from the G2 Update page to keep terminal output and Appliance Update state together.
- Repo URL in Appliance section must be valid before G2 run is enabled.
- Backend injects active repo-root environment for `/run`:
  - `SATURN_REPO_ROOT`
  - `SATURN_DIR`
  - `SATURN_ACTIVE_REPO_ROOT`
- `/run` rejects Python script execution when the resolved script path is inside active `SATURN_REPO_ROOT`.
- Python runs from `/run` set `PYTHONDONTWRITEBYTECODE=1` and `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`.
- This allows `update-G2.py` to target the active Saturn checkout without hardcoded path dependence.
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.

### Saturn Go Self-Update (Dedicated Terminal + Redeploy)

- Open `/saturn/saturngo`.
- The page stores a separate Saturn Go repo/ref policy (`GET/POST /saturngo_policy`) so it does not overwrite the G2 Appliance Update policy.
- The page runs `update-saturn-go.sh` via `/run` with live terminal output and buffered resume (`/run_log`).
- The page provides explicit run options that map to script flags:
  - `Verbose output` -> `--verbose`
  - `Dry run` -> `--dry-run`
  - `Skip Git update` -> `--skip-git`
  - `Skip build` -> `--skip-build`
  - `Skip deploy` -> `--skip-deploy`
- `Last Deploy Status` panel polls `GET /saturngo_deploy_status` (status JSON written by the script and detached root helper).

Typical test sequence:

1. `--verbose --dry-run --skip-git` (preview all steps)
2. `--verbose --skip-git --skip-build` (fast deploy path using existing release binary)
3. `--verbose --skip-git` (full local rebuild + redeploy)
4. `--verbose` (full git pull + rebuild + redeploy from Saturn Go policy)

Operational notes:

- The script builds from the active repo root selected on Backup / Restore page (`SATURN_ACTIVE_REPO_ROOT`).
- Final stop/copy/start of `saturn-go.service` is dispatched via detached `systemd-run` helper (`saturn-go-self-deploy-<timestamp>`).
- The web terminal may disconnect when `saturn-go.service` restarts; reload after ~10-20 seconds.
- Some successful lines may still be prefixed `ERR:` in the terminal because `cargo` and `systemd-run` emit informational output on stderr.

### P2/P3 App Test Lab (Hidden / Experimental)

- Open `/saturn/p23test` directly (it is intentionally not linked in navigation).
- This page is intended for testing `P2_app` vs `P3_app` builds/deploys and service switching.
- It runs `p23-app-manager.sh` via `/run` and resumes terminal output using `/run_log`.

Capabilities:

- `Status` (script-based status summary)
- `Build P2` / `Build P3`
- `Deploy + Switch P2` / `Deploy + Switch P3`
- `Switch To P2` / `Switch To P3` (uses previously deployed binaries)
- `Revert To Unit Default` (removes Saturn override and restores unit `ExecStart`)
- `Emergency Revert Now` button (forces a real revert + restart even if `Dry run` / `No restart` options are selected)
- Separate status panel backed by `GET /p23_status`

Switching implementation details:

- Deployed binaries are staged under `/opt/saturn-go/p23-apps/`
- Active selection is `/opt/saturn-go/p23-apps/current` symlink
- `p2app.service` is redirected via systemd drop-in:
  - `/etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf`
- Revert action removes that drop-in and reloads systemd
- Switch/deploy overrides can include:
  - startup profile (`panel`, `panel-debug`, `headless`) -> mapped service args
  - `Environment=SATURN_FRONT_PANEL_MODE=...` (`auto`, `g2`, `g2v2`, `prefer-g2`, `prefer-g2v2`, `off`)
- `GET /p23_status` parses the generated override comment metadata (`# saturn-p23 mode=... panel=...`) for display

Safety/usage notes:

- Use `Dry run` first for deploy/switch/revert actions
- Non-dry-run switch/deploy/revert actions require browser confirmation
- Web mode requires `sudo -n` permission for install/symlink/systemctl steps
- `No restart` updates symlink/override without restarting `p2app.service`
- If a switch leaves the local panel UI unusable but networking still works (e.g. Thetis continues to connect), use `/saturn/p23test` from another device and click `Emergency Revert Now`

### Update piHPSDR (Dedicated Terminal)

- Run `update-pihpsdr.py` from `/saturn/pihpsdr`.
- This page mirrors the dedicated terminal workflow (flags + SSE output) used by Update G2.
- In non-interactive web execution, backup prompts are skipped unless `-y` is selected.
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.
- On systems exposing non-UTF-8 stdout/stderr (for example `latin-1`), the updater now degrades unsupported status symbols instead of crashing; mirrored log files are written as UTF-8.

### FPGA Flash (Dedicated Terminal)

- Run `flash_fpga.sh` from `/saturn/fpga`.
- The page discovers candidate images from `GET /get_fpga_images`.
- Use `Show only most current firmware` to limit dropdown selection to `latest_image` from backend scan.
- The script uses `sw_tools/load-FPGA/load-FPGA` with:
  - `-b <image>`
  - optional `-v` verify (enabled by default in UI)
  - optional `-f` fallback slot
- Flash is confirmation-gated (`--confirm FLASH` or short hash shown by script).
- In web mode, service-user passwordless sudo is required for hardware flashing.
- Output can be resumed from backend run logs (`/run_log`) after navigating away and back.

### Custom Scripts (Browser Managed)

- Use `/saturn/custom` to add/update/delete custom script entries from browser.
- Optional script content can be written directly to scripts directory.
- Custom script metadata is persisted in `SATURN_CUSTOM_SCRIPTS_FILE`.
- Default custom scripts are seeded by backend startup:
  - `cleanup-saturn-logs.sh`
  - `cleanup-saturn-backups.sh`
- Custom script output also resumes through `/run_log` buffering.

### Backup and Restore Scope

- Backup / Restore page now focuses on:
  - repo-root selection
  - full backup/restore
  - repair pack and config verification
  - Pi image and removable-device clone workflows

### Password Change

`POST /change_password` updates `/etc/nginx/.htpasswd` for user `admin`.

If direct write is denied, backend retries with `sudo -n`. Ensure service user has sudoers permission for `htpasswd` if this route should work non-interactively.

### Monitor and Process Control

- `monitor.html` polls `/get_system_data` every 1 second.
- process kill button calls `POST /kill_process/:pid` with CSRF header.
- backend blocks protected/root-owned process targets.

## Environment Variables

Service environment commonly used in deployment:

- `SATURN_ADDR` (default `127.0.0.1:8080`)
- `SATURN_WEBROOT` (default `/var/lib/saturn-web`)
- `SATURN_CONFIG` (default `$SATURN_WEBROOT/config.json`)
- `SATURN_SCRIPTS_DIR` (default `/opt/saturn-go/scripts`)
- `SATURN_STATE_DIR` (default `/var/lib/saturn-state`)
- `SATURN_REPO_ROOT_FILE` (default `$SATURN_STATE_DIR/repo_root.txt`)
- `SATURN_UPDATE_POLICY_FILE` (default `$SATURN_STATE_DIR/update_policy.json`)
- `SATURN_SATURNGO_UPDATE_POLICY_FILE` (default `$SATURN_STATE_DIR/saturngo_update_policy.json`)
- `SATURN_SATURNGO_DEPLOY_STATUS_FILE` (default `$SATURN_STATE_DIR/saturngo_deploy_status.json`)
- `SATURN_UPDATE_STATE_FILE` (default `$SATURN_STATE_DIR/update_state.json`)
- `SATURN_SNAPSHOT_DIR` (default `$SATURN_STATE_DIR/snapshots`)
- `SATURN_STAGING_DIR` (default `$SATURN_STATE_DIR/repo-staging`)
- `SATURN_CUSTOM_SCRIPTS_FILE` (default `$SATURN_STATE_DIR/custom_scripts.json`)
- `SATURN_PIHPSDR_ROOT` (default `$HOME/github/pihpsdr`)
- `SATURN_FPGA_DIR` (optional override for FPGA image scan path)
- `SATURN_MAX_BODY_BYTES` (default `2147483648`)
- `SATURN_RESTORE_MAX_UPLOAD_BYTES` (default `2147483648`)
- `SATURN_NGINX_CLIENT_MAX_BODY_SIZE` (installer default `2G`)
- `SATURN_WATCHDOG_URL` (default `http://$SATURN_ADDR/healthz`)
- `SATURN_WATCHDOG_INTERVAL` (default `30s`)

## Troubleshooting

### UI Loads, API Calls Fail

1. Check backend service status/logs.
2. Confirm NGINX proxy config is valid.
3. Confirm backend bind address matches NGINX proxy target.

### Script Runs Show No Output or Slow Output

- verify script exists and is executable in `/opt/saturn-go/scripts`
- check service logs for spawn errors
- check NGINX still has dedicated `/saturn/run` SSE location
- verify buffered lines are available:

```bash
curl -sS "http://127.0.0.1:8080/run_log?script=update-G2.py&from=0&limit=50" | jq
```

### Restore Errors

Common causes:

- confirm token missing for non-dry-run restore
- archive too large for configured upload limit
- archive contains unsafe paths or unexpected top-level layout

### Appliance Update Errors

Common causes:

- invalid policy values (refs/owner/repo)
- remote fetch failures
- health check URL failure after staging
- insufficient disk space for snapshots/staging
- overlapping update actions (G2/appliance update/rollback) triggering `409 Conflict`

Check:

```bash
curl -sS http://127.0.0.1:8080/update_status | jq
ls -lah /var/lib/saturn-state/snapshots
ls -lah /var/lib/saturn-state/repo-staging
```

### Saturn Go Self-Update Errors

Common causes:

- Saturn Go policy repo URL not saved/invalid
- local repo has uncommitted changes and run omitted `--skip-git`
- `sudo -n` permissions missing for deploy copy/restart commands
- service restart happened, but browser did not reconnect yet

Check:

```bash
curl -sS http://127.0.0.1:8080/saturngo_deploy_status | jq
sudo cat /var/lib/saturn-state/saturngo_deploy_status.json
sudo systemctl status saturn-go.service
sudo journalctl -u saturn-go.service -n 200 --no-pager
```

### P2/P3 Test Lab Errors

Common causes:

- `P3_app` or `P2_app` source tree missing under active repo root
- build failure in `make`
- `sudo -n` denied for deploy/switch/revert (`install`, `ln`, `systemctl`)
- stale or unexpected systemd override contents from manual edits
- wrong front-panel detection mode after switching (try `Front panel mode = g2` or `g2v2` instead of `auto`)

Check:

```bash
curl -sS http://127.0.0.1:8080/p23_status | jq
sudo systemctl status p2app.service
sudo cat /etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf
ls -lah /opt/saturn-go/p23-apps
sudo /opt/saturn-go/scripts/p23-app-manager.sh --revert --verbose
```

### Main UI "Expected token '<'" / HTML Instead Of JSON

If the browser reports a JSON parse error but shows HTML content (for example a login page or an NGINX error page), the UI is receiving an HTML response from an API route such as `/custom_scripts`.

Check:

```bash
curl -i http://127.0.0.1:8080/custom_scripts
sudo systemctl status saturn-go.service
sudo journalctl -u saturn-go.service -n 100 --no-pager
```

Typical causes:

- `saturn-go.service` stopped/crashed (backend unavailable)
- reverse proxy/auth returned a login page or error page instead of JSON
- stale browser session/auth after service restart (refresh/login again)

### Verify Runtime File Set

```bash
curl -sS http://127.0.0.1:8080/verify_system_config | jq
```

### Export Repair Pack

```bash
curl -sS http://127.0.0.1:8080/repair_pack -o saturn-repair-pack.tar.gz
```
