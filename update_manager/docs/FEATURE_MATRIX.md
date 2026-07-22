# Feature Matrix

This matrix maps current capabilities to the implementation points in UI, backend API, scripts, and persisted state.

The Saturn Remote frontend ships as `/remote-next` (rendered from `saturn-remote-next.html` + the Vite IIFE bundle `saturn-remote-next.js`) against the `saturn-bridge` backend, using basic auth, `remote_settings.json`, `remote_profiles.json`, and the TCI websocket. The legacy `/remote` page (`saturn-remote.html`) was retired on 2026-07-14 and its paths redirect to `/remote-next`; capability rows below that still name `saturn-remote.html` are historical and describe behavior now owned by `/remote-next`. Current frontend scope is tracked in `update_manager/remote-web/README.md`.

| Capability | UI | API Endpoints | Scripts / Commands | State / Files |
|---|---|---|---|---|
| Appliance overview and shared offline shell | `overview.html`, `templates/assets/` | `GET /`, `GET /overview`, `GET /readyz`, `GET /assets/{*path}` | Local Inter/Tailwind/Chart.js/ansi_up assets; Nginx `/saturn/` proxy | Browser theme preference |
| Browser-managed custom script runner with live output | `index.html` (`/custom`) | `POST /run`, `GET /run_log`, `GET /maintenance_jobs` | `/opt/saturn-go/scripts/*` launched through the host lock/job broker | `custom_scripts.json`, in-memory resume buffer, durable maintenance job records/output/results |
| Custom script catalog management (add/update/delete + upload) | `index.html` (`/custom`) | `GET/POST /custom_scripts`, `POST /custom_scripts_delete` | Optional script file write/remove in scripts dir | `custom_scripts.json`, `/opt/saturn-go/scripts` |
| Backend-seeded default custom maintenance scripts | `index.html` (`/custom`) | `GET /custom_scripts` | `cleanup-saturn-logs.sh`, `cleanup-saturn-backups.sh` | `custom_scripts.json`, `/opt/saturn-go/scripts` |
| Dedicated Update G2 terminal runner | `update.html` (requires valid Appliance repo URL in UI) | `POST /run`, `GET /run_log` (with `script=update-G2.py`) | `update-G2.py` | Process-local update-activity lock, in-memory run-log buffer |
| G2 app/firmware info pull | `update.html` (`Show App / Firmware Info`) | `POST /run` (with `script=g2-version-info.sh`) | `g2-version-info.sh`, local `curl /p23_perf`, `journalctl -u p2app.service` | In-memory run-log buffer |
| Optional post-G2 web-manager self-update chain | `update.html` (`Update Web Manager Too`) | `POST /run` (first `update-G2.py`, then `update-saturn-go.sh`) | `update-G2.py` emits `SATURN_WEB_MANAGER_CHANGED=1` when `update_manager/` changed; page then launches `update-saturn-go.sh --skip-git --verbose` from the active repo root without a second repo pull | In-memory run-log buffer; detached saturn-go deploy status file |
| Dedicated piHPSDR terminal runner | `pihpsdr.html` | `POST /run`, `GET /run_log` (with `script=update-pihpsdr.py`) | `update-pihpsdr.py` | In-memory run-log buffer |
| Dedicated deskHPSDR terminal runner | `deskhpsdr.html` | `POST /run`, `GET /run_log` (with `script=update-deskhpsdr.py`) | `update-deskhpsdr.py`, `scripts/deskhpsdr-test-build-on-current-image.sh`, startup guard `scripts/patches/deskhpsdr-active-receiver-init.patch`, and conditional legacy `deskhpsdr-libgpiod-v2.patch` from active repo root | In-memory run-log buffer |
| Dedicated FPGA flash runner | `fpga.html` | `POST /run`, `GET /run_log` (with `script=flash_fpga.sh`), `GET /get_fpga_images` | `flash_fpga.sh` -> `sw_tools/load-FPGA/load-FPGA` | In-memory run-log buffer |
| Script catalog and flag metadata | `index.html`, `update.html`, `saturngo.html`, `pihpsdr.html`, `deskhpsdr.html` | `GET /get_scripts`, `GET /get_flags`, `GET /get_versions` | Reads `config.json` | `/var/lib/saturn-web/config.json` |
| Repo root discovery and switch | `backup.html` | `GET /list_repo_roots`, `GET /get_repo_root`, `POST /set_repo_root` | Path validation in backend | `repo_root.txt` |
| Portable settings backup download | `backup.html` | `GET /backup_settings` | Private allowlist staging, per-file SHA-256 manifest, regular-file and size limits, `tar -czf -` | Managed Saturn settings, registered operator scripts, piHPSDR/deskHPSDR `*.props`; credentials/device identity explicitly omitted |
| Source repository backup download | `backup.html` | `GET /backup_source`; legacy alias `GET /backup_full` | `tar -czf -` | Complete active repo root only; not appliance settings |
| Immutable release list/download | `backup.html` | `GET /backup_releases`, `GET /backup_release?commit=...` | Full-commit/direct-child/manifest validation, `tar -czf -` | One installed immutable release; no mutable state |
| Transactional settings restore | `backup.html` | `POST /restore_settings`, `GET /restore_status` | Manifest/schema/hash/path/owner/mode/space validation; durable old/new staging; atomic file replacement; startup rollback | Managed settings and registered operator content; host policy opt-in |
| Transactional source restore | `backup.html` | `POST /restore_source`; compatibility alias `POST /restore_full` | Safe extract, complete generation copy and flush, atomic repo pointer, startup rollback | New generation under state root; prior checkout retained |
| Restore from script-managed directory backups | `backup.html` | `GET /g2_backups`, `POST /g2_restore`, `GET /pihpsdr_backups`, `POST /pihpsdr_restore` | Saturn uses transactional generation switching; piHPSDR retains legacy `rsync -a --delete` | `~/saturn-backup-*`, `~/pihpsdr-backup-*` |
| Transactional appliance update | `update.html` (repo URL + branch/ref + health fields in UI) | `GET/POST /update_policy`, `POST /update_start`, `GET /update_status`, `POST /update_rollback` | `git fetch`, `git worktree add/remove`, `curl` health check, snapshot `tar` | `update_policy.json`, `update_state.json`, `snapshots/`, `repo-staging/` |
| Saturn Go self-update (rebuild + redeploy) with live terminal | `saturngo.html` | `GET/POST /saturngo_policy`, `POST /run`, `GET /run_log` (with `script=update-saturn-go.sh`) | `update-saturn-go.sh`, `cargo build --release`, staged web/script asset sync, `systemd-run`, `systemctl` | `saturngo_update_policy.json`, in-memory run-log buffer |
| Saturn Go last deploy status panel | `saturngo.html` | `GET /saturngo_deploy_status` | Reads status JSON written by `update-saturn-go.sh` / detached helper | `saturngo_deploy_status.json` |
| Advanced `p2app` build/deploy/restart/revert lab (startup profile + panel-mode override + unit-default restore) | Radio Telemetry (`p23test.html`) | `POST /run`, `GET /run_log` (with `script=p23-app-manager.sh`) | `p23-app-manager.sh`, `make`, `systemctl`, systemd drop-in override (`SATURN_FRONT_PANEL_MODE`) | In-memory run-log buffer; `/opt/saturn-go/p23-apps`; `/etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf` |
| `p2app` status panel (parsed override metadata + effective service runtime env) | Radio Telemetry (`p23test.html`) | `GET /p23_status` | Backend filesystem + `systemctl` inspection + override metadata parse; reports effective `SATURN_FRONT_PANEL_MODE` and optional `SATURN_P3_RT_AUDIO_*` values from `p2app.service` | N/A |
| `p2app` workload/performance dashboard + snapshot capture | Radio Telemetry (`p23test.html`) | `GET /p23_perf` | Host process/network/XDMA sampling plus `/dev/shm/saturn_p23_perf_stats.json` app telemetry overlay; browser-side snapshot export of current sample + derived/baseline view + service-runtime subset. Speaker/DUC underrun counters are reported as underrun episodes rather than repeated polls of the same active underflow condition. Snapshot copy falls back when Clipboard API support is missing. | N/A |
| Buffered terminal resume across page switches | `update.html`, `saturngo.html`, `pihpsdr.html`, `deskhpsdr.html`, `index.html` | `GET /run_log` | Offset polling by script + run ID | In-memory per-script run log ring |
| Pre-update snapshots + retention | `update.html` status panel | Part of update workflow | `tar` snapshot + prune logic | `snapshots/` |
| G2/appliance mutual exclusion guard | `update.html` conflict feedback | `POST /run` (`update-G2.py` only), `POST /update_start`, `POST /update_rollback` | In-memory activity acquisition/release | Process-local lock slot |
| Whole-disk imaging compatibility response | `backup.html` explains local-console policy | Legacy image/clone/device/wipe routes return `410 Gone` | Manual local-console scripts remain in repository | No Saturn Go imaging jobs or privileged imaging helpers |
| Repair pack export | `backup.html` | `GET /repair_pack` | `tar -czf -` over key runtime files | Generated manifest in `/tmp` |
| Runtime/config verification | `backup.html` | `GET /verify_system_config` | Filesystem checks + `systemctl is-active` | N/A |
| Password change in UI | `index.html` | `POST /change_password` | `sudo -n saturn-admin-password.sh set` (stdin) | `/etc/nginx/.htpasswd` + `saturn-go.service.d/10-remote-auth.conf` |
| Service liveness/readiness | Overview displays target-aware readiness and the embedded commit; watchdog is not directly in UI | `GET /livez` for watchdog process liveness, `GET /readyz` for dependencies and release identity, compatibility `GET /healthz` alias | `/usr/local/lib/saturn-go/saturn-health-watchdog.sh`, systemd timer/service; deployment broker supplies expected commit | `saturn-go-watchdog.*` units |
| System monitor dashboard | `monitor.html` | `GET /get_system_data`, `GET /network_test`, `POST /kill_process/{pid}` | `/proc`, sysfs, `curl`, `kill` | N/A |
| Tailscale VPN enrollment, status, and Remote Serve controls | `tailscale.html` | `GET /tailscale_status`; `POST /tailscale/install`, `/tailscale/up`, `/tailscale/down`, `/tailscale/logout`, `/tailscale/serve` | Root-owned `saturn-tailscale.sh` helper via `sudo -n` | `tailscaled.service` state and Tailscale Serve configuration |
| FPGA image discovery for flash UI | `fpga.html` | `GET /get_fpga_images` | Directory scan for `.bin` files | `SATURN_FPGA_DIR` or repo paths |
| Legacy backup prompt response hook | `index.html` (modal) | `POST /backup_response` | No-op backend endpoint | N/A |
| Controlled backend shutdown | `index.html` Exit button | `POST /exit`, `GET /shutdown_status` | Graceful admission close; finish-policy drain; process-group TERM/KILL for declared cancel-safe scripts | Durable cancelled job result under `maintenance-jobs/`; systemd `KillMode=mixed` and bounded stop timeout |

## Added/Expanded Areas

Compared to a simple script-runner deployment, the following were added as first-class features:

- Backup and restore page with repo-root awareness
- Dedicated G2 Update page that pairs Update G2 terminal output with Appliance Update controls
- Dedicated Saturn Go self-update page with separate policy and rebuild/redeploy terminal workflow
- Dedicated deskHPSDR Update page for clone/update/build workflow using the active Saturn repo-root helper scripts
- Dedicated FPGA Flash page for safe `load-FPGA` execution
- Transactional appliance update policy, execution, status, and rollback
- Pre-update snapshots and staging lifecycle management
- Shared update-activity lock to prevent overlapping G2/appliance update actions
- Local-console whole-disk imaging policy; browser imaging and cloning disabled
- Repair pack generation and install verification tooling
- CSRF enforcement on all mutating routes
- Watchdog timer/service for automatic restart after failed health checks
- Enhanced monitor endpoint coverage with process actions and throughput metrics
