# Script Catalog

This file documents the scripts deployed into `/opt/saturn-go/scripts` and related helper scripts currently sourced from `update_manager/scripts` plus selected repo-root `scripts/` helpers.

## Config-Driven Scripts (Exposed by API)

Defined in `config.json` and surfaced by `/get_scripts`.

| Script | Version | Purpose | Typical Flags |
|---|---|---|---|
| `update-G2.py` | `2.15` | Update Saturn repository and related components with backup options and privilege-aware behavior, including resilient per-user log handling. | `--skip-git`, `-y`, `-n`, `--dry-run`, `--verbose` |
| `update-pihpsdr.py` | `1.12` | Update/build piHPSDR with dependency preflight and current WDSP 2.00 Linux compatibility. | `--skip-git`, `-y`, `-n`, `--no-gpio`, `--dry-run`, `--verbose` |
| `update-deskhpsdr.py` | `1.2` | Clone/update/build deskHPSDR with privileged prerequisite preflight, repo-local WDSP library preparation, resumable no-clean builds, and conditional legacy libgpiod patch handling. | `--skip-git`, `-y`, `-n`, `--no-install-deps`, `--no-clean`, `--no-desktop-shortcut`, `--dry-run`, `--verbose` |
| `log_cleaner.sh` | `3.00` | Find and optionally delete `*.log` files under home directory. | `--delete-all`, `--no-recursive`, `--dry-run` |
| `restore-backup.sh` | `3.10` | Restore Saturn or piHPSDR from backup directories with list/latest/explicit selection support. | `--saturn`, `--pihpsdr`, `--latest`, `--list`, `--backup-dir`, `--backup-name`, `--dry-run`, `--verbose`, `--json` |

UI usage notes:

- `index.html` (Custom Scripts page) intentionally excludes `update-G2.py` from the dropdown.
- `update.html` (G2 Update page) is the dedicated UI for running `update-G2.py` with live SSE terminal output.
- `saturngo.html` is the dedicated UI for running `update-saturn-go.sh` (separate Saturn Go repo policy + self-redeploy workflow).
- `saturngo.html` also exposes an `XDMA Doctor` button that runs `xdma-doctor.sh` through the same `/run` terminal path and reports PCIe/XDMA/module/service state plus whether DKMS or the legacy recovery hook owns kernel updates.
- `saturngo.html` also exposes a `Stage Running Kernel` button that runs `xdma-stage-current.sh` through the same `/run` terminal path.
- `p23test.html` is presented as Radio Telemetry & Diagnostics in the main navigation. It combines live radio/performance data with advanced converged `p2app` build/deploy/restart controls using `p23-app-manager.sh`.
- `index.html` (Custom Scripts page) intentionally excludes `update-pihpsdr.py` from the dropdown.
- `pihpsdr.html` is the dedicated UI for running `update-pihpsdr.py` with live SSE terminal output.
- `index.html` (Custom Scripts page) intentionally excludes `update-deskhpsdr.py` from the dropdown.
- `deskhpsdr.html` is the dedicated UI for running `update-deskhpsdr.py` with live SSE terminal output.
- `fpga.html` is the dedicated UI for running `flash_fpga.sh` with confirmation and FPGA image discovery.
- `restore-backup.sh` is intentionally excluded from the main dropdown; Backup / Restore page provides dedicated restore controls for script-created backup directories.
- `index.html` is used as the browser-managed Custom Scripts page (`/custom`), backed by `custom_scripts.json`.
- `/run` executes scripts from `/opt/saturn-go/scripts`.
- `/run_log` provides buffered per-script run output by offset (used for resume after page switches).

## Backend-Seeded Default Custom Scripts

These are created/registered by backend startup when missing and appear in `/custom_scripts`.

| Script | Purpose | Typical Flags |
|---|---|---|
| `cleanup-saturn-logs.sh` | Remove Saturn update log files in `~/saturn-logs` (retention-oriented by default). | `--all`, `--older-7`, `--dry-run`, `--verbose` |
| `cleanup-saturn-backups.sh` | Prune `~/saturn-backup-*` and `~/pihpsdr-backup-*` directories (keeps newest 2 by default). | `--saturn-only`, `--pihpsdr-only`, `--delete-all`, `--dry-run`, `--verbose` |
| `fix-LED-power-button.sh` | Install the BCM15 front-panel LED handler for red-on-boot and white-on-shutdown behavior; script self-elevates with `sudo -n` when launched from the web manager. | none |
| `setup-eth-fallback.sh` | Configure NetworkManager DHCP-to-APIPA fallback for direct Ethernet links; script self-elevates with `sudo -n` when launched from the web manager. | none |

## Local-Console Disk Maintenance

These scripts remain available in the repository for an operator working at a
local terminal. Saturn Go does not install them as privileged web helpers and
does not expose disk imaging, cloning, or target wiping through API routes.

| Script | Trigger Route(s) | Purpose |
|---|---|---|
| `make_pi_image.sh` | Manual local-console invocation | Create an SD image from `/dev/mmcblk0`, with optional shrink/compression. |
| `clone_pi_to_device.sh` | Manual local-console invocation | Clone `/dev/mmcblk0` to an explicitly selected removable target device. |

## Additional Utilities in Repo

Not all utilities are directly wired into current UI buttons, but are included in the managed script set.

| Script | Purpose | Key Flags |
|---|---|---|
| `flash_fpga.sh` | Web-facing wrapper that hands FPGA flashing to the root-owned `saturn-flash-fpga.sh` helper, preserving confirmation guard and `load-FPGA` behavior without granting sudo to the writable runtime script path. | `--image`, `--latest`, `--primary`, `--fallback`, `--verify`, `--no-verify`, `--confirm`, `--dry-run` |
| `g2-version-info.sh` | Read-only helper for G2 Update page that reports active binary family, live runtime app identity/version from `/p23_perf` when available, and current or retained `p2app.service` startup banner lines for FPGA firmware/date-code/temp when those lines exist in the journal. | none |
| `install-shutdown-waiter-service.sh` | Install or refresh `saturn-shutdown-waiter.service` and its default config. Used by Update G2. | `--enabled-default <mode>`, `--saturn-user <user>` |
| `shutdown-waiter.sh` | Runtime shutdown-waiter payload installed by `install-shutdown-waiter-service.sh`. | none |
| `saturn-release-build.sh` | Build and validate one complete inactive application release from an exact clean commit; never activates or restarts services. | `--output-root DIR`, `--dry-run` |
| `saturn-release-install-root.sh` | Root-owned broker that revalidates and installs a completed bundle into `/opt/saturn/releases/<full-commit>` without changing the active release or restarting services. | `--validate BUNDLE`, `BUNDLE` |
| `saturn-release-activate-root.sh` | Root-owned REM-0203/0204/0205 broker that validates an installed full commit and its state contract, snapshots persistent state before migration, snapshots the prior pointer/service configuration, atomically switches `/opt/saturn/current`, restarts affected services in dependency order, and requires exact-commit readiness. Failures restore state and the prior release; incomplete recovery is persisted as `rollback_failed`. Production activation remains disabled pending an approved appliance test. | `--validate FULL_COMMIT`, `--approve-one-way-migration FULL_COMMIT`, `FULL_COMMIT` (disabled by default) |
| `saturn-release-manifest.py` | Create or validate the v2 application release manifest, state contract, and exact SHA-256 inventory; installed v1 manifests remain supported as legacy releases. | `create ...`, `validate ...`, `state-contract ...` |
| `saturn-state-compatibility.py` | Root-owned REM-0205 helper that preflights state compatibility, creates checksummed pre-migration backups, writes the schema marker, and restores a validated backup during rollback. It is not exposed through Saturn Go sudoers. | `preflight ...`, `migrate ...`, `restore ...` |
| `saturn-maintenance-lock.py` | Trusted REM-0402 host-level resource broker. It acquires fixed-order locks and remains the lock owner while a maintenance child runs, preserving exclusivity across Saturn Go restarts. | `probe`, `hold`, `run` with fixed resource classes |
| `update-saturn-go.sh` | Rebuild/redeploy `saturn-go` from the active Saturn repo root (used by `saturngo.html`). | `--verbose`, `--dry-run`, `--skip-git`, `--skip-build`, `--skip-deploy` |
| `update-p2app.sh` | Run the Protocol 2 tests, build the active checkout, and hand production deployment to the trusted root broker with verification and rollback. | `SATURN_P2_BUILD_JOBS`, `SATURN_P2_SKIP_TESTS` |
| `saturn-p2-deploy.sh` | Root-owned no-argument P2 production deployment broker; source/runtime paths come from root-owned `/etc/default/saturn-p2-deploy`. | none |
| `seal-saturn-image.sh` | Remove cloned machine/SSH/Tailscale/auth identity and arm first-boot personalization before golden-image capture. | `--confirm SEAL`, `--no-poweroff`, `--keep-build-cache` |
| `saturn-first-boot.sh` | One-shot golden-image clone personalization that generates a hostname (unless customized), SSH keys, and a unique five-character Saturn Go login, preserves an image-customized Linux password (or unlocks a still-locked account), and runs before Saturn Go creates a new Remote TLS identity. | none |
| `xdma-doctor.sh` | Run the classified Saturn XDMA doctor through the privileged helper path (used by `saturngo.html`); helper emits an advisory when XDMA is loaded but not staged for the running kernel. | passthrough (`--json`, `--stage-only`, `--skip-service-check` if needed) |
| `xdma-stage-current.sh` | Pre-stage XDMA for the running kernel through a narrow privileged helper without restarting `p2app.service`; helper re-execs through a transient systemd unit when launched from Saturn Go so `/lib/modules` stays writable. | none |
| `p23-app-manager.sh` | Advanced helper to build/deploy/restart/revert the converged `p2app` service path (used by the Radio Telemetry page), with startup-profile and front-panel-mode override support. Legacy `p3` arguments are accepted but mapped to the same converged binary. | `--status`, `--build [p2\|p3]`, `--deploy [p2\|p3]`, `--restart [p2\|p3]`, `--switch [p2\|p3]`, `--revert`, `--mode panel\|headless\|panel-debug`, `--panel auto\|g2\|g2v2\|prefer-g2\|prefer-g2v2\|off`, `--dry-run`, `--verbose`, `--no-restart`, `--no-clean` |
| `qemu_pi_boot.sh` | Boot Raspberry Pi image in QEMU by extracting kernel/DTB and launching `qemu-system-aarch64`. | `--img`, `--work-dir`, `--memory`, `--cpus`, `--machine`, `--extra-append`, `--dry-run` |
| `log_cleaner.sh` | Local log cleanup helper. | see above |

## Operational Notes

- Scripts are copied from `update_manager/scripts` plus selected repo-root helper scripts during install.
- Installer also writes `/etc/sudoers.d/saturn-go-maintenance` so the service
  user can run root-owned copies of `install-shutdown-waiter-service.sh`,
  `setup-eth-fallback.sh`, `fix-LED-power-button.sh`,
  `saturn-flash-fpga.sh`, and `saturn-xdma-doctor.sh` from
  `/usr/local/lib/saturn-go/scripts` with `sudo -n`.
- File permissions are normalized by installer:
  - `*.sh` and `*.py` scripts are set executable.
- Script execution from UI is constrained to filenames in `/opt/saturn-go/scripts`.
- `/run` rejects `.py` execution if the resolved script path is under the active repo root (`SATURN_REPO_ROOT`), preventing repo-tree Python runs.
- Installer permissions keep `/opt/saturn-go/scripts` writable by the service user so browser-managed custom script content updates can persist.
- Matching root-owned helper copies live in `/usr/local/lib/saturn-go/scripts`
  for privileged handoff from Update G2 and the Custom Scripts page.
- SSE streaming route (`/run`) handles stdout and stderr with low-latency buffering behavior.
- `/run` injects active repo-root context (`SATURN_REPO_ROOT`, `SATURN_DIR`, `SATURN_ACTIVE_REPO_ROOT`) so scripts operate on the currently selected Saturn checkout.
- `/run` injects Saturn Go self-update policy variables and `SATURN_SATURNGO_DEPLOY_STATUS_FILE` when launching `update-saturn-go.sh`.
- Python scripts launched by `/run` use `PYTHONDONTWRITEBYTECODE=1` and `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`.
- `update-G2.py` participates in the shared update-activity lock with appliance update/rollback routes to avoid overlapping update operations.
- `update-G2.py` now runs `install-shutdown-waiter-service.sh` and
  `fix-LED-power-button.sh` as part of the G2 maintenance flow.
- `setup-eth-fallback.sh` remains available through Custom Scripts for manual
  execution when needed.
- `update-saturn-go.sh` also participates in the shared update-activity lock and writes last deploy status JSON for the Saturn Go page.
- `update-G2.py` emits `SATURN_WEB_MANAGER_CHANGED=1` when pulled commits modify paths under `update_manager/`; the G2 page uses that marker to optionally chain a final `update-saturn-go.sh --skip-git --verbose` post-step.
- `update-saturn-go.sh --skip-git` now works from the active repo root even if no separate Saturn Go repo policy URL is configured, which is what allows the post-G2 self-update chain to reuse the repo that `update-G2.py` just updated.
- `update-deskhpsdr.py` resolves helper scripts from the active repo root, clones/pulls `~/github/deskhpsdr` unless `--skip-git` is selected, and then delegates the build to `scripts/deskhpsdr-test-build-on-current-image.sh`.
- `scripts/deskhpsdr-test-build-on-current-image.sh` applies `scripts/patches/deskhpsdr-libgpiod-v2.patch` only for older deskHPSDR checkouts that still include the legacy `src/gpio.c` path, and accepts an already-applied patch as success.
- The same helper always applies
  `scripts/patches/deskhpsdr-active-receiver-init.patch` to prevent the Saturn
  XDMA Connect initialization crash, and installs a direct Desktop application
  launcher after a successful build.
- For current upstream deskHPSDR checkouts where direct Raspberry Pi GPIO support has been removed, the helper skips the obsolete patch and builds with `SATURN=ON` only for the G2/XDMA path.
- The deskHPSDR helper keeps `libpulse-dev` for building Pulse audio support but prefers `pipewire-pulse` at runtime and removes the redundant `pulseaudio` daemon package when PipeWire Pulse is installed.
- `p23-app-manager.sh` is an advanced local test/deploy helper; it modifies a systemd drop-in override for `p2app.service` rather than editing the base unit file directly.
- `p23-app-manager.sh` writes `Environment=SATURN_FRONT_PANEL_MODE=...` into the generated override for forced/assisted panel detection testing and tags the override with a `# saturn-p23 mode=... panel=...` comment that the status API parses.
- `p23-app-manager.sh` now drives only the converged `P2_app` source tree and deployed `p2app` binary; old `p3` arguments remain compatibility aliases.
