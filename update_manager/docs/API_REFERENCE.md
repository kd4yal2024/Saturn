# API Reference

## Base Path and Auth

In production, NGINX exposes the backend under `/saturn/`.

- Browser/UI calls use relative paths like `./update_status`, which resolve under `/saturn/`.
- NGINX has a dedicated SSE mapping for `/saturn/run` to backend `/run`.
- Basic Auth is enforced by NGINX for `/saturn/*` routes.

Direct backend routes are shown below without the `/saturn` prefix.

Ordinary JSON/configuration requests and custom-script multipart arguments are
limited to 64 KiB at both the reverse proxy and backend. Restore archive routes
have a separate configurable streaming allowance; oversized requests receive
HTTP 413 before route processing.

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
| `/livez` | `GET` | No | none | `200` JSON with process status and embedded full Git commit |
| `/readyz` | `GET` | No | optional `?expected_commit=<40-hex-sha>` | `200` structured component results when required checks pass; `503` with the same structure when not ready |
| `/healthz` | `GET` | No | none | Compatibility alias for `/livez`; deprecated after the 2026 release transition |
| `/get_scripts` | `GET` | No | none | `{ "scripts": { "Category": [...] }, "warnings": [] }` |
| `/get_flags` | `GET` | No | `?script=<filename>` | `{ "flags": ["--flag", ...] }` |
| `/get_versions` | `GET` | No | none | `{ "versions": { "script": "version\|unknown" } }` |
| `/custom_scripts` | `GET` | No | none | `{ "scripts": [ { "filename","name","description","flags",... }, ... ] }` (includes seeded default cleanup entries if present) |
| `/custom_scripts` | `POST` | Yes | JSON `{ "filename","name","description","flags","content" }` | `{ "status":"ok", "script": {...} }` |
| `/custom_scripts_delete` | `POST` | Yes | JSON `{ "filename", "delete_file": bool }` | `{ "status":"ok" }` |
| `/get_fpga_images` | `GET` | No | none | `{ "dir", "images", "latest_image", "checked", "warning" }` (searches `SATURN_FPGA_DIR`, active repo-root `FPGA/`, and common repo paths for `.bin` images) |

`/readyz` requires the running binary's embedded full Git commit to match the
expected commit. The installer and root deployment broker always supply the
staged commit explicitly. State writability, configuration parsing, free disk
space are required checks. The Saturn Bridge listener is reported but the
installer marks it optional because P2-only startup deliberately leaves that
operator-controlled service stopped. P2 and XDMA state are also reported but
do not block an application deployment unless `SATURN_READY_REQUIRE_P2` is
enabled.

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
| `/remote_metrics` | `GET` | No | none | Authenticated logical-client/connection counts, configured limit, rejection counters, and high-water marks |
| `/bridge_diag` | `GET` | No | none | Saturn Bridge service state plus recent parsed diagnostic/status journal entries |
| `/saturn/bridge_diag` | `GET` | No | none | Compatibility alias for `/bridge_diag` |
| `/tci` | `GET` upgrade | No | WebSocket upgrade | Proxied TCI/WebSocket session to `saturn-bridge` |
| `/saturn/control?session=<id>` | `GET` upgrade | No | Authenticated split-control WebSocket upgrade | Control lane for one logical remote session |
| `/saturn/media?session=<id>` | `GET` upgrade | No | Authenticated split-media WebSocket upgrade | Media lane paired with the control lane of the same logical session |

Remote settings and profiles use camelCase JSON fields. Profile names are
limited to 64 ASCII letters, digits, spaces, hyphens, underscores, or periods.
These routes are served by the authenticated TLS listener on port 8443. Saturn
Remote permits four logical clients globally; matching split control/media
lanes consume one client slot. A fifth client receives HTTP 429 and
`Retry-After: 5`, a duplicate lane receives HTTP 409, and a split upgrade
without a valid session id receives HTTP 400.

`/bridge_diag` exposes the latest numeric bridge diagnostics under
`bridge.journal.latest_diag.fields`, including connection/queue limits and current
depths, rejected connections, coalesced/replaced/dropped traffic, enqueue-to-
write latency, and outbound/TCP high-water marks.

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
| `/performance_benchmarks` | `GET` | No | none | Persistent bounded benchmark history |
| `/performance_benchmarks` | `POST` | Yes | Validated benchmark-run document | Updated persistent benchmark history |
| `/performance_benchmarks/compare` | `POST` | Yes | `{ "baseline_id": string, "candidate_id": string }` | Workload compatibility, verdict, and per-metric checks |
| `/performance_benchmarks/delete` | `POST` | Yes | `{ "id": string }` | Updated history after deleting one run |
| `/radio_backend` | `GET` | No | none | Selected/runtime backend, persisted and operational status, P2/bridge service state, and mutual-exclusion result |
| `/radio_backend` | `POST` | Yes | `{ "backend":"p2|xdma|bridge", "action":"switch|start|stop" }`; `action` defaults to `switch`; `bridge` accepts only `start` or `stop` | Transaction result and refreshed backend/service status |
| `/appliance_power` | `POST` | Yes | `{ "action":"poweroff", "confirmation":"POWER OFF" }` | HTTP 202 after the selected radio backend is stopped and a delayed systemd-owned G2 poweroff is scheduled |

Notes:

- `p23_status` is used by the `/telemetry` page status panel.
- The overview page uses `/radio_backend` for exclusive P2/XDMA start and stop
  plus independent Saturn Bridge control while P2 owns the radio. In direct
  XDMA mode the bridge control uses the complete backend transaction because
  Saturn Bridge is the FPGA owner. P2 selection enables only P2app at boot;
  starting Bridge in P2 mode is an on-demand action that does not enable it for
  the next boot. Starting either radio backend is a transactional ownership
  switch; stopping preserves the selection so Start resumes the same backend.
- `/appliance_power` refuses while maintenance operations are active, requires
  exact confirmation text, and does not schedule poweroff unless receive-safe
  radio shutdown succeeds first.
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

## Settings, Source, Release Backup, and Transactional Restore

Settings backup accepts only regular allowlisted files, caps each file at 16
MiB and the selected payload at 128 MiB, and emits a manifest with a digest for
every payload file. Registered scripts with version `custom-default` are
regenerable and their content is omitted. Other registered scripts are treated
as operator-authored; a missing or redirected file fails backup creation.

Settings and source imports are transactional. Installed-release archive import
and activation remain separate local release-manager operations.

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/backup_settings` | `GET` | No | none | schema-v1 settings `application/gzip` attachment; private operator data, not a support bundle |
| `/backup_source` | `GET` | No | none | complete active source-repository `application/gzip` attachment |
| `/backup_full` | `GET` | No | none | compatibility alias for `/backup_source`; not a complete appliance backup |
| `/backup_releases` | `GET` | No | none | `{ "format":"saturn-installed-release-list", "active_commit", "releases":[{"commit","active","manifest_present"}] }` |
| `/backup_release` | `GET` | No | query `commit=<full-40-character-hex-commit>`; omitted commit selects active immutable release when one exists | selected manifest-bearing immutable-release `application/gzip` attachment |
| `/restore_settings` | `POST` | Yes | settings archive as `multipart/form-data` field `file`; apply requires `confirm=RESTORE`; query `dry_run=1`; optional explicit `include_host_policy=1` | validation summary or committed transaction ID |
| `/restore_source` | `POST` | Yes | source archive as `multipart/form-data` field `file`; apply requires `confirm=RESTORE`; query `dry_run=1` | validation summary or old/new repository roots plus transaction ID |
| `/restore_full` | `POST` | Yes | compatibility alias for `/restore_source` | same as `/restore_source` |
| `/restore_status` | `GET` | No | none | latest durable settings/source restore transaction records |
| `/g2_backups` | `GET` | No | none | `{ "home":"/home/...", "backups":[{ "name","path","files","dirs","bytes","modified_epoch" }, ...] }` |
| `/g2_restore` | `POST` | Yes | JSON `{ "backup_name":"saturn-backup-...", "dry_run":bool, "confirm":"RESTORE" }` | dry-run stats or `{ "status":"ok", ... }` |
| `/pihpsdr_backups` | `GET` | No | none | `{ "home":"/home/...", "backups":[{ "name","path","files","dirs","bytes","modified_epoch" }, ...] }` |
| `/pihpsdr_restore` | `POST` | Yes | JSON `{ "backup_name":"pihpsdr-backup-...", "dry_run":bool, "confirm":"RESTORE" }` | dry-run stats or `{ "status":"ok", ... }` |

Restore responses:

- settings dry-run: `{ "status":"ok", "dry_run":true, "files", "bytes", "skipped_host_policy", ... }`
- source dry-run: `{ "status":"ok", "dry_run":true, "source_root", "previous_repo_root", "bytes" }`
- apply: `{ "status":"ok", "transaction_id", ... }`

Source-restore safety checks:

- upload size limit from `SATURN_RESTORE_MAX_UPLOAD_BYTES`
- tar entries may not be absolute or contain `..`
- symlink targets may not be absolute or contain `..`
- the backend rejects archives whose uncompressed size exceeds the configured
  expansion-ratio guard
- the backend rejects uploads or extracted archives that would consume the
  configured readiness disk reserve
- upload and extraction staging use `$SATURN_STATE_DIR/restore-tmp`, keeping
  large archives off the appliance's small `/tmp` tmpfs
- tar path traversal guard (reject absolute and `..` paths)
- must extract to a single top-level directory
- extracted top-level directory must pass Saturn repo-root validation (`.git` + `update_manager/`)
- validates source ownership and rejects unsafe symlinks, special files, and
  special permission bits
- requires free space for the generation plus a 512 MiB reserve
- flushes a unique complete generation and atomically switches the repository
  pointer; the previous checkout is retained
- non-dry-run acquires update-activity lock and returns `409` if another update action is active

Settings restore additionally verifies the manifest/schema, exact declared
file set, per-file hashes and size limits, semantic JSON/registry relationships,
live owner and destination type, and capacity for old/new transaction data.
Output modes are fixed by destination class. Host-specific repository/update
policy is excluded unless explicitly requested. Startup recovery rolls back any
transaction not durably marked committed.

`/g2_restore` safety checks:

- backup name must match `saturn-backup-*` and cannot include path traversal
- selected backup must resolve under backend `$HOME`
- selected backup and active repo root must both pass Saturn repo-root validation
- non-dry-run requires `confirm=RESTORE`
- Saturn backups use the transactional generation switch; piHPSDR retains its
  historical in-place source restore
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
| `/run` | `POST` | Yes | `multipart/form-data` with `script`, repeated `flags`, and optional `deadline_seconds` | SSE stream (`text/event-stream`) |
| `/run_log` | `GET` | No | `?script=<filename>&from=<offset>&limit=<n>` | `{ "script","run_id","status","running","started_at","finished_at","from","next_from","total_lines","retained_bytes","truncated","lines":[...] }` |
| `/backup_response` | `POST` | Yes | form payload from legacy prompt | `204 No Content` |
| `/exit` | `POST` | Yes | none | `{ "status":"shutting down" }` |

SSE output is streamed line-by-line, including stderr lines prefixed with `ERR:`.
The live channel holds at most 128 events and emits an `output backpressure`
notice when events are omitted because a client cannot keep up.

Run-log buffering behavior:

- `/run` and `/run_log` share in-memory per-script run state.
- `run_log` supports resume via `from` offset and returns `next_from`.
- `run_log` returns `status` (`idle|running|done|error|cancelled|timed_out`) and
  current `run_id`.
- `run_log` max fetch `limit` is clamped by backend.
- In-memory resume output is capped at 1 MiB and 5,000 lines. Durable job output
  is capped at 4 MiB and 5,000 lines and includes an explicit truncation marker.
- Routine scripts default to a 30-minute deadline and update scripts to four
  hours. Optional `deadline_seconds` is clamped to a six-hour maximum.
- Deadline expiry terminates the complete maintenance process group and records
  `timed_out`/`timeout` in the durable maintenance job record.

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
- `update-deskhpsdr.py` uses the active repo-root env to find `scripts/deskhpsdr-test-build-on-current-image.sh`, clones/pulls `~/github/deskhpsdr` unless `--skip-git` is set, then runs the helper-script build flow with the selected flags. `--legacy-gpio` instead selects the pinned upstream `2.6.84` tag and requires the Saturn libgpiod-v2 V1 controller patch.
- `scripts/deskhpsdr-test-build-on-current-image.sh` always applies the
  idempotent `scripts/patches/deskhpsdr-active-receiver-init.patch` startup
  guard. It applies `deskhpsdr-libgpiod-v2.patch` only to older checkouts that
  still include `src/gpio.c`, builds the native G2/XDMA path with `SATURN=ON`,
  and installs a direct application launcher after a successful build.
- Python child runs also include:
  - `PYTHONDONTWRITEBYTECODE=1`
  - `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`

## Credentials

| Route | Method | CSRF | Request | Success Response |
|---|---|---|---|---|
| `/change_password` | `POST` | Yes | `application/x-www-form-urlencoded`, `new_password=<value>` | `{ "status":"success" }` or `{ "status":"error", "message":"..." }` |

Behavior:

- requires at least 5 characters for newly set passwords and rejects control characters
- runs `sudo -n saturn-admin-password.sh set` with the password on stdin
- the helper updates `/etc/nginx/.htpasswd` and the TLS auth drop-in
  together (all-or-nothing with rollback), then schedules a deferred
  `saturn-go` restart (~2s) so the TLS listener picks up the change
- success response tells the user that the restart invalidates every
  remembered-device login and remote clients must use the new password

## Whole-Disk Imaging Compatibility Routes

Whole-disk image creation, download, cloning, device enumeration, and target
wiping are intentionally disabled in Saturn Go. Legacy `/pi_image_*`,
`/pi_clone_*`, `/pi_devices`, and `/pi_wipe_target` routes return HTTP `410 Gone`
with a JSON message directing the operator to the local-console procedure in
the Operations Runbook. Repository/settings backup and restore routes are not
affected.

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
