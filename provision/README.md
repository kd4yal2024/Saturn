# Saturn Provisioning

This directory contains provisioning assets for cloud-init based setup of a Saturn system.

## Current Layout

- `cloud-init/user-data.example.yaml`
- `cloud-init/meta-data.example.yaml`
- `cloud-init/provision-saturn.sh`
- `cloud-init/saturn-provision-powerctl.sh`
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
- optional `p2app-control` install also adds a kernel post-install hook so future kernel upgrades can pre-stage XDMA automatically
- optionally installs udev rules
- configures the power-switch/front-panel LED helper so BCM15 is driven red during normal operation and white on shutdown
- installs a root-owned provisioning power helper so the desktop UI can request reboot reliably at the end of first-boot setup
- optionally installs `p2app-control` tray control (AppIndicator-based)
- optionally installs Update Manager
- optionally flashes FPGA (disabled by default)

Completion and logs:

- state file: `/var/lib/saturn-provision/complete`
- provisioning profile file: `/var/lib/saturn-provision/profile.env`
- front-panel state file: `/var/lib/saturn-provision/front-panel-type`
- system-role state file: `/var/lib/saturn-provision/system-role`
- log file: `/var/log/saturn-provision.log`
- live status file (desktop UI): `/var/lib/saturn-provision/ui-status`
- completion state now also records:
  - `hardware_model`
  - `hardware_platform_vendor`
  - `hardware_module_family`
  - `hardware_storage_variant`
  - `xdma_present`
  - `system_role`

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
- `SATURN_FORCE_SYSTEM_ROLE=local_saturn|remotehead_candidate|unknown` (optional conservative support override)
- `SATURN_PROFILE_ENV_FILE` (default: `/var/lib/saturn-provision/profile.env`)

Notes:

- Cloud-init boots without a desktop session in many images; in that case `auto` mode will skip UI and continue normal provisioning.
- Provisioning now installs desktop UI prerequisites early (`g++`, `pkg-config`, `libgtk-3-dev`) and attempts to launch the UI near the start of the run.
- Provisioning now installs a per-user autostart entry at `~/.config/autostart/saturn-provision-ui.desktop` for `SATURN_USER` (default `pi`), so the widget appears when that desktop session starts.
- Provisioning also installs `/usr/local/sbin/saturn-provision-powerctl` plus a restricted sudoers entry so the UI `Reboot Now` action does not depend on an interactive polkit prompt or TTY-bound `systemctl` authorization.
- Once launched, the provisioning UI remains open after completion until the user clicks `Close`.
- On successful provisioning completion, that autostart entry is removed automatically to avoid launching on every future desktop login.
- On successful provisioning completion, temporary Saturn artifacts under `/tmp` are cleaned by default (`SATURN_CLEAN_TMP_AFTER_PROVISION=1`).
- For interactive desktop runs, preserve display environment when escalating, for example:
  - `sudo -E SATURN_DESKTOP_UI=1 bash provision-saturn.sh`
- No Python files are added for this UI path; the widget is C++/GTK only.
- Apt lock retries only activate on lock contention. The normal apt path does not add an extra delay.

Environment precedence:

- Explicit runtime `SATURN_*` environment variables now take precedence over `/etc/default/saturn-provision`.

## Standalone piHPSDR Installer Shortcut

Provisioning can also stage a separate Desktop shortcut for a standalone piHPSDR installer UI.
This is intentionally separate from the Saturn provisioning window.

What it does:

- installs a Desktop shortcut at `~/Desktop/piHPSDR-Installer.desktop`
- uses its own GTK C++ UI with a dedicated `Install piHPSDR` button
- shows terminal output in a bottom panel
- runs the installed `/opt/saturn-go/scripts/update-pihpsdr.py` flow
- removes the Desktop shortcut after a successful install when the user clicks `Close`

Environment controls:

- `SATURN_PIHPSDR_INSTALLER_ENABLED=1|0` (default: `1`)
- `SATURN_PIHPSDR_INSTALLER_BINARY` (default: `/usr/local/bin/pihpsdr-installer-ui`)
- `SATURN_PIHPSDR_INSTALLER_RUNNER` (default: `/usr/local/bin/pihpsdr-installer-run.sh`)
- `SATURN_PIHPSDR_INSTALLER_LAUNCHER` (default: `/usr/local/bin/pihpsdr-installer-launcher.sh`)
- `SATURN_PIHPSDR_INSTALLER_SHORTCUT_NAME` (default: `piHPSDR-Installer.desktop`)
- `SATURN_PIHPSDR_INSTALLER_TITLE` (default: `piHPSDR Installer`)
- `SATURN_PIHPSDR_INSTALLER_ICON_FILE` (optional explicit icon override)

Notes:

- This shortcut is created only after the Saturn Update Manager install step has deployed `update-pihpsdr.py` under `/opt/saturn-go/scripts/`.
- The standalone installer is not auto-launched; it is meant to be user-invoked from the Desktop.
- When no explicit icon override is set, provisioning reuses the existing piHPSDR icon when present (`piHPSDR_logo.png` or the current `pihpsdr.desktop` icon path).
- Closing the window while an install is still running leaves the Desktop shortcut in place so the user can reopen the installer and continue watching progress.

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
- LCD/profile detection logic is centralized in `scripts/saturn-lcd-lib.sh` so provisioning, CLI detection, and the GTK setup tool resolve profiles the same way.
- Hardware classification now reads `/proc/device-tree/model` and reports:
  - raw `model`
  - `platform_vendor` (`raspberrypi`, `radxa`, or `unknown`)
  - `module_family` (`cm4`, `cm5`, or `unknown`)
  - `storage_variant` (`lite`, `emmc`, or `unknown`; best-effort only)
- Auto mode first preserves a known Saturn-managed profile id from the managed LCD block comment when one is present.
- Auto mode preserves Laurence-style single-DSI 7-inch configs when it finds `dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0` paired with `dtoverlay=uart3` or `dtoverlay=uart2-pi5`.
- Auto mode preserves an existing dual-overlay CM5 7" G2 config when it finds both `dsi1/i2c0` and `dsi0/i2c1` overlay lines already present.
- Auto mode resolves in this order: `SATURN_LCD_SIZE_INCH` -> existing Waveshare overlay in `config.txt` -> I2C probe (`i2c-10`/`i2c-0` implies 7", `i2c-1` implies 8") -> `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH`.
- `SATURN_LCD_PROFILE=auto` currently applies Raspberry Pi CM4/CM5 overlay rules only; on non-Raspberry-Pi platforms such as Radxa it will log the detected model/vendor and require an explicit profile.
- After `cm` and size are known, the 7-inch front-panel tiebreaker now separates G2 variants:
  - `CM4 + G2V1/G2V2` -> `${cm}-7-g2-single-dsi`
- First provisioning now installs udev rules and detects the front panel before applying the LCD profile, so the G2 7-inch tiebreaker is active on the first run instead of only on reprovision/helper use.
- Front-panel detection in Saturn is now intentionally hardware-only:
  - state file values are `G2V1`, `G2V2`, or `NONE`
  - `ZZZS08...` is currently treated as `G2V2`-class hardware for LCD/provisioning purposes
  - a future `RemoteHead` concept should be modeled separately as a system role, not as a front-panel type
- provisioning now also records a conservative `system_role` result:
  - `local_saturn` when XDMA is present
  - `remotehead_candidate` when the current heuristic sees `CM5 + G2V2 + no XDMA`
  - `unknown` otherwise
- provisioning now also writes a factual machine-readable profile summary to `/var/lib/saturn-provision/profile.env`
  - current first fields include:
    - `radio_profile`
    - `radio_profile_source`
    - `discovered_processor`
    - `hardware_*`
    - `front_panel_type`
    - `front_panel_device_path`
    - `front_panel_device_addr`
    - `xdma_present`
    - `system_role`
    - `expected_display_type`
    - `configured_display_type`
    - `lcd_profile`
    - `display_size_inch`
    - `lcd_profile_source`
    - `uart_overlay`
    - `panel_overlay`
    - `pa_protection`
    - `ganymede_present`
    - `ganymede_device_path`
    - `atu`
    - `aries_present`
    - `aries_device_path`
- `expected_display_type` is the front-panel/system-role expectation and may differ from a user-installed display
- `configured_display_type` reflects the actual configured LCD/profile result written or preserved in boot config
- that `system_role` state is reporting-only for now; it does not auto-apply role-specific boot behavior yet
- `CM5 + 7"` is currently explicit/manual-only in `SATURN_LCD_PROFILE=auto`
  - explicit profiles `cm5-7`, `cm5-7-g2-single-dsi`, and `cm5-7-g2-dual-dsi` are still available for Saturn LCD Setup and manual testing
  - auto mode now refuses to pick a CM5 7-inch profile until that path is validated under Trixie
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
- shows detected front-panel type and whether it came from the saved provisioning state file or a live probe
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
- on current Raspberry Pi OS cloud-init images, preserve the login-user creation in the active `user-data`
  - if you replace Imager-generated `user-data` wholesale, make sure the login user still exists and matches `SATURN_USER`

### 3. Write SD card with Raspberry Pi Imager

- Select Raspberry Pi device
- Select OS image
- Select storage
- Open Imager OS customization and set hostname/user/SSH/network as needed
- Ensure the username configured in Imager matches `SATURN_USER` in `user-data`
- If you later replace or merge `user-data`, do not accidentally drop the login-user creation that Imager wrote there

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
- writes `/usr/local/sbin/saturn-cloudinit-bootstrap.sh`
- logs early bootstrap work to `/var/log/saturn-cloudinit-bootstrap.log`
- ensures `~/github/Saturn` exists for `SATURN_USER`
- clones or updates `kd4yal2024/Saturn`
- runs:
  - `bash "$SATURN_HOME/github/Saturn/provision/cloud-init/provision-saturn.sh"`

Bootstrap behavior before the main Saturn script now is:

- waits for system clock synchronization before first apt/git activity
- installs bootstrap prerequisites itself after the clock is sane
- retries while waiting for `SATURN_USER` to exist
- if the configured `SATURN_USER` never appears, falls back to the first normal `/home/*` login user when one exists
- fails with an explicit bootstrap log message instead of silently stopping before Saturn logging begins

Then `provision-saturn.sh` performs:

- apt package install
- I2C/SSH/VNC enablement
- optional udev rules install
- optional front-panel detection
  - now runs a pre-udev raw detection pass before udev rule installation
  - then runs a post-udev verification pass after rule installation
  - runs before LCD profile application
  - detects G2V1 by an MCP23017-compatible register response (`IODIR_A == 0xFF`) at I2C address `0x20`
  - pre-udev serial detection now probes raw UART/USB device nodes first so G2V2-class front-panel hardware can be identified before serial alias rules are installed
  - post-udev serial detection still uses `/dev/serial/by-id/g2-front-9600` for stable normal operation
  - detects G2V2-class panel hardware by `ZZZS05...` or `ZZZS08...` CAT response
  - records `G2V1`, `G2V2`, or `NONE` in `/var/lib/saturn-provision/front-panel-type`
  - also includes `front_panel_type=...` in `/var/lib/saturn-provision/complete`
- udev serial rule installation now receives the detected front-panel type for logging/state, but currently installs the standard `61-g2-serial.rules` file unless explicitly overridden
- LCD boot profile apply
  - uses the shared logic from `scripts/saturn-lcd-lib.sh`
  - on 7-inch systems, now separates G2 variants:
    - `CM4 + G2V1/G2V2` -> `cm4-7-g2-single-dsi`
  - on `CM5 + 7"` systems, `SATURN_LCD_PROFILE=auto` now warns and requires an explicit profile instead of pretending the path is validated
- kernel header checks/install (for XDMA build path)
- Saturn repo sync and build of apps/tools
- desktop launcher install
- optional XDMA build/install/load
  - later kernel package upgrades can be handled by rerunning:
    - `sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh`
  - that helper rebuilds for the running kernel and, when a newer same-flavor kernel is already installed, also pre-stages XDMA for that next boot
  - the helper now builds in the repo as `SATURN_USER` and reserves root only for module install/reload, which avoids leaving kernel build outputs owned by root or other mapped IDs in `linuxdriver/xdma`
  - beta images can also register the same supported driver source with DKMS:
    - `sudo bash /home/pi/github/Saturn/scripts/install-xdma-dkms.sh --force`
  - a successful DKMS install disables `/etc/kernel/postinst.d/saturn-xdma` so
    the legacy manual hook and DKMS do not both rebuild XDMA during kernel
    package updates
- optional p2app-control install
  - installs tray autostart (`~/.config/autostart/P2_app-Control-tray.desktop`)
  - requires `libayatana-appindicator3-dev` and `ayatana-indicator-application`
  - installs `/usr/local/bin/saturn-xdma-doctor.sh` as the supported XDMA diagnostic helper
  - installs `/usr/local/bin/saturn-xdma-ready.sh` plus `saturn-xdma-ready.service` as the dedicated XDMA readiness gate
  - installs `/usr/local/bin/saturn-fix-xdma.sh` and `/usr/local/bin/saturn-xdma-kernel-postinst.sh`
  - installs `/etc/kernel/postinst.d/saturn-xdma` so future kernel package installs pre-stage `xdma.ko` without unloading the live module or restarting `p2app.service`
  - generated `p2app.service` now depends on `saturn-xdma-ready.service` instead of owning the XDMA readiness loop directly
  - installer now waits for `p2app.service` to reach `active/running`
  - if the XDMA kernel module is loaded but `/dev/xdma0_user` is still missing, provisioning logs the condition and continues instead of failing the whole run at the `p2app-control` step
  - if a panel does not render the tray icon, run `/usr/local/bin/p2app-control --window` as fallback
- optional Update Manager install
- optional FPGA flash (only if explicitly enabled and confirmed)
- completion marker write

### 6. Verify completion

After boot completes, verify:

- `sudo tail -n 200 /var/log/saturn-cloudinit-bootstrap.log`
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
- via `/usr/local/sbin/saturn-cloudinit-bootstrap.sh`, which records early bootstrap activity in `/var/log/saturn-cloudinit-bootstrap.log`

Cloud-init bootstrap behavior in the example config:

- `package_update: false`
- `package_upgrade: false`
- bootstrap now waits for NTP/system clock sync before its own apt/git work
- the example no longer uses top-level cloud-init `packages:` for Saturn bootstrap prerequisites
  - this avoids early apt signature checks running before time sync is established
  - the bootstrap helper installs `git`, `ca-certificates`, and `sudo` itself only after the clock is sane

This keeps Saturn responsible for the first meaningful apt activity, avoids a redundant cloud-init package index refresh before `provision-saturn.sh` starts, and prevents early package work from racing ahead of time synchronization.

`cloud-init/meta-data.example.yaml` is the companion metadata file for NoCloud style cloud-init.

## Important Defaults

From `user-data.example.yaml`:

- `SATURN_USER=pi`
- `SATURN_USER_RETRY_SECONDS=30`
- `SATURN_CLOCK_SYNC_WAIT_SECONDS=180`
- `SATURN_CLOCK_SYNC_POLL_SECONDS=5`
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
- `SATURN_ADMIN_PASSWORD=admin` effective default (the example leaves it blank, and provisioning keeps the admin/admin first-login path)

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
- On current Raspberry Pi OS cloud-init images, if you replace the generated `user-data`, keep the login-user creation block or Saturn bootstrap may not find the expected user.
- Network access is required on first boot for apt and git operations.
- On first boot without an RTC, provisioning now waits for network time before bootstrap apt/git activity instead of hitting repository signature checks with a stale clock.
- `cloud-init` user-data is root-owned; read it with `sudo cat /var/lib/cloud/instance/user-data.txt`.
- Provisioning now waits and retries every `SATURN_USER_RETRY_SECONDS` (default `30`) until `SATURN_USER` exists.
- Early clone/bootstrap failures are logged to `/var/log/saturn-cloudinit-bootstrap.log`.
- `P1_app` is intentionally skipped in provisioning (legacy target not required for current images).
- Before capturing/cloning a reusable image, run `sudo cloud-init clean --logs` so first-boot provisioning re-runs on cloned targets.
- Keep `meta-data` `instance-id` unique per image/seed when possible.
