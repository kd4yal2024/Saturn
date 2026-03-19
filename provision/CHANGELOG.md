# Changelog

All notable changes to provisioning assets are documented in this file.

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
