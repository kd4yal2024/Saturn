# Saturn Provisioning

This directory contains provisioning assets for cloud-init based setup of a Saturn system.

## Current Layout

- `cloud-init/user-data.example.yaml`
- `cloud-init/meta-data.example.yaml`
- `cloud-init/provision-saturn.sh`
- `cloud-init/saturn-provision-ui.cpp`

## What It Does

`cloud-init/provision-saturn.sh` runs as root and:

- installs required apt packages
- retries transient apt lock conflicts during package index/install operations instead of failing immediately
- installs matching Raspberry Pi kernel headers when available in apt sources
- installs desktop developer tools when available in apt sources:
  - Visual Studio Code (`code`)
  - Git Cola (`git-cola`)
- installs VS Code extensions for `SATURN_USER` when `code` is available:
  - `ms-vscode.cpptools` (C/C++)
  - `eamodio.gitlens` (GitLens)
- enables `i2c`, `ssh`, and `vnc` by default when requested (`SATURN_ENABLE_*`)
- ensures USB host overlay in boot config:
  - `dtoverlay=dwc2,dr_mode=host`
- ensures cordless input tuning in boot cmdline:
  - `usbhid.mousepoll=0`
- can apply a managed LCD boot profile for CM4/CM5 + Waveshare 7"/8" combos (`SATURN_LCD_PROFILE`)
- clones or updates `kd4yal2024/Saturn` (default branch `main`)
- builds Saturn apps and tools (including `sw_tools`)
- installs desktop launchers (deprecated `P2app.desktop` is removed and not reinstalled)
- optionally builds/installs XDMA
- leaves `/home/pi/github/Saturn/scripts/fix-xdma.sh` available for later XDMA rebuilds after kernel updates
- optionally installs udev rules
- optionally installs `p2app-control` tray control (AppIndicator-based)
- optionally installs Update Manager
- optionally flashes FPGA (disabled by default)

Completion and logs:

- state file: `/var/lib/saturn-provision/complete`
- front-panel state file: `/var/lib/saturn-provision/front-panel-type`
- log file: `/var/log/saturn-provision.log`
- live status file (desktop UI): `/var/lib/saturn-provision/ui-status`

## Desktop GTK Provisioning UI (Optional)

`provision-saturn.sh` now supports an optional desktop widget written in C++ with GTK3.
It can show:

- current provisioning stage
- elapsed time and ETA countdown
- toggleable live log panel
- final success/failure state

Environment controls:

- `SATURN_DESKTOP_UI=auto|1|0` (default: `auto`)
  - `auto`: launches UI only when an X11 display is available
  - `1`: force attempt to launch UI
  - `0`: disable UI
- `SATURN_UI_TIMEOUT_SECONDS` (default: `2700`)
- `SATURN_UI_SHOW_LOG_DEFAULT=1|0` (default: `0`)
- `SATURN_UI_BINARY` (default: `/usr/local/bin/saturn-provision-ui`)
- `SATURN_UI_STATUS_FILE` (default: `/var/lib/saturn-provision/ui-status`)
- `SATURN_CLEAN_TMP_AFTER_PROVISION=1|0` (default: `1`)
- `SATURN_APT_LOCK_TIMEOUT_SECONDS` (default: `120`)
- `SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS` (default: `3`)
- `SATURN_DETECT_FRONT_PANEL=1|0` (default: `1`)
- `SATURN_FRONT_PANEL_STATE_FILE` (default: `/var/lib/saturn-provision/front-panel-type`)

Notes:

- Cloud-init boots without a desktop session in many images; in that case `auto` mode will skip UI and continue normal provisioning.
- Provisioning now installs desktop UI prerequisites early (`g++`, `pkg-config`, `libgtk-3-dev`) and attempts to launch the UI near the start of the run.
- Provisioning now installs a per-user autostart entry at `~/.config/autostart/saturn-provision-ui.desktop` for `SATURN_USER` (default `pi`), so the widget appears when that desktop session starts.
- Once launched, the provisioning UI remains open after completion until the user clicks `Close`.
- On successful provisioning completion, that autostart entry is removed automatically to avoid launching on every future desktop login.
- On successful provisioning completion, temporary Saturn artifacts under `/tmp` are cleaned by default (`SATURN_CLEAN_TMP_AFTER_PROVISION=1`).
- For interactive desktop runs, preserve display environment when escalating, for example:
  - `sudo -E SATURN_DESKTOP_UI=1 bash provision-saturn.sh`
- No Python files are added for this UI path; the widget is C++/GTK only.
- Apt lock retries only activate on lock contention. The normal apt path does not add an extra delay.

Environment precedence:

- Explicit runtime `SATURN_*` environment variables now take precedence over `/etc/default/saturn-provision`.

## LCD Profiles (CM4/CM5 + 7"/8")

Provisioning can append a managed LCD block to `config.txt` without replacing the file.
This is designed to preserve existing HDMI-related entries while adding panel-specific overlays.

Environment controls:

- `SATURN_LCD_PROFILE=none|cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8|auto` (default: `auto`)
- `SATURN_LCD_SIZE_INCH=7|8` (optional explicit override for `SATURN_LCD_PROFILE=auto`)
- `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH=7|8` (optional fallback when auto detection is ambiguous)
- `SATURN_LCD_I2C_DETECT_ADDR=0x45` (optional I2C address used by size auto-probe; valid `i2cdetect` range is `0x08..0x77`)
- `SATURN_LCD_DETECT_ONLY=1|0` (default: `0`; when `1`, resolves/logs profile but does not write `config.txt`)

Profile mapping:

- `cm4-7`: `dtoverlay=uart3` + `dtoverlay=vc4-kms-dsi-waveshare-800x480`
- `cm4-7-custom-jd`: `dtoverlay=uart3` + `dtoverlay=vc4-kms-dsi-waveshare-800x480`
- `cm4-7-g2-single-dsi`: `dtoverlay=uart3` + `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0`
- `cm4-8`: `dtoverlay=uart3` + `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1`
- `cm5-7`: `dtoverlay=uart2-pi5` + `dtoverlay=vc4-kms-dsi-waveshare-800x480`
- `cm5-7-g2-single-dsi`: `dtoverlay=uart2-pi5` + `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0`
- `cm5-7-g2-dual-dsi`: `dtoverlay=uart2-pi5` + `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1` + `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0`
- `cm5-8`: `dtoverlay=uart2-pi5` + `dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1`

Notes:

- The managed block is delimited by `# BEGIN SATURN LCD PROFILE` and `# END SATURN LCD PROFILE`.
- Re-running provisioning replaces only that managed block.
- Existing HDMI lines outside the managed block are left untouched.
- Display profile changes generally require reboot to take effect.
- Auto mode first preserves a known Saturn-managed profile id from the managed LCD block comment when one is present.
- Auto mode preserves Laurence-style single-DSI 7-inch configs when it finds `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0` paired with `dtoverlay=uart3` or `dtoverlay=uart2-pi5`.
- Auto mode preserves an existing dual-overlay CM5 7" G2 config when it finds both `dsi1/i2c0` and `dsi0/i2c1` overlay lines already present.
- Auto mode resolves in this order: `SATURN_LCD_SIZE_INCH` -> existing Waveshare overlay in `config.txt` -> I2C probe (`i2c-10`/`i2c-0` implies 7", `i2c-1` implies 8") -> `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH`.
- Safe dry-run example (no boot config changes):
  - `sudo SATURN_FORCE_REPROVISION=1 SATURN_LCD_PROFILE=auto SATURN_LCD_DETECT_ONLY=1 /home/pi/github/Saturn/provision/cloud-init/provision-saturn.sh`

## Saturn LCD Setup Desktop Tool

For interactive systems, Saturn now includes a standalone desktop tool for LCD profile selection and recovery:

- `Saturn LCD Setup`
- binary: `sw_tools/saturn-lcd-setup/saturn-lcd-setup`
- helper: `scripts/saturn-lcd-helper.sh`

Use this when:

- auto-detection is ambiguous
- you are testing multiple LCD configurations
- you want to preview the exact managed LCD block before writing
- you need to restore a previous `config.txt` backup after a failed LCD attempt

Behavior:

- shows detected compute module, current profile, and current Waveshare overlay lines
- previews the selected LCD profile before apply
- creates a timestamped backup before changing `config.txt`
- allows restore from saved LCD backups through the UI
- prompts for reboot after apply or restore

Provisioning integration:

- provisioning builds `saturn-lcd-setup` during the normal app/tool build stage
- provisioning installs the desktop launcher through `scripts/update-desktop-apps.sh`

Backups created by the tool are stored next to `config.txt` as:

- `config.txt.bak.lcd-tool.<timestamp>`

CLI examples:

- list backups:
  - `/home/pi/github/Saturn/scripts/saturn-lcd-helper.sh backups`
- restore latest backup:
  - `sudo /home/pi/github/Saturn/scripts/saturn-lcd-helper.sh restore-latest`

## Standalone Execution

You can run `provision-saturn.sh` directly without cloud-init.

Requirements:

- run as root (`sudo`)
- target user exists (`SATURN_USER`, default `pi`)
- network access for apt and git operations
- writable boot files (`/boot/firmware/...` or `/boot/...`) for LCD/USB/cmdline updates

From inside the Saturn repo:

- `sudo ./provision/cloud-init/provision-saturn.sh`

Or by absolute path:

- `sudo /home/pi/github/Saturn/provision/cloud-init/provision-saturn.sh`

With example overrides:

- `sudo SATURN_USER=pi SATURN_REPO_BRANCH=main SATURN_FORCE_REPROVISION=1 /home/pi/github/Saturn/provision/cloud-init/provision-saturn.sh`

Behavior notes:

- script is idempotent by state marker (`/var/lib/saturn-provision/complete`)
- set `SATURN_FORCE_REPROVISION=1` to force a full rerun
- if `SATURN_USER` does not exist yet, script waits and retries every `SATURN_USER_RETRY_SECONDS` (default `30`)
- when already provisioned and not forced, UI status is `SKIPPED` (not `SUCCESS`)

## Raspberry Pi Imager Workflow (End-to-End)

This section describes the complete workflow for building an SD card with Raspberry Pi Imager and having Saturn provision automatically on first boot.

### 1. Choose an OS image that supports cloud-init

- This provisioning flow depends on cloud-init.
- Use an image that has cloud-init enabled (for example Ubuntu Server images in Raspberry Pi Imager, or a custom image where cloud-init is installed and active).
- If cloud-init is not active on the image, provisioning will not auto-run.

### 2. Prepare cloud-init inputs from this repo

From this repo:

- `provision/cloud-init/user-data.example.yaml`
- `provision/cloud-init/meta-data.example.yaml`

Copy and customize as needed:

- set `SATURN_USER` to the actual login user that will exist on the target image
- keep `SATURN_REPO_URL` and `SATURN_REPO_BRANCH` as needed
- review feature toggles (`SATURN_INSTALL_*`, `SATURN_REBUILD_XDMA`, `SATURN_BUILD_OPTIONAL_TOOLS`, `SATURN_ENABLE_*`, `SATURN_LCD_*`)
- leave FPGA flashing off by default unless intentionally enabled

### 3. Write SD card with Raspberry Pi Imager

- Select Raspberry Pi device
- Select OS image
- Select storage
- Open Imager OS customization and set hostname/user/SSH/network as needed
- Ensure the username configured in Imager matches `SATURN_USER` in `user-data`

### 4. Provide cloud-init seed files to the image

Depending on image behavior:

- If image + Imager path already supports cloud-init user-data injection, use that mechanism.
- Otherwise, after writing the card, mount the boot/system-boot partition and place files as:
  - `user-data`
  - `meta-data`
  using content from:
  - `provision/cloud-init/user-data.example.yaml`
  - `provision/cloud-init/meta-data.example.yaml`

### 5. First boot execution flow

On first boot, cloud-init processes `user-data` and executes:

- writes `/etc/default/saturn-provision`
- ensures `~/github/Saturn` exists for `SATURN_USER`
- clones or updates `kd4yal2024/Saturn`
- runs:
  - `bash "$SATURN_HOME/github/Saturn/provision/cloud-init/provision-saturn.sh"`

Then `provision-saturn.sh` performs:

- apt package install
- kernel header checks/install (for XDMA build path)
- Saturn repo sync and build of apps/tools
- desktop launcher install
- optional XDMA build/install/load
  - later kernel package upgrades can be handled by rerunning:
    - `sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh`
  - that helper rebuilds for the running kernel and, when a newer same-flavor kernel is already installed, also pre-stages XDMA for that next boot
  - the helper now builds in the repo as `SATURN_USER` and reserves root only for module install/reload, which avoids leaving kernel build outputs owned by root or other mapped IDs in `linuxdriver/xdma`
- optional udev rules install
- optional front-panel detection
  - runs after the udev-install step
  - detects G2V1 by I2C device presence at `0x20`
  - detects G2V2 by CAT response on `/dev/serial/by-id/g2-front-9600`
  - records `G2V1`, `G2V2`, or `NONE` in `/var/lib/saturn-provision/front-panel-type`
  - also includes `front_panel_type=...` in `/var/lib/saturn-provision/complete`
- optional p2app-control install
  - installs tray autostart (`~/.config/autostart/P2_app-Control-tray.desktop`)
  - requires `libayatana-appindicator3-dev` and `ayatana-indicator-application`
  - generated `p2app.service` now waits for `/dev/xdma0_user` before launching `p2app`
  - installer now waits for `p2app.service` to reach `active/running`
  - if the XDMA kernel module is loaded but `/dev/xdma0_user` is still missing, provisioning logs the condition and continues instead of failing the whole run at the `p2app-control` step
  - if a panel does not render the tray icon, run `/usr/local/bin/p2app-control --window` as fallback
- optional Update Manager install
- optional FPGA flash (only if explicitly enabled and confirmed)
- completion marker write

### 6. Verify completion

After boot completes, verify:

- `sudo cat /var/lib/saturn-provision/complete`
- `sudo cat /var/lib/saturn-provision/front-panel-type`
- `sudo tail -n 200 /var/log/saturn-provision.log`
- `sudo systemctl status p2app.service --no-pager`
- `ls -l /dev/xdma0_user /dev/xdma/card0 2>/dev/null`

If Update Manager is enabled, also verify service status:

- `sudo systemctl status saturn-go.service --no-pager`

### 7. Re-run behavior

- Provisioning is idempotent by state marker:
  - if `/var/lib/saturn-provision/complete` exists, script exits cleanly
- To force a full rerun, set:
  - `SATURN_FORCE_REPROVISION=1`
  in `/etc/default/saturn-provision`, then run the script again as root

## Cloud-Init Inputs

`cloud-init/user-data.example.yaml` writes `/etc/default/saturn-provision` and executes:

- `bash "$SATURN_HOME/github/Saturn/provision/cloud-init/provision-saturn.sh"`

Cloud-init bootstrap behavior in the example config:

- `package_update: false`
- `package_upgrade: false`
- `packages:` remains limited to bootstrap prerequisites:
  - `git`
  - `ca-certificates`
  - `sudo`

This keeps Saturn responsible for the main apt workflow and avoids a redundant cloud-init package index refresh before `provision-saturn.sh` starts.

`cloud-init/meta-data.example.yaml` is the companion metadata file for NoCloud style cloud-init.

## Important Defaults

From `user-data.example.yaml`:

- `SATURN_USER=pi`
- `SATURN_USER_RETRY_SECONDS=30`
- `SATURN_INSTALL_UPDATE_MANAGER=1`
- `SATURN_INSTALL_P2APP_CONTROL=1`
- `SATURN_INSTALL_UDEV_RULES=1`
- `SATURN_INSTALL_SHUTDOWN_WAITER=1`
- `SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT=auto`
- `SATURN_REBUILD_XDMA=1`
- `SATURN_BUILD_OPTIONAL_TOOLS=1`
- `SATURN_DETECT_FRONT_PANEL=1`
- `SATURN_ENABLE_I2C=1`
- `SATURN_ENABLE_SSH=1`
- `SATURN_ENABLE_VNC=1`
- `SATURN_LCD_PROFILE=auto`
- `SATURN_DESKTOP_UI=auto`
- `SATURN_UI_TIMEOUT_SECONDS=2700`
- `SATURN_UI_SHOW_LOG_DEFAULT=0`
- `SATURN_FLASH_FPGA=0` (safety default)
- `SATURN_APT_LOCK_TIMEOUT_SECONDS=120`
- `SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS=3`

Important safety settings for flashing:

- `SATURN_FLASH_FPGA=0` keeps flashing disabled by default
- If set to `1`, `SATURN_FLASH_CONFIRM` is required
- Optional fallback flashing is controlled by `SATURN_FLASH_FALLBACK`

## Repo-Clean Safety Guard

Provisioning is configured to keep the repo clean of Python cache artifacts:

- `PYTHONDONTWRITEBYTECODE=1`
- `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`
- blocks Python script execution from inside the repo tree during provisioning
- removes `__pycache__`, `*.pyc`, and `*.pyo` under repo before completion

## Notes

- Ensure `SATURN_USER` exists in the target image before provisioning runs.
- If you use a user other than `pi`, update `SATURN_USER` in `user-data`.
- Network access is required on first boot for apt and git operations.
- `cloud-init` user-data is root-owned; read it with `sudo cat /var/lib/cloud/instance/user-data.txt`.
- Provisioning now waits and retries every `SATURN_USER_RETRY_SECONDS` (default `30`) until `SATURN_USER` exists.
- `P1_app` is intentionally skipped in provisioning (legacy target not required for current images).
- Before capturing/cloning a reusable image, run `sudo cloud-init clean --logs` so first-boot provisioning re-runs on cloned targets.
- Keep `meta-data` `instance-id` unique per image/seed when possible.
