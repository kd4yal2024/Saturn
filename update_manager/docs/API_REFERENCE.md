# API Reference

## Base Path and Auth

In production, NGINX exposes the backend under `/saturn/`.

- Browser/UI calls use relative paths like `./update_status`, which resolve under `/saturn/`.
- NGINX has a dedicated SSE mapping for `/saturn/run` to backend `/run`.
- Basic Auth is enforced by NGINX for `/saturn/*` routes.

Direct backend routes are shown below without the `/saturn` prefix.

## CSRF Requirements

All `POST` endpoints require header:

- `X-Saturn-CSRF: 1`

Backend also enforces same-host checks when `Origin` or `Referer` is present.

## Page Routes

| Route | Method | CSRF | Description |
|---|---|---|---|
| `/` | `GET` | No | Serve `overview.html` (Saturn Go Overview landing page). |
| `/assets/{*path}` | `GET` | No | Serve shared shell CSS/JS, local fonts, and vendored browser assets from the web root. |
| `/overview` | `GET` | No | Serve `overview.html`. |
| `/overview.html` | `GET` | No | Serve `overview.html`. |
| `/custom` | `GET` | No | Serve `index.html` (Custom Scripts page). |
| `/custom.html` | `GET` | No | Serve `index.html` (Custom Scripts page). |
| `/index` | `GET` | No | Serve `index.html` (Custom Scripts page). |
| `/index.html` | `GET` | No | Serve `index.html` (Custom Scripts page). |
| `/backup` | `GET` | No | Serve `backup.html`. |
| `/backup.html` | `GET` | No | Serve `backup.html`. |
| `/update` | `GET` | No | Serve `update.html` (G2 + Appliance Update page). |
| `/update.html` | `GET` | No | Serve `update.html` (G2 + Appliance Update page). |
| `/saturngo` | `GET` | No | Serve `saturngo.html` (Saturn Go self-update page). |
| `/saturngo.html` | `GET` | No | Serve `saturngo.html` (Saturn Go self-update page). |
| `/saturn-go` | `GET` | No | Serve `saturngo.html` (Saturn Go self-update page). |
| `/saturn-go.html` | `GET` | No | Serve `saturngo.html` (Saturn Go self-update page). |
| `/telemetry` | `GET` | No | Serve `p23test.html` as the Radio Telemetry & Diagnostics page. |
| `/telemetry.html` | `GET` | No | Serve `p23test.html` as the Radio Telemetry & Diagnostics page. |
| `/p23test` | `GET` | No | Legacy alias for the Radio Telemetry & Diagnostics page. |
| `/p23test.html` | `GET` | No | Legacy alias for the Radio Telemetry & Diagnostics page. |
| `/fpga` | `GET` | No | Serve `fpga.html` (FPGA flash terminal/control page). |
| `/fpga.html` | `GET` | No | Serve `fpga.html` (FPGA flash terminal/control page). |
| `/pihpsdr` | `GET` | No | Serve `pihpsdr.html` (piHPSDR update terminal). |
| `/pihpsdr.html` | `GET` | No | Serve `pihpsdr.html` (piHPSDR update terminal). |
| `/deskhpsdr` | `GET` | No | Serve `deskhpsdr.html` (deskHPSDR update terminal). |
| `/deskhpsdr.html` | `GET` | No | Serve `deskhpsdr.html` (deskHPSDR update terminal). |
| `/monitor` | `GET` | No | Serve `monitor.html`. |
| `/monitor.html` | `GET` | No | Serve `monitor.html`. |
| `/tailscale` | `GET` | No | Serve the Tailscale VPN page (`tailscale.html`). |
| `/tailscale.html` | `GET` | No | Serve the Tailscale VPN page (`tailscale.html`). |
| `/remote`, `/remote.html`, `/saturn-remote`, `/saturn-remote.html` | `GET` | No | Redirect to the Saturn Remote TLS page (`/remote-next`) on port 8443 with the default feature query. |
| `/remote-next`, `/remote-next.html` | `GET` | No | Redirect to the next-generation Saturn Remote TLS page on port 8443 with the default feature query. |
| fallback mapped page paths | `GET` | No | Supports `/saturn`, `/saturn/custom`, `/saturn/backup`, `/saturn/update`, `/saturn/saturngo`, `/saturn/telemetry`, `/saturn/fpga`, `/saturn/pihpsdr`, `/saturn/deskhpsdr`, `/saturn/monitor`, etc. |

## Health and Metadata

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/healthz` | `GET` | No | none | `200 OK` |
| `/get_scripts` | `GET` | No | none | `{ "scripts": { "Category": [...] }, "warnings": [] }` |
| `/get_flags` | `GET` | No | `?script=<filename>` | `{ "flags": ["--flag", ...] }` |
| `/get_versions` | `GET` | No | none | `{ "versions": { "script": "version\|unknown" } }` |
| `/custom_scripts` | `GET` | No | none | `{ "scripts": [ { "filename","name","description","flags",... }, ... ] }` (includes seeded default cleanup entries if present) |
| `/custom_scripts` | `POST` | Yes | JSON `{ "filename","name","description","flags","content" }` | `{ "status":"ok", "script": {...} }` |
| `/custom_scripts_delete` | `POST` | Yes | JSON `{ "filename", "delete_file": bool }` | `{ "status":"ok" }` |
| `/get_fpga_images` | `GET` | No | none | `{ "dir", "images", "latest_image", "checked", "warning" }` (searches `SATURN_FPGA_DIR`, active repo-root `FPGA/`, and common repo paths for `.bin` images) |

## Repo Root Management

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/get_repo_root` | `GET` | No | none | `{ "repo_root": "/path" }` |
| `/list_repo_roots` | `GET` | No | none | `{ "active": "/path", "repo_roots": ["/path1", ...] }` |
| `/set_repo_root` | `POST` | Yes | JSON `{ "repo_root": "/path" }` | `{ "status":"ok", "repo_root":"/canonical/path" }` |

Validation rules for `/set_repo_root`:

- target must be a directory
- must contain `.git`
- must contain `update_manager/`

## Appliance Update

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/update_policy` | `GET` | No | none | `{ "policy": { ... } }` |
| `/update_policy` | `POST` | Yes | full policy JSON | `{ "status":"ok", "policy": { ...normalized... } }` |
| `/update_start` | `POST` | Yes | JSON `{ "channel":"stable\|beta\|custom", "custom_ref": "..." }` | `{ "status":"started", "job_id":"upd-..." }` |
| `/update_status` | `GET` | No | none | `{ "job": {...}\|null, "last_update": {...}\|null }` |
| `/update_rollback` | `POST` | Yes | none | `{ "status":"rolled_back", "repo_root":"/path" }` |

Conflict behavior (`409`):

- `POST /update_start`
  - returns conflict when an appliance update is already running
  - also returns conflict when another update activity is active (for example `update-G2.py`)
- `POST /update_rollback`
  - returns conflict when an appliance update is already running
  - also returns conflict when another update activity is active

`update_policy` fields:

- `owner`, `repo`, `remote`
- `channel`, `stable_ref`, `beta_ref`, `custom_ref`
- `auto_snapshot`, `keep_snapshots`
- `healthcheck_url`, `healthcheck_timeout_secs`
- `healthcheck_retries`
- `healthcheck_initial_delay_secs`

Normalization rules:

- invalid owner/repo/remote/ref values are sanitized to safe defaults
- `keep_snapshots` is clamped to `1..50`
- `healthcheck_timeout_secs` is clamped to `2..30`
- `healthcheck_retries` is clamped to `0..5`
- `healthcheck_initial_delay_secs` is clamped to `0..30`
- saved repo URLs must be publicly reachable over HTTPS for anonymous appliance
  updates; update start also verifies the selected ref before fetching directly
  from the policy URL

Current UI behavior notes (`update.html`):

- Appliance form is simplified to GitHub repo URL + branch/ref + healthcheck URL/timeout.
- UI saves policy using `channel=custom` and `custom_ref=<branch/ref>`.
- `Run Update G2` is gated by valid repo URL in Appliance form and persists that policy before script start.

## Saturn Go Self-Update

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/saturngo_policy` | `GET` | No | none | `{ "policy": { ... } }` |
| `/saturngo_policy` | `POST` | Yes | full policy JSON | `{ "status":"ok", "policy": { ...normalized... } }` |
| `/saturngo_deploy_status` | `GET` | No | none | `{ "status":"ok", "deploy": { ... } }` |

Notes:

- `saturngo_policy` uses the same `UpdatePolicy` schema/normalization rules as `update_policy`, but persists to a separate file.
- `/saturngo_deploy_status` returns a synthetic `idle` payload if no status file exists yet.
- The Saturn Go page runs `update-saturn-go.sh` through `POST /run` and uses `/run_log` for resume across page refresh/navigation.

## Saturn Remote, Bridge, and Profiles

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/remote_settings` | `GET` | No | none | `{ "status":"ok", "settings": { ... } }` |
| `/remote_settings` | `POST` | Yes | Remote settings JSON | `{ "status":"ok", "settings": { ... } }` |
| `/remote_profiles` | `GET` | No | none | `{ "status":"ok", "profiles": { "startupProfile": null, "profiles": { ... } } }` |
| `/remote_profiles/save` | `POST` | Yes | `{ "name", "settings", "makeStartup" }` | Saved profile and complete profile catalog |
| `/remote_profiles/delete` | `POST` | Yes | `{ "name" }` | Updated profile catalog |
| `/remote_profiles/startup` | `POST` | Yes | `{ "name": "<profile>" }` or `{ "name": null }` | Updated profile catalog; `null` clears the startup profile |
| `/bridge_diag` | `GET` | No | none | Saturn Bridge service state plus recent parsed diagnostic/status journal entries |
| `/saturn/bridge_diag` | `GET` | No | none | Compatibility alias for `/bridge_diag` |
| `/tci` | `GET` upgrade | No | WebSocket upgrade | Proxied TCI/WebSocket session to `saturn-bridge` |

Remote settings and profiles use camelCase JSON fields. Profile names are
limited to 64 ASCII letters, digits, spaces, hyphens, underscores, or periods.

## Tailscale VPN

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/tailscale_status` | `GET` | No | none | Installation/service state, local identity, tailnet addresses, Serve state, and calculated Remote URL |
| `/tailscale/install` | `POST` | Yes | none | SSE helper output |
| `/tailscale/up` | `POST` | Yes | Optional `{ "auth_key", "hostname", "ssh", "accept_routes", "accept_dns", "reset" }` | SSE helper output |
| `/tailscale/down` | `POST` | Yes | none | SSE helper output |
| `/tailscale/logout` | `POST` | Yes | none | SSE helper output |
| `/tailscale/serve` | `POST` | Yes | `{ "enable": true, "port": 443 }`; `port` is optional | SSE helper output |

Tailscale mutation endpoints run the root-owned
`/usr/local/lib/saturn-go/scripts/saturn-tailscale.sh` helper through
passwordless `sudo -n`. SSE responses disable proxy buffering.

## Radio Telemetry & Advanced Service Controls

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/p23_status` | `GET` | No | none | `{ "status":"ok", "p23": { service, sources, deployed, override, repo_root } }` |
| `/p23_perf` | `GET` | No | none | `{ "status":"ok", "perf": { collected_at_ms, system, process, network, xdma, workload, app_telemetry } }` |
| `/p23_adc_telemetry` | `POST` | Yes | `{ "enabled": bool }` | ADC peak telemetry control/snapshot paths and effective enabled state |

Notes:

- `p23_status` is used by the `/telemetry` page status panel.
- It reports:
  - `p2app.service` active/enabled/main PID (and running executable path when available)
  - source/deployed binary details in the active repo root and `/opt/saturn-go/p23-apps`
  - systemd drop-in override file state (`/etc/systemd/system/p2app.service.d/10-saturn-p23-switch.conf`)
- `p23_status.p23.override` also includes parsed Saturn test-lab metadata when present:
  - `panel_mode` from `Environment=SATURN_FRONT_PANEL_MODE=...`
  - `saturn_meta` parsed from the generated comment (`# saturn-p23 mode=... panel=...`)
- `p23_perf` is used by the `/telemetry` performance dashboard.
- It reports:
  - host/process/network/XDMA snapshots used for baseline deltas
  - `workload` metadata derived from the deployed `current` symlink and the `p2app.service` drop-in (`selected_app`, startup mode, panel mode, workload key)
  - `app_telemetry` parsed from `/dev/shm/saturn_p23_perf_stats.json` when the running `p2app`-compatible app exports live counters
- `app_telemetry.current` includes:
  - runtime flags and feature flags
  - port/DDC/wideband routing shape
  - FIFO/ADC gauges
  - cumulative counters for high-priority, mic, DDC, wideband, DUC, and speaker packet/DMA/error activity

## Full Backup / Restore

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/backup_full` | `GET` | No | none | streaming `application/gzip` attachment |
| `/restore_full` | `POST` | Yes | `multipart/form-data` with `file`; optional `confirm=RESTORE`; optional boolean query `dry_run` | JSON status |
| `/g2_backups` | `GET` | No | none | `{ "home":"/home/...", "backups":[{ "name","path","files","dirs","bytes","modified_epoch" }, ...] }` |
| `/g2_restore` | `POST` | Yes | JSON `{ "backup_name":"saturn-backup-...", "dry_run":bool, "confirm":"RESTORE" }` | dry-run stats or `{ "status":"ok", ... }` |
| `/pihpsdr_backups` | `GET` | No | none | `{ "home":"/home/...", "backups":[{ "name","path","files","dirs","bytes","modified_epoch" }, ...] }` |
| `/pihpsdr_restore` | `POST` | Yes | JSON `{ "backup_name":"pihpsdr-backup-...", "dry_run":bool, "confirm":"RESTORE" }` | dry-run stats or `{ "status":"ok", ... }` |

Restore responses:

- dry-run: `{ "status":"ok", "dry_run":true, "files", "dirs", "bytes", ... }`
- apply: `{ "status":"ok" }`

Restore safety checks:

- upload size limit from `SATURN_RESTORE_MAX_UPLOAD_BYTES`
- tar entries may not be absolute or contain `..`
- symlink targets may not be absolute or contain `..`
- the backend rejects archives whose uncompressed size exceeds the configured
  expansion-ratio guard
- the backend rejects archives that would exceed current `/tmp` free space
- tar path traversal guard (reject absolute and `..` paths)
- must extract to a single top-level directory
- extracted top-level directory must pass Saturn repo-root validation (`.git` + `update_manager/`)
- uses `rsync -a --delete` into active repo root
- non-dry-run acquires update-activity lock and returns `409` if another update action is active

`/g2_restore` safety checks:

- backup name must match `saturn-backup-*` and cannot include path traversal
- selected backup must resolve under backend `$HOME`
- selected backup and target repo root must both pass Saturn repo-root validation
- non-dry-run requires `confirm=RESTORE`
- non-dry-run acquires update-activity lock and returns `409` if conflicting update action is active

`/pihpsdr_restore` safety checks:

- backup name must match `pihpsdr-backup-*` and cannot include path traversal
- selected backup must resolve under backend `$HOME`
- selected backup and target piHPSDR root must both be valid git checkouts
- non-dry-run requires `confirm=RESTORE`
- non-dry-run acquires update-activity lock and returns `409` if conflicting update action is active

## Script Execution and Legacy Hooks

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/run` | `POST` | Yes | `multipart/form-data` with `script` + repeated `flags` | SSE stream (`text/event-stream`) |
| `/run_log` | `GET` | No | `?script=<filename>&from=<offset>&limit=<n>` | `{ "script","run_id","status","running","started_at","finished_at","from","next_from","total_lines","lines":[...] }` |
| `/backup_response` | `POST` | Yes | form payload from legacy prompt | `204 No Content` |
| `/exit` | `POST` | Yes | none | `{ "status":"shutting down" }` |

SSE output is streamed line-by-line, including stderr lines prefixed with `ERR:`.

Run-log buffering behavior:

- `/run` and `/run_log` share in-memory per-script run state.
- `run_log` supports resume via `from` offset and returns `next_from`.
- `run_log` returns `status` (`idle|running|done|error`) and current `run_id`.
- `run_log` max fetch `limit` is clamped by backend.

Update-activity behavior for `/run`:

- For `update-G2.py`/`update-G2.sh` and `update-saturn-go.sh`, backend acquires the shared update-activity lock.
- If appliance update/rollback (or another conflicting update action) is active, route returns `409` with `{ "message": "..." }`.
- `/run` rejects Python scripts when the resolved script path is under active `SATURN_REPO_ROOT`.
- Child process environment includes:
  - `SATURN_REPO_ROOT`
  - `SATURN_DIR`
  - `SATURN_ACTIVE_REPO_ROOT`
- `update-saturn-go.sh` runs also include:
  - `SATURN_SATURNGO_POLICY_OWNER`
  - `SATURN_SATURNGO_POLICY_REPO`
  - `SATURN_SATURNGO_POLICY_REMOTE`
  - `SATURN_SATURNGO_POLICY_REF`
  - `SATURN_SATURNGO_POLICY_URL`
  - `SATURN_SATURNGO_DEPLOY_STATUS_FILE`
- `p23-app-manager.sh` (advanced Radio Telemetry service-lab workflow) uses the active repo-root env (`SATURN_ACTIVE_REPO_ROOT`) and runs privileged deploy/restart actions via `sudo -n` when not root.
- `update-deskhpsdr.py` uses the active repo-root env to find `scripts/deskhpsdr-test-build-on-current-image.sh`, clones/pulls `~/github/deskhpsdr` unless `--skip-git` is set, then runs the helper-script build flow with the selected flags.
- `scripts/deskhpsdr-test-build-on-current-image.sh` applies `scripts/patches/deskhpsdr-libgpiod-v2.patch` only for older checkouts that still include the legacy `src/gpio.c` path. Current upstream builds skip that obsolete patch and use `SATURN=ON` for the native G2/XDMA path.
- Python child runs also include:
  - `PYTHONDONTWRITEBYTECODE=1`
  - `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`

## Credentials

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/change_password` | `POST` | Yes | `application/x-www-form-urlencoded`, `new_password=<value>` | `{ "status":"success" }` or `{ "status":"error", "message":"..." }` |

Behavior:

- requires exactly 5 characters for newly set passwords and rejects control characters
- runs `sudo -n saturn-admin-password.sh set` with the password on stdin
- the helper updates `/etc/nginx/.htpasswd` and the TLS auth drop-in
  together (all-or-nothing with rollback), then schedules a deferred
  `saturn-go` restart (~2s) so the TLS listener picks up the change
- success response includes a `message` telling the user remote sessions
  reconnect in a few seconds

## Pi Image Workflow

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/pi_image_start` | `POST` | Yes | JSON `{ "shrink":bool, "compress":bool, "out_dir":"/path" }` | `{ "job_id":"piimg-..." }` |
| `/pi_image_status` | `GET` | No | `?job_id=<id>` | job JSON (`running\|done\|error\|cancelled`) |
| `/pi_image_cancel` | `POST` | Yes | `?job_id=<id>` | `{ "status":"cancelled" }` |
| `/pi_image_download` | `GET` | No | `?job_id=<id>` | binary file download |

`/pi_image_download` schedules best-effort cleanup of the image file after download starts.

## Clone SD to Removable Device

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/pi_devices` | `GET` | No | none | `{ "devices": [{ "name", "path", "size_bytes", "model" }, ...] }` |
| `/pi_wipe_target` | `POST` | Yes | JSON `{ "target":"/dev/sdX" }` | `{ "status":"ok", "message":"...", "log":[...] }` |
| `/pi_clone_start` | `POST` | Yes | JSON `{ "target":"/dev/sdX" }` | `{ "job_id":"piclone-..." }` |
| `/pi_clone_status` | `GET` | No | `?job_id=<id>` | clone job JSON |
| `/pi_clone_cancel` | `POST` | Yes | `?job_id=<id>` | `{ "status":"cancelled" }` |

Notes:

- `/pi_devices` enumerates supported clone targets from `/sys/block`, including USB-attached disks/readers that may report `removable=0`.
- `/pi_wipe_target` and `/pi_clone_start` reject non-`/dev/*` targets, `/dev/mmcblk0`, and unsupported internal/virtual devices.
- `/pi_wipe_target` is a quick pre-clone metadata wipe (not a full secure erase): best-effort partition unmount, `wipefs`, optional `sgdisk --zap-all`, and zeroing of the first/last 16 MiB.

## Monitor and Diagnostics

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/get_system_data` | `GET` | No | optional query: `proc_sort`, `proc_order`, `proc_user`, `proc_regex`, `proc_top`, `proc_page`, `proc_page_size`, `rate_scope=overview` | CPU/memory/swap/disk/network/load/uptime/temp/processes JSON |
| `/network_test` | `GET` | No | none | `{ "tx_bps", "rx_bps", "seconds" }` or `{ "error": "..." }` |
| `/kill_process/{pid}` | `POST` | Yes | optional `?sig=term` or `?sig=kill` | `{ "message":"OK" }` or error JSON |

`rate_scope=overview` isolates Overview disk/network delta sampling from the
default Monitor baseline; arbitrary scope names are ignored.

`/kill_process/{pid}` safeguards:

- rejects `pid <= 0`
- rejects protected processes (PID <= 2 or root-owned)

## Repair and Verification

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/repair_pack` | `GET` | No | none | streaming `tar.gz` repair bundle with manifest |
| `/verify_system_config` | `GET` | No | none | `{ "ok":bool, "missing":[], "warnings":[], "checks":[] }` |

## Common Error Format

Most error responses use:

```json
{ "message": "..." }
```

Some endpoints also return route-specific payloads such as `status` or `error` fields.
