# Saturn Provisioning

This directory contains the shared provisioning engine and the thin cloud-init
bootstrap for Saturn setup. Manual and cloud-init installs both enter through
the repository's `install.sh`; cloud-init does not maintain a second appliance
installation sequence.

## Current Layout

- `../install.sh` (public canonical entry point)
- `../scripts/install-saturn-appliance.sh` (profile/CLI wrapper)
- `cloud-init/user-data.example.yaml`
- `cloud-init/meta-data.example.yaml`
- `cloud-init/provision-saturn.sh`
- `cloud-init/pihpsdr-installer-run.sh`
- `cloud-init/pihpsdr-installer-ui.cpp`
- `cloud-init/saturn-provision-powerctl.sh`
- `cloud-init/saturn-provision-ui.cpp`
- `LCD_DECISION_MATRIX.md`

## Supported Target and Installation Modes

The supported appliance target is Debian 13 / Raspberry Pi OS Trixie on
`arm64`/`aarch64`, with `apt`, systemd, and a normal non-root login user. The
canonical installer rejects other architectures and OS codenames. Setting
`SATURN_ALLOW_UNSUPPORTED_OS=1` bypasses only the codename check; it does not
make another distribution supported.

Choose the profile that matches where the install is running:

| Profile | Intended use | Optional desktop developer bundle | Builds piHPSDR during provisioning | Desktop provisioning UI | Default verification |
| --- | --- | --- | --- | --- | --- |
| `appliance` | A Saturn/G2 connected to its FPGA/XDMA hardware | No | No; a separate installer remains available | `auto` | `hardware` |
| `desktop` | An interactive development/operator workstation attached to Saturn hardware | Yes, when packages are available | Yes | `auto` | `hardware` |
| `image-factory` | Building a reusable image away from radio hardware | No | No | Off | `software` |

The `appliance` profile is the default. Hardware verification expects a real
XDMA device, a running P2 service, a healthy Saturn Go service, and a running
Saturn Bridge listener. It will fail by design on an off-hardware image. Use
`image-factory` or explicitly select `--verify software` for that case.

Verification modes are deliberate deployment choices:

- `hardware` checks installed artifacts/units plus the live XDMA character
  device, active P2/Saturn Go/Bridge services, Saturn Go `/readyz`, and the
  Bridge TCP listener on port 50001.
- `software` checks DKMS registration, installed artifacts/configuration, and
  enabled units but skips live hardware and active-runtime requirements.
- `none` skips final verification and should be reserved for diagnosis or a
  deliberately partial install; it is not proof of a working appliance.

## What the End User Should Expect

A first install is not a small package update. It can download hundreds of
packages, prepare a Python virtual environment, install a Rust toolchain, and
compile native, kernel, Rust, and web components. Plan for a stable network,
several gigabytes of free space, continuous power, and roughly 30-60 minutes;
slow storage or first-time Rust downloads can take longer. The installer does
not currently enforce a whole-filesystem free-space minimum.

During a normal first run:

1. The wrapper validates the profile, target user, architecture, OS, and basic host tools.
2. Packages and matching kernel headers are installed non-interactively.
3. The optional GTK status window is built and launched if a desktop session is available.
4. USB, I2C, SSH, VNC, front-panel, system-role, and LCD configuration is applied.
5. The repository/Python environment is prepared, Saturn applications and
   tools are tested/built, and desktop launchers are installed.
6. The shutdown waiter and red-running/white-shutdown power LED behavior are configured.
7. XDMA is built through DKMS, loaded, and configured for future boots.
8. The P2 runtime and control tool are installed and started.
9. Saturn Go, nginx, synchronized admin/Remote credentials, Saturn Bridge, and
   the optional piHPSDR runtime/installer shortcut are installed; FPGA flashing
   follows only when explicitly enabled and confirmed.
10. Verification runs, completion/profile files are written, temporary files
    are cleaned, and a reboot is recommended.

Compiler output is intentionally verbose. Warnings from optional tools do not
necessarily mean provisioning failed; the authoritative result is the final
`Saturn provisioning completed successfully` message and
`/var/lib/saturn-provision/complete`. Any required command failure sets the UI
state to `FAILED`, records the failing line/command, and exits nonzero.

### Password and login behavior

The Linux login password and the Saturn web password are separate credentials.
On a new interactive install, the terminal prompts late in the Saturn Go phase:

```text
Choose a Saturn password (at least five characters), or press Enter to generate one:
```

The value must contain at least five characters. Pressing Enter, using
`--non-interactive`, or running through cloud-init generates a device-specific
value. It is not printed into the general provisioning log; retrieve it locally
as root:

```bash
sudo cat /var/lib/saturn-provision/update-manager-admin-password
```

The username is `admin`. The same managed credential is applied to nginx's
Saturn Go login and the Saturn Remote TLS listener. A rerun preserves existing
synchronized credentials instead of resetting them. The root-only plaintext
file is an initial/recovery convenience and may not exist on older upgraded
systems whose credential is already synchronized.

### Reboot expectation

Reboot after a successful first install before operating the radio. Boot
configuration, LCD overlays, early LED state, kernel module loading, user group
membership, autostart launchers, and enabled services are most accurately
tested after a fresh boot. The GTK window offers `Reboot Now`; a terminal-only
install can use:

```bash
sudo systemctl reboot
```

## What It Does

`cloud-init/provision-saturn.sh` runs as root and:

- installs required apt packages
- retries transient apt lock conflicts during package index/install operations instead of failing immediately
- installs matching Raspberry Pi kernel headers when available in apt sources
- with the `desktop` profile, installs developer tools when available:
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
- uses the exact repository checkout selected by `install.sh`; cloud-init
  fetches the requested `SATURN_REPO_REF` once before invoking it
- builds Saturn apps and tools (including `sw_tools`)
- installs desktop launchers (deprecated `P2app.desktop` is removed and not reinstalled)
- installs the supported XDMA source through DKMS
- leaves `scripts/fix-xdma.sh` in the selected checkout for field recovery
- keeps the legacy XDMA kernel post-install hook disabled whenever DKMS is registered
- optionally installs udev rules
- configures the power-switch/front-panel LED helper so BCM15 is driven red during normal operation and white on shutdown
- installs a root-owned provisioning power helper so the desktop UI can request reboot reliably at the end of first-boot setup
- optionally installs `p2app-control` tray control (AppIndicator-based)
- when the detected front-panel state is `NONE`, installs P2 with panel mode
  off; the radio runtime remains installed and supported
- installs the shutdown waiter in `auto` mode: detected G2V1 hardware uses its
  own I2C/front-panel shutdown path, while other/no-panel configurations use
  the guarded GPIO26 waiter
- optionally installs Update Manager
- installs Saturn Bridge with its pinned WDSP 2.00 source by default; piHPSDR is an optional desktop application and is no longer a bridge prerequisite
- installs piHPSDR native build dependencies by default for the standalone desktop and Update Manager installers
- optionally flashes FPGA (disabled by default)
- resumes completed package, build, DKMS, P2, and Saturn Go phases after an interruption when the install contract is unchanged
- performs hardware verification on radios or software-only verification for image factories

Completion and logs:

- state file: `/var/lib/saturn-provision/complete`
- provisioning profile file: `/var/lib/saturn-provision/profile.env`
- front-panel state file: `/var/lib/saturn-provision/front-panel-type`
- system-role state file: `/var/lib/saturn-provision/system-role`
- log file: `/var/log/saturn-provision.log`
- live status file (desktop UI): `/var/lib/saturn-provision/ui-status`
- completion state now also records:
  - `installer_version`
  - `install_contract`
  - `install_profile`
  - `verification_mode`
  - `repo_ref`
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
- elapsed time (the installer does not calculate an ETA)
- toggleable live log panel
- final success/failure state

Environment controls:

- `SATURN_DESKTOP_UI=auto|1|0` (default: `auto`)
  - `auto`: launches when a usable desktop display can be discovered
  - `1`: force attempt to launch UI
  - `0`: disable UI
- `SATURN_UI_TIMEOUT_SECONDS` (default: `2700`; compatibility setting passed
  to the UI, not an installer deadline or ETA)
- `SATURN_UI_SHOW_LOG_DEFAULT=1|0` (default: `0`)
- `SATURN_UI_BINARY` (default: `/usr/local/bin/saturn-provision-ui`)
- `SATURN_UI_STATUS_FILE` (default: `/var/lib/saturn-provision/ui-status`)
- `SATURN_CLEAN_TMP_AFTER_PROVISION=1|0` (default: `1`)
- `SATURN_APT_LOCK_TIMEOUT_SECONDS` (default: `120`)
- `SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS` (default: `3`)
- `SATURN_INSTALL_PIHPSDR=1|0` (default: `0`; optional desktop application)
- `SATURN_INSTALL_SATURN_BRIDGE=1|0` (default: `1`)
- `SATURN_REQUIRE_SATURN_BRIDGE=1|0` (default: `1`; when enabled, provisioning fails instead of silently completing without the Remote backend)
- `SATURN_DETECT_FRONT_PANEL=1|0` (default: `1`)
- `SATURN_FRONT_PANEL_STATE_FILE` (default: `/var/lib/saturn-provision/front-panel-type`)
- `SATURN_FORCE_SYSTEM_ROLE=local_saturn|remotehead_candidate|unknown` (optional conservative support override)
- `SATURN_PROFILE_ENV_FILE` (default: `/var/lib/saturn-provision/profile.env`)

Notes:

- Cloud-init boots without a desktop session in many images; in that case
  provisioning continues in the log and terminal. The autostart entry can show
  the UI if a desktop login occurs while provisioning is still active; a
  successful headless run removes the entry, so it does not display a stale
  completion window at a later login.
- On a fresh OS, the package-install phase runs before GTK prerequisites exist.
  The UI is built and launched immediately after that phase, so watch the
  terminal or log for the initial package output.
- Provisioning now installs a per-user autostart entry at `~/.config/autostart/saturn-provision-ui.desktop` for `SATURN_USER` (default `pi`), so the widget appears when that desktop session starts.
- Provisioning also installs `/usr/local/sbin/saturn-provision-powerctl` plus a restricted sudoers entry so the UI `Reboot Now` action does not depend on an interactive polkit prompt or TTY-bound `systemctl` authorization.
- Once launched, the provisioning UI remains open after completion until the
  user clicks `Close` or chooses `Reboot Now`.
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
- Because the standalone installer is enabled by default, provisioning also installs the piHPSDR native build dependencies; the unprivileged installer does not need an interactive sudo prompt.
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
- Re-running provisioning replaces that managed block. Before writing it, the
  helper also removes matching Waveshare DSI panel overlay lines outside the
  block so duplicate/competing panel overlays are not left active.
- Existing HDMI lines outside the managed block are left untouched.
- `SATURN_LCD_PROFILE=none` is a no-op. It does not remove a managed LCD block
  left by an earlier run; use Saturn LCD Setup to change or recover a profile.
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
- Provisioning sets `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH=7`, so a fresh CM4 with
  no conclusive overlay or I2C response falls back to the generic 7-inch
  profile. Set an explicit profile or size before installation when that is
  not the attached display.
- `SATURN_LCD_PROFILE=auto` currently applies Raspberry Pi CM4/CM5 overlay rules only; on non-Raspberry-Pi platforms such as Radxa it will log the detected model/vendor and require an explicit profile.
- After `cm` and size are known, the 7-inch front-panel tiebreaker now separates G2 variants:
  - `CM4 + G2V1/G2V2` -> `${cm}-7-g2-single-dsi`
- First provisioning now installs udev rules and detects the front panel before applying the LCD profile, so the G2 7-inch tiebreaker is active on the first run instead of only on reprovision/helper use.
- Front-panel detection in Saturn is now intentionally hardware-only:
  - state file values are `G2V1`, `G2V2`, or `NONE`
  - `ZZZS08...` is currently treated as `G2V2`-class hardware for LCD/provisioning purposes
  - a future `RemoteHead` concept should be modeled separately as a system role, not as a front-panel type
- `NONE` is the expected front-panel result when no Saturn front panel is
  installed. It does not mean an independently installed Waveshare LCD is
  missing or unsupported. Existing recognized 7-inch overlay configuration is
  preserved independently of the front-panel result.
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
- A fresh/unrecognized `CM5 + 7"` configuration is currently explicit/manual-only in `SATURN_LCD_PROFILE=auto`
  - explicit profiles `cm5-7`, `cm5-7-g2-single-dsi`, and `cm5-7-g2-dual-dsi` are still available for Saturn LCD Setup and manual testing
  - auto mode preserves a recognized existing CM5 7-inch managed, single-DSI,
    or dual-DSI profile, but refuses to invent one when no recognized config
    exists
- Lightweight detection that does not run provisioning or change boot config:
  - `cd /path/to/Saturn && sudo scripts/detect-lcd-profile.sh`
- `SATURN_LCD_DETECT_ONLY=1` prevents LCD writes only. Using it with
  `./install.sh --force` still reruns every other provisioning phase and is not
  a lightweight LCD detector.

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
  - `scripts/saturn-lcd-helper.sh backups`
- restore latest backup:
  - `sudo scripts/saturn-lcd-helper.sh restore-latest`

## Manual Installation

Start with a 64-bit Debian 13 or Raspberry Pi OS Trixie installation. The
installer must run on the Saturn itself, not on the computer used to SSH into
it.

Requirements:

- an `arm64`/`aarch64` host using systemd and `apt`
- a normal non-root runtime user with a home directory
- root access through `sudo`
- stable Internet access and power for package, Git, Rust, and npm downloads
- several gigabytes of free space
- writable boot files (`/boot/firmware/...` or `/boot/...`) for LCD, USB, and
  kernel-command-line changes
- Saturn/FPGA hardware present when using the default hardware verification

Before starting, decide whether to choose the Saturn web password or let the
installer generate it. The prompt accepts **five or more characters**; a
shorter new value stops provisioning. Pressing Enter generates a
five-character value. This does not change the Linux login password.

```bash
cd ~
sudo apt-get update
sudo apt-get install -y git
mkdir -p github
cd github
git clone https://github.com/kd4yal2024/Saturn.git
cd Saturn
./install.sh --dry-run --user "$USER"
sudo ./install.sh --user "$USER"
```

Clone as the normal user, not with `sudo`, so updates and builds remain
user-owned. Also clone from the home directory (or another writable directory):
running `git clone` while in `/var`, `/var/log`, or `/` normally fails with
`Permission denied`.

The dry run prints the resolved repository, user, profile, feature choices, and
verification mode without changing the host. Remove it only after confirming
those values. If the runtime account is actually `pi`, `sudo ./install.sh`
selects it automatically; `--user` is clearer on systems with another account.

### Installer options

| Option | Effect |
| --- | --- |
| `--user NAME` | Runtime/build account; defaults to `SUDO_USER`, then `pi` |
| `--profile appliance\|desktop\|image-factory` | Select the defaults described above |
| `--non-interactive` | Never prompt and generate the five-character Saturn password |
| `--force` | Ignore a matching completion record and rerun all phases for the current contract |
| `--verify hardware\|software\|none` | Select final verification strength |
| `--skip-packages` | Do not run apt; all dependencies must already exist |
| `--skip-driver` | Do not install or load XDMA through DKMS |
| `--skip-p2` | Do not build/install `p2app.service` |
| `--skip-saturn-go` | Do not install Saturn Go or Saturn Bridge |
| `--skip-verify` | Alias for `--verify none` |
| `--dry-run` | Print the resolved install contract without changing the host |

Examples:

- `sudo ./install.sh --profile desktop`
- `sudo ./install.sh --profile image-factory`
- `sudo SATURN_LCD_PROFILE=none ./install.sh` when intentionally leaving boot
  LCD configuration untouched
- `sudo SATURN_LCD_PROFILE=cm4-8 ./install.sh` when the automatic choice is not
  appropriate

Do not use skip flags merely to work around a failure. They deliberately omit
parts of the appliance and can also make hardware verification fail. The
normal recovery action is to fix the reported cause and rerun the same command.

### Interruption, rerun, and update behavior

- A matching completed install exits successfully as `SKIPPED`; it does not
  rebuild an already completed appliance.
- A failed install records checkpoints under
  `/var/lib/saturn-provision/phases/`. Rerunning the same command resumes
  completed expensive phases and reruns the failed phase from its beginning.
- The completion contract includes the installer schema, repository commit,
  kernel, profile, runtime user/path, verification mode, and material feature
  options. A changed contract reruns affected work and reuses only compatible
  checkpoints.
- `--force` bypasses the completion record and clears phase checkpoints for the
  current contract. It is intended for a deliberate full reprovision, not the
  first response to a normal failure.
- The installer uses the exact local checkout. It does not fetch or switch the
  manual installation behind the operator's back. Update the checkout
  intentionally before running it when a newer revision is wanted.
- Provisioning does not uninstall older manually installed software or restore
  arbitrary local boot/service changes. Back up a production system before a
  major migration.

### Successful-install checklist

After the final success message:

1. Record the Saturn credential path and reboot:

   ```bash
   sudo cat /var/lib/saturn-provision/update-manager-admin-password
   sudo systemctl reboot
   ```

2. After reconnecting, confirm the recorded profile and completion contract:

   ```bash
   sudo cat /var/lib/saturn-provision/complete
   sudo cat /var/lib/saturn-provision/profile.env
   sudo cat /var/lib/saturn-provision/front-panel-type
   ```

3. On a hardware appliance, check the driver, services, devices, and web health:

   ```bash
   sudo dkms status -m saturn-xdma
   sudo systemctl --no-pager --full status saturn-xdma-ready.service p2app.service saturn-go.service saturn-bridge.service
   ls -l /dev/xdma0_user /dev/xdma/card0/user 2>/dev/null
   curl -fsS http://127.0.0.1:8080/readyz
   ```

4. From another LAN computer, open `http://<saturn-ip>/saturn/` and sign in as
   `admin`. Saturn Remote is available at
   `https://<saturn-ip>:8443/remote-next`; a newly generated local certificate
   may require an initial browser trust exception.

5. To change the Saturn password in the current UI, open **System → Custom
   Scripts**, scroll to the output card, and select **Change Password**. The
   control requires at least five characters and updates both the
   nginx Saturn Go login and Saturn Remote credential together. The deferred
   Saturn Go restart can end the current session, so sign in again with the new
   value. This location is easy to overlook; it is the current location, not a
   recommendation about future UI organization.

6. Verify the installed LCD after reboot. A saved front-panel value of `NONE`
   and a missing `g2-front-9600` serial alias are normal when no Saturn front
   panel is fitted; they do not indicate a problem with a separate Waveshare
   LCD.

## Raspberry Pi Imager Workflow (End-to-End)

This section describes the complete workflow for building an SD card with Raspberry Pi Imager and having Saturn provision automatically on first boot.

### 1. Choose an OS image that supports cloud-init

- Use a 64-bit Debian 13 or Raspberry Pi OS Trixie image with cloud-init
  installed and enabled. Both conditions matter: an Ubuntu cloud-init image is
  not a supported Saturn appliance target merely because it includes
  cloud-init.
- Confirm that the image uses systemd, `apt`, and the `arm64` architecture.
- If cloud-init is absent or disabled, use the manual workflow; placing seed
  files on the boot partition alone will not make provisioning run.

### 2. Prepare cloud-init inputs from this repo

From this repo:

- `provision/cloud-init/user-data.example.yaml`
- `provision/cloud-init/meta-data.example.yaml`

Copy and customize as needed:

- set `SATURN_USER` to the actual login user that will exist on the target image
- set `SATURN_REPO_REF` to a release tag or tested commit for a reproducible image
- leave `SATURN_ADMIN_PASSWORD` blank to generate a unique five-character
  value, or set at least five characters; it is the Saturn `admin` web credential, not the
  Linux account password
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

Raspberry Pi Imager behavior varies by image. Verify the actual `user-data`
written to the card instead of assuming Imager merged its user/network settings
with this example. The final file must contain both the required login-user
creation and the Saturn `write_files`/`runcmd` content.

### 5. First boot execution flow

On first boot, cloud-init processes `user-data` and executes:

- writes `/etc/default/saturn-provision`
- writes `/usr/local/sbin/saturn-cloudinit-bootstrap.sh`
- logs early bootstrap work to `/var/log/saturn-cloudinit-bootstrap.log`
- initializes the configured Saturn checkout for `SATURN_USER`
- fetches and checks out `SATURN_REPO_REF` exactly once
- runs:
  - `bash "$SATURN_REPO_DIR/install.sh" --user "$SATURN_USER" --profile "$SATURN_INSTALL_PROFILE" --non-interactive`

Bootstrap behavior before the main Saturn script now is:

- waits for system clock synchronization before first apt/git activity
- installs bootstrap prerequisites itself after the clock is sane
- retries while waiting for `SATURN_USER` to exist
- makes 20 attempts at `SATURN_USER_RETRY_SECONDS` intervals (about ten minutes
  with the default 30-second interval)
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
  - `NONE` and a missing front-panel serial alias are expected when no Saturn
    front panel is fitted; LCD detection/configuration still runs independently
- udev serial rule installation now receives the detected front-panel type for logging/state, but currently installs the standard `61-g2-serial.rules` file unless explicitly overridden
- LCD boot profile apply
  - uses the shared logic from `scripts/saturn-lcd-lib.sh`
  - on 7-inch systems, now separates G2 variants:
    - `CM4 + G2V1/G2V2` -> `cm4-7-g2-single-dsi`
  - on `CM5 + 7"` systems, `SATURN_LCD_PROFILE=auto` now warns and requires an explicit profile instead of pretending the path is validated
- kernel header checks/install (for XDMA build path)
- build of apps/tools from the bootstrap-selected checkout
- desktop launcher install
- XDMA DKMS build/install/load
  - DKMS rebuilds the supported driver for installed kernels
  - a successful install disables `/etc/kernel/postinst.d/saturn-xdma` so the
    legacy hook and DKMS never both own the same kernel update
  - `scripts/fix-xdma.sh` remains a field-recovery tool, not the normal lifecycle owner
- optional p2app-control install
  - installs tray autostart (`~/.config/autostart/P2_app-Control-tray.desktop`)
  - requires `libayatana-appindicator3-dev` and `ayatana-indicator-application`
  - installs `/usr/local/bin/saturn-xdma-doctor.sh` as the supported XDMA diagnostic helper
  - installs `/usr/local/bin/saturn-xdma-ready.sh` plus `saturn-xdma-ready.service` as the dedicated XDMA readiness gate
  - installs `/usr/local/bin/saturn-fix-xdma.sh` and `/usr/local/bin/saturn-xdma-kernel-postinst.sh`
  - leaves `/etc/kernel/postinst.d/saturn-xdma` disabled when DKMS is registered
  - installs the trusted production P2 deploy helper with service verification and rollback
  - generated `p2app.service` now depends on `saturn-xdma-ready.service` instead of owning the XDMA readiness loop directly
  - installer now waits for `p2app.service` to reach `active/running`
  - if the XDMA kernel module is loaded but `/dev/xdma0_user` is still missing, provisioning logs the condition and continues instead of failing the whole run at the `p2app-control` step
  - final `hardware` verification will still fail if the required XDMA device
    never appears; the earlier continuation keeps the diagnosis attached to the
    correct hardware/driver verification step
  - if a panel does not render the tray icon, run `/usr/local/bin/p2app-control --window` as fallback
- optional Update Manager install
- Saturn Go/nginx/Remote credential setup; cloud-init never prompts and writes
  the generated value root-only to
  `/var/lib/saturn-provision/update-manager-admin-password`
- Saturn Bridge install and health/listener verification
- shutdown-button waiter and BCM15 red-running/white-shutdown LED configuration
- optional FPGA flash (only if explicitly enabled and confirmed)
- profile/completion marker write only after the selected verification succeeds

### 6. Verify completion

`cloud-init`'s final message means its command returned; the Saturn completion
file is the authoritative success signal. After the first boot, verify:

- `cloud-init status --wait`
- `sudo tail -n 200 /var/log/saturn-cloudinit-bootstrap.log`
- `sudo cat /var/lib/saturn-provision/complete`
- `sudo cat /var/lib/saturn-provision/profile.env`
- `sudo cat /var/lib/saturn-provision/front-panel-type`
- `sudo tail -n 200 /var/log/saturn-provision.log`
- `sudo dkms status -m saturn-xdma`
- `sudo systemctl status saturn-xdma-ready.service p2app.service --no-pager`
- `ls -l /dev/xdma0_user /dev/xdma/card0/user 2>/dev/null`

If Update Manager is enabled, also verify service status:

- `sudo systemctl status saturn-go.service saturn-bridge.service --no-pager`
- `curl -fsS http://127.0.0.1:8080/readyz`

Retrieve the generated password with `sudo cat
/var/lib/saturn-provision/update-manager-admin-password`, then reboot once and
repeat the hardware/service checks.

### 7. Re-run behavior

- Provisioning is idempotent by an install-contract marker:
  - it exits cleanly only when the host schema, repository commit, kernel,
    profile, user/path, and material feature options still match
  - a changed contract reuses only compatible completed phase checkpoints
- A failed first boot does not cause cloud-init `runcmd` to run on every later
  boot. After correcting the cause, rerun the canonical installer from the
  checkout:
  - `cd /home/<user>/github/Saturn && sudo ./install.sh --user <user> --profile appliance --non-interactive`
- To intentionally discard reusable phase checkpoints for the current
  contract, add `--force`.
- `SATURN_FORCE_REPROVISION=1` forces a full run when the bootstrap/installer
  is invoked; it does not change cloud-init's once-per-instance execution.
- Keep the `meta-data` `instance-id` unique for each new seed/image instance.
  Changing it on an already deployed appliance causes cloud-init to treat the
  seed as a new instance and should be done only deliberately.

## Cloud-Init Inputs

`cloud-init/user-data.example.yaml` writes `/etc/default/saturn-provision` and
installs `/usr/local/sbin/saturn-cloudinit-bootstrap.sh`. The `runcmd` invokes
that helper once for the cloud-init instance. After resolving the user, it
fetches `SATURN_REPO_REF` at depth one, checks out the fetched commit detached,
and executes:

- `bash "$SATURN_REPO_DIR/install.sh" --user "$SATURN_USER" --profile "$SATURN_INSTALL_PROFILE" --non-interactive`

Early bootstrap activity is recorded in
`/var/log/saturn-cloudinit-bootstrap.log`; the canonical install records its
work separately in `/var/log/saturn-provision.log`.

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

These are the effective defaults in the example appliance seed and canonical
installer. Explicit `SATURN_*` variables supplied to the process take
precedence over `/etc/default/saturn-provision`.

- `SATURN_USER=pi`
- `SATURN_REPO_REF=main` (convenient but not reproducible; use a tested tag or
  full commit for production images)
- `SATURN_INSTALL_PROFILE=appliance`
- `SATURN_USER_RETRY_SECONDS=30`
- `SATURN_CLOCK_SYNC_WAIT_SECONDS=180`
- `SATURN_CLOCK_SYNC_POLL_SECONDS=5`
- `SATURN_INSTALL_PACKAGES=1`
- `SATURN_INSTALL_UPDATE_MANAGER=1`
- `SATURN_INSTALL_PIHPSDR=0` (`desktop` profile changes this to `1`)
- `SATURN_PIHPSDR_INSTALLER_ENABLED=1`
- `SATURN_INSTALL_SATURN_BRIDGE=1`
- `SATURN_REQUIRE_SATURN_BRIDGE=1`
- `SATURN_INSTALL_CLOUD_INIT=0` (automatically `1` for the `image-factory` profile)
- `SATURN_INSTALL_P2APP_CONTROL=1`
- `SATURN_RUN_P2_TESTS=1`
- `SATURN_INSTALL_UDEV_RULES=1`
- `SATURN_INSTALL_SHUTDOWN_WAITER=1`
- `SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT=auto`
- `SATURN_REBUILD_XDMA=1`
- `SATURN_BUILD_OPTIONAL_TOOLS=1`
- `SATURN_INSTALL_DEVELOPER_TOOLS=0` in the appliance example
- `SATURN_DETECT_FRONT_PANEL=1`
- `SATURN_ENABLE_I2C=1`
- `SATURN_ENABLE_SSH=1`
- `SATURN_ENABLE_VNC=1`
- `SATURN_LCD_PROFILE=auto`
- `SATURN_LCD_AUTO_DEFAULT_SIZE_INCH=7`
- `SATURN_DESKTOP_UI=auto`
- `SATURN_UI_TIMEOUT_SECONDS=2700` (not an install timeout)
- `SATURN_UI_SHOW_LOG_DEFAULT=0`
- `SATURN_CLEAN_TMP_AFTER_PROVISION=1`
- `SATURN_RESUME=1`
- `SATURN_VERIFY_MODE=hardware` (`image-factory` defaults to `software`)
- `SATURN_FORCE_REPROVISION=0`
- `SATURN_FLASH_FPGA=0` (safety default)
- `SATURN_APT_LOCK_TIMEOUT_SECONDS=120`
- `SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS=3`
- `SATURN_ADMIN_PASSWORD=` generates a unique five-character initial password
- newly supplied passwords must contain at least five characters, with no
  composition rules; generated passwords contain five characters and existing
  synchronized credentials are preserved during upgrades

Important safety settings for flashing:

- `SATURN_FLASH_FPGA=0` keeps flashing disabled by default
- If set to `1`, `SATURN_FLASH_CONFIRM` is required
- Optional fallback flashing is controlled by `SATURN_FLASH_FALLBACK`

## Failure Diagnosis and Recovery

Start with the final error and the saved phase; do not delete the checkout or
manually reinstall every dependency. These commands are read-only:

```bash
sudo cat /var/lib/saturn-provision/current-phase 2>/dev/null
sudo cat /var/lib/saturn-provision/ui-status 2>/dev/null
sudo tail -n 200 /var/log/saturn-cloudinit-bootstrap.log 2>/dev/null
sudo tail -n 300 /var/log/saturn-provision.log
sudo journalctl -u saturn-xdma-ready -u p2app -u saturn-go -u saturn-bridge -n 150 --no-pager
```

For XDMA/DKMS failures, inspect both registration and the compiler's actual
error:

```bash
uname -r
sudo dkms status -m saturn-xdma
sudo find /var/lib/dkms/saturn-xdma -type f -name make.log -print
sudo find /var/lib/dkms/saturn-xdma -type f -name make.log -exec tail -n 200 {} \;
```

`The kernel is built without module signing facility` is normally
informational on this Raspberry Pi kernel; a later `Bad return status` and the
contents of `make.log` determine whether the build failed.

Common interpretations:

- `install.sh: No such file or directory` means the selected Git ref does not
  contain the canonical installer. Check `SATURN_REPO_REF` and the checkout
  before rerunning.
- `could not create work tree dir ... Permission denied` means `git clone` was
  started in an unwritable directory. Change to the login user's home and
  clone there.
- `Front panel: NONE` or a missing
  `/dev/serial/by-id/g2-front-9600` is normal when the appliance has no Saturn
  front panel.
- A CM5 7-inch auto-selection warning means no recognized existing profile was
  safe to preserve. Select one with Saturn LCD Setup; do not guess from the
  front-panel result.
- No GTK window during early package installation or headless cloud-init is
  normal. Follow the log with `sudo tail -f /var/log/saturn-provision.log`.
  (`tail -f FILE` is the command; `cat tail -f FILE` is not.)
- A password-length failure occurs before Saturn Go deployment when a newly
  supplied value is shorter than five characters. Rerun and enter at least five
  characters, press Enter to generate one, or leave
  `SATURN_ADMIN_PASSWORD=` blank for unattended provisioning.

After fixing the cause, rerun the same canonical install command without
`--force`; resumable checkpoints avoid repeating completed expensive phases.
Use `--force` only when a complete from-scratch reprovision is intentional.

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
- Provisioning retries user lookup for a bounded period and then fails clearly.
- Early clone/bootstrap failures are logged to `/var/log/saturn-cloudinit-bootstrap.log`.
- `P1_app` is intentionally skipped in provisioning (legacy target not required for current images).
- Before capturing a reusable fully provisioned image, run
  `sudo scripts/seal-saturn-image.sh --confirm SEAL`. It removes machine, SSH,
  Tailscale, Saturn Remote TLS/cookie, and admin identity and powers the source
  off. It also erases builder login hashes, SSH/client credentials, and
  provisioning logs and operator-specific Remote profiles/settings. Each clone
  creates a unique hostname (unless customized), five-character Saturn Go login,
  and new TLS certificate on its first boot. A Linux password supplied by
  Raspberry Pi Imager or cloud-init is preserved; an otherwise locked local
  account is unlocked with the generated five-character value.
- When customizing a sealed image in Raspberry Pi Imager, keep the provisioned
  Saturn username unchanged. The installed service units are deliberately tied
  to that account; network and password customization remain supported.
- Keep `meta-data` `instance-id` unique per image/seed when possible.
