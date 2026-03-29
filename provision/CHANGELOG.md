# Changelog

All notable changes to provisioning assets are documented in this file.

## [Unreleased]

### Changed

- `cloud-init/user-data.example.yaml`
  - now writes `/usr/local/sbin/saturn-cloudinit-bootstrap.sh` as an explicit first-boot bootstrap helper
  - bootstrap now logs to `/var/log/saturn-cloudinit-bootstrap.log` before the Saturn repo is cloned
  - bootstrap now retries while waiting for `SATURN_USER`
  - if the configured `SATURN_USER` never appears, bootstrap now falls back to the first normal `/home/*` login user instead of failing immediately at `getent passwd`
  - bootstrap now waits for system clock synchronization before first apt/git activity
  - added `SATURN_CLOCK_SYNC_WAIT_SECONDS` (default `180`) and `SATURN_CLOCK_SYNC_POLL_SECONDS` (default `5`)
  - bootstrap now installs its own `git` / `ca-certificates` / `sudo` prerequisites after the clock is sane instead of relying on top-level cloud-init `packages:`
  - removed the example top-level cloud-init `packages:` block so apt signature checks do not run before time sync is established

- `cloud-init/provision-saturn.sh`
  - now runs `../scripts/fix-LED-power-button.sh` during provisioning so first-boot setup configures the BCM15 power-switch/front-panel LED the same way as the Update G2 maintenance flow
  - now sources shared LCD/profile logic from `../scripts/saturn-lcd-lib.sh` instead of carrying a separate copy of those helpers
  - added explicit `cm4-7-custom-jd` LCD profile as a preserved alias for the current CM4 custom 7-inch panel setup
  - added explicit `cm4-7-g2-single-dsi` and `cm5-7-g2-single-dsi` LCD profiles for Laurence-style `7_0_inchC,i2c0` configs
  - auto LCD detection now preserves Saturn-managed explicit profile ids from the managed block comment before falling back to overlay heuristics
  - auto LCD detection now preserves existing Laurence-style single-DSI 7-inch configs instead of collapsing them into the generic `cm4-7` / `cm5-7` profiles
  - LCD auto mode now separates G2 variants on 7-inch systems:
    - `CM4 + G2V1/G2V2` -> `cm4-7-g2-single-dsi`
    - `CM5 + G2V1` -> `cm5-7-g2-dual-dsi`
    - `CM5 + G2V2` -> `cm5-7-g2-single-dsi`
  - front-panel detection is now treated as hardware-only state:
    - provisioning records only `G2V1`, `G2V2`, or `NONE`
    - `ZZZS08...` is folded into `G2V2`-class hardware for LCD/provisioning purposes
    - any future `RemoteHead` concept should be modeled separately as a system role
  - front-panel detection now runs in two phases:
    - a pre-udev raw-device pass for early `G2V2` identification
    - a post-udev verification pass after rule installation
  - udev rule installation now receives the detected front-panel type for logging/state, while continuing to use the standard serial rules file unless explicitly overridden
  - added `SATURN_DETECT_FRONT_PANEL` (default `1`)
  - front-panel detection now records `G2V1`, `G2V2`, or `NONE` in provisioning state

- `../scripts/saturn-lcd-lib.sh`
  - new shared shell library for LCD/profile detection, rendering, and config application
  - now serves as the single authoritative implementation for provisioning, CLI detection, and the LCD setup helper
  - LCD I2C probe path now uses `i2cdetect -r` read-mode probing
  - now classifies hardware from `/proc/device-tree/model` into `platform_vendor`, `module_family`, and `storage_variant`
  - `SATURN_LCD_PROFILE=auto` now logs full hardware classification and refuses to apply Raspberry Pi overlay assumptions to non-Raspberry-Pi platforms

- `../sw_tools/p2app-control/install.sh`
  - installer now installs `/usr/local/bin/saturn-xdma-doctor.sh` as the supported XDMA diagnostic helper
  - installer now installs `/usr/local/bin/saturn-xdma-ready.sh` and a dedicated `saturn-xdma-ready.service` gate
  - installer now installs `/usr/local/bin/saturn-fix-xdma.sh` plus `/usr/local/bin/saturn-xdma-kernel-postinst.sh`
  - installer now installs `/etc/kernel/postinst.d/saturn-xdma` so future kernel installs can pre-stage XDMA without touching the live module/service
  - generated `p2app.service` now depends on `saturn-xdma-ready.service` instead of owning the XDMA readiness loop directly
  - installer now waits for `p2app.service` to reach `active/running` instead of treating transient start/restart states as an immediate provisioning failure
  - if XDMA is loaded but `/dev/xdma0_user` is still missing, the installer now logs the condition and lets provisioning continue so the log points at XDMA/FPGA device enumeration instead of the tray-widget install step

- `../scripts/detect-lcd-profile.sh`
  - now acts as a thin CLI wrapper around `saturn-lcd-lib.sh`
  - added explicit `cm4-7-custom-jd` profile detection/output
  - added explicit `cm4-7-g2-single-dsi` and `cm5-7-g2-single-dsi` profile detection/output
  - now reports raw hardware model plus `platform_vendor`, `module_family`, and `storage_variant`

- `../scripts/saturn-lcd-helper.sh`
  - now sources `saturn-lcd-lib.sh` directly instead of sourcing `provision-saturn.sh`
  - now prefers `/var/lib/saturn-provision/front-panel-type` over a live probe when resolving front-panel state
  - `detect` output now includes both `front_panel_type=` and `front_panel_source=`
  - `apply --profile auto` now reports and writes the same resolved profile
  - now lists the custom and Laurence-style single-DSI 7-inch profiles alongside the existing generic and dual-DSI profiles

- `../scripts/detect-front-panel.sh`
  - added a standalone front-panel detector
  - G2V1 detection now checks for an MCP23017-compatible `IODIR_A == 0xFF` register response at I2C address `0x20`
  - serial detection now retries once before returning `NONE`
  - now supports `--pre-udev` raw probing and `--post-udev` aliased probing modes
  - pre-udev mode probes raw UART/USB candidates first so G2V2-class panel hardware can be identified before udev renaming
  - G2V2 detection sends `ZZZS;` and looks for `ZZZS05`
  - `ZZZS08...` replies are currently treated as `G2V2`-class front-panel hardware for provisioning/LCD purposes

- `../rules/install-rules.sh`
  - now accepts `SATURN_FRONT_PANEL_TYPE` for logging/state while continuing to install the standard `61-g2-serial.rules` file unless explicitly overridden

### Documentation

- `README.md`
  - documented that provisioning now configures the BCM15 power-switch/front-panel LED helper
  - documented front-panel detection behavior, state file, and provisioning toggle
  - documented the new shared LCD/profile library and the first-provision front-panel-aware G2 7-inch LCD tiebreaker
  - documented the new custom and Laurence-style single-DSI 7-inch LCD profiles
  - documented the new XDMA readiness gate and doctor path installed with `p2app-control`
  - documented the new hardware classification fields and that `SATURN_LCD_PROFILE=auto` is currently Raspberry-Pi-only

  - documented the new `/var/log/saturn-cloudinit-bootstrap.log` early bootstrap log and the warning about preserving login-user creation in cloud-init `user-data`
  - documented the new first-boot clock-sync wait and the removal of early cloud-init `packages:` bootstrap apt work
- `../sw_tools/p2app-control/README.md`
  - documented the new XDMA readiness gate, doctor script, and the provisioning-time troubleshooting path for `register write attempted before XDMA register device was opened`

## [2026-03-18]

### Changed

- `../scripts/fix-xdma.sh`
  - now builds the XDMA module as the invoking Saturn user and only uses root for module install/reload
  - now creates timestamped backups of any installed `xdma.ko` or `xdma.ko.xz` before replacing them
  - now normalizes the active installed XDMA module ownership to `root:root` after `modules_install`

### Documentation

- `README.md`
  - documented that `fix-xdma.sh` keeps repo build artifacts owned by the Saturn user instead of root-only build/install flow

## [2026-03-17]

### Changed

- `cloud-init/provision-saturn.sh`
  - added bounded apt lock retry handling for `apt-get update` and `apt-get install`
  - added `SATURN_APT_LOCK_TIMEOUT_SECONDS` (default `120`) to cap wait time when another apt client temporarily owns the lock
  - added `SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS` (default `3`) to control retry cadence during apt lock contention

- `cloud-init/user-data.example.yaml`
  - changed `package_update` from `true` to `false` so cloud-init does not perform a redundant package index refresh before handing off to Saturn provisioning

### Documentation

- `README.md`
  - documented that Saturn provisioning now retries transient apt lock conflicts instead of failing immediately
  - documented the new apt lock retry tuning environment variables
  - documented that `user-data.example.yaml` no longer requests a separate cloud-init package index refresh

## [2026-03-16]

### Documentation

- `README.md`
  - documented `/home/pi/github/Saturn/scripts/fix-xdma.sh` as the preferred post-install XDMA recovery path after kernel updates
  - documented that the helper rebuilds for the running kernel and pre-stages XDMA for a newer already-installed same-flavor kernel before reboot

## [2026-03-09]

### Changed

- `../rules/install-rules.sh`
  - installer now copies both the serial udev rule and the XDMA PCIe rule/helper used by Saturn FPGA access
  - installer now applies executable permissions to `xdma-udev-command.sh`
  - installer now reloads udev rules and retriggers the XDMA subsystem after installation

### Fixed

- provisioning/invocation paths that relied on `rules/install-rules.sh` no longer miss the XDMA udev rules, which previously left systems without `/dev/xdma/card0/*` symlinks and non-root-accessible XDMA device nodes after install

## [2026-03-07]

### Changed

- `cloud-init/provision-saturn.sh`
  - moved desktop UI prerequisites/install path earlier in `main()` so status UI can launch near start of provisioning
  - added `SATURN_USER_RETRY_SECONDS` (default `30`) to control `SATURN_USER` wait-loop cadence
  - changed `SATURN_USER` wait loop log text to report retry seconds explicitly
  - hardened LCD auto-detection helper behavior to avoid propagating probe failures into profile strings
  - updated I2C probe range validation to align with `i2cdetect` limits (`0x08..0x77`)
  - changed ambiguous LCD auto-detect fallback default from `8` to `7` (`SATURN_LCD_AUTO_DEFAULT_SIZE_INCH`)
  - reordered provisioning flow so I2C setup runs before LCD profile resolution (improves first-boot auto-detect reliability)
  - added `SATURN_LCD_DETECT_ONLY=1` mode to resolve/log profile without writing `config.txt`
  - when applying LCD profile, now removes active legacy Waveshare panel overlay lines before writing managed block
  - removed distro `rustc`/`cargo` install from base dependency path (update-manager uses rustup-managed toolchain)
  - when invoking `scripts/update-desktop-apps.sh`, sets `SATURN_SKIP_P2APP_BUILD=1` to avoid duplicate P2App rebuild

- `scripts/detect-lcd-profile.sh`
  - mirrored LCD auto-detection hardening and `i2cdetect` range validation fixes from provisioning script

- `scripts/update-desktop-apps.sh`
  - added `SATURN_SKIP_P2APP_BUILD=1` support to skip optional P2App rebuild during launcher refresh

### Documentation

- `README.md`
  - documented early UI prerequisite install and near-start launch behavior
  - documented `SATURN_USER_RETRY_SECONDS` default and behavior
  - documented I2C detect address valid range note for LCD auto-probing
  - documented `SATURN_LCD_DETECT_ONLY` safe dry-run mode and example command
- `cloud-init/user-data.example.yaml`
  - added `SATURN_USER_RETRY_SECONDS=30` example default
  - updated LCD fallback example to `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH=7`
  - added `SATURN_LCD_DETECT_ONLY` example toggle

## [2026-03-06]

### Changed

- `cloud-init/provision-saturn.sh`
  - ensures USB host overlay in boot `config.txt`:
    - `dtoverlay=dwc2,dr_mode=host`
  - ensures cordless mouse/keyboard polling tuning in boot `cmdline.txt`:
    - `usbhid.mousepoll=0`
  - installs matching Raspberry Pi kernel headers during base package setup when available in apt sources
  - installs desktop dev tools when available in apt sources:
    - `code` (Visual Studio Code)
    - `git-cola`
  - installs VS Code extensions for `SATURN_USER` when `code` CLI is available:
    - `ms-vscode.cpptools`
    - `eamodio.gitlens`
  - when a completion marker already exists and reprovision is not forced, now writes UI state `SKIPPED` instead of `SUCCESS`

- `cloud-init/saturn-provision-ui.cpp`
  - added explicit `SKIPPED` state handling:
    - displays "Provisioning skipped"
    - message clarifies no new run executed on already provisioned systems

### Documentation

- `README.md`
  - documented new USB/cmdline boot tuning done by provisioning
  - documented desktop dev tool install behavior (`code`, `git-cola`)
  - documented automatic VS Code extension installation (`C/C++`, `GitLens`)
  - added standalone execution section with prerequisites, commands, and rerun behavior
  - documented `SKIPPED` status semantics for already provisioned systems

## [2026-03-01]

### Added

- `cloud-init/saturn-provision-ui.cpp`
  - new C++/GTK3 desktop provisioning widget with:
    - stage/status display
    - elapsed and ETA countdown
    - toggleable live log panel
    - final success/failure summary based on provisioning state

### Changed

- `cloud-init/provision-saturn.sh`
  - added optional desktop UI launch controls:
    - `SATURN_DESKTOP_UI=auto|1|0`
    - `SATURN_UI_TIMEOUT_SECONDS`
    - `SATURN_UI_SHOW_LOG_DEFAULT`
    - `SATURN_UI_BINARY`
    - `SATURN_UI_STATUS_FILE`
  - installs desktop autostart for `SATURN_USER` (default `pi`) at `~/.config/autostart/saturn-provision-ui.desktop`
  - removes the desktop autostart entry automatically after successful provisioning
  - added UI status protocol file updates (`RUNNING|SUCCESS|FAILED`)
  - added explicit stage updates throughout provisioning for richer desktop progress feedback
  - enhanced error handling to publish failure state/messages for UI consumption
  - explicit runtime `SATURN_*` environment variables now override `/etc/default/saturn-provision` values
  - added default-on provisioning toggles for remote/hardware access:
    - `SATURN_ENABLE_I2C=1`
    - `SATURN_ENABLE_SSH=1`
    - `SATURN_ENABLE_VNC=1`
  - provisioning now attempts to enable I2C/SSH/VNC during first boot
    - prefers `raspi-config` non-interactive actions on Raspberry Pi OS images
    - falls back to service/boot-config handling when `raspi-config` is unavailable
  - added LCD boot-profile support for mixed hardware variants:
    - `SATURN_LCD_PROFILE=none|cm4-7|cm4-8|cm5-7|cm5-8|auto`
    - default LCD profile mode is now `auto`
    - optional `SATURN_LCD_SIZE_INCH=7|8` explicit override for `auto` resolution
    - optional `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH=7|8` fallback for ambiguous `auto` detection
    - optional `SATURN_LCD_I2C_DETECT_ADDR` for custom panel-detect I2C address (default `0x45`)
    - `auto` mode now detects size in order: env override -> existing config overlay -> I2C probe
    - writes/replaces a managed LCD block in `config.txt` instead of replacing the whole file
    - preserves existing HDMI-related lines outside the managed block

### Documentation

- `README.md`
  - documented optional desktop C++/GTK provisioning UI and usage flags
  - documented LCD profile mapping and `SATURN_LCD_*` controls
- `cloud-init/user-data.example.yaml`
  - added desktop provisioning UI example settings (`SATURN_DESKTOP_UI`, `SATURN_UI_TIMEOUT_SECONDS`, `SATURN_UI_SHOW_LOG_DEFAULT`)
  - added default `SATURN_ENABLE_I2C`, `SATURN_ENABLE_SSH`, and `SATURN_ENABLE_VNC` toggles
  - added LCD profile settings examples (`SATURN_LCD_PROFILE`, `SATURN_LCD_SIZE_INCH`, `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH`, `SATURN_LCD_I2C_DETECT_ADDR`)

## [2026-02-28]

### Changed

- `cloud-init/provision-saturn.sh`
  - run application/tool builds as `SATURN_USER` instead of root to avoid git safe-directory warnings
  - skip `P1_app` build in provisioning flow (not required for current images)
  - wait/retry every 5 minutes until `SATURN_USER` exists
  - make optional tool build failures non-fatal when `SATURN_BUILD_OPTIONAL_TOOLS=1`

### Documentation

- `README.md`
  - documented `P1_app` skip behavior and root-only cloud-init user-data access
  - documented `cloud-init clean --logs` before image capture and unique `instance-id` guidance
- `cloud-init/meta-data.example.yaml`
  - clarified `instance-id` should be unique per image/seed

## [2026-02-15]

### Added

- Cloud-init provisioning script:
  - `cloud-init/provision-saturn.sh`
- Cloud-init example files:
  - `cloud-init/user-data.example.yaml`
  - `cloud-init/meta-data.example.yaml`
- Provisioning documentation:
  - `README.md`
  - `CHANGELOG.md`

### Changed

- Provisioning flow now includes repo-clean protections for Python:
  - disables repo bytecode writes with `PYTHONDONTWRITEBYTECODE=1`
  - uses `PYTHONPYCACHEPREFIX=/var/cache/saturn-python`
  - blocks Python script execution from repo tree during provisioning
  - cleans `__pycache__`, `*.pyc`, `*.pyo` from repo before completion
