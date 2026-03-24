# p2app-control

Small GTK desktop control widget for starting/stopping the system `p2app.service`
(P2_app for ANAN G2).

This tool supports:
- window mode (`Start`, `Stop`, `Restart`, `Enable Boot`, `Disable Boot` + status)
- tray mode (`--tray`) using Ayatana AppIndicator (works with modern Wayland/X11 panels)

It also installs/updates the systemd service and configures tray autostart.

Location in repo:
`Saturn/sw_tools/p2app-control`

---

## What it installs / updates

### Binary
- Installs the widget binary to:
  - `/usr/local/bin/p2app-control`

### Legacy desktop shortcut
- Older installs may have:
  - `~/Desktop/P2_app-Control.desktop`
  - `~/.local/share/applications/P2_app-Control.desktop`
  - `~/Desktop/P2app.desktop`
  - `~/.local/share/applications/P2app.desktop`
- `install.sh` now removes these legacy shortcuts automatically.

### Tray autostart
- Installs a tray autostart entry by default:
  - `~/.config/autostart/P2_app-Control-tray.desktop`
- Autostart command:
  - `/usr/local/bin/p2app-control --tray`

### systemd service (system-level)
- Ensures the service exists and matches the current template:
  - `/etc/systemd/system/p2app.service`
- Enables it at boot and starts/restarts it:
  - `systemctl enable p2app.service`
  - `systemctl start|restart p2app.service`
- Waits up to `P2APP_XDMA_WAIT_SECONDS` seconds (default `20`) for
  `/dev/xdma0_user` before launching `p2app`
- Waits up to `P2APP_START_TIMEOUT_SECONDS` seconds (default `30`) for the
  service to reach `active/running` during install

The service runs `P2_app` as root using the repo build:
- Working directory:
  - `/home/pi/github/Saturn/sw_projects/P2_app`
- ExecStart:
  - `/home/pi/github/Saturn/sw_projects/P2_app/p2app -s -p`

### Scoped privilege rules (no password prompts)
To allow the desktop user to control `p2app.service` without password prompts,
the installer adds:
- `/etc/polkit-1/rules.d/49-p2app.rules`
- `/etc/sudoers.d/49-p2app-control`

Scope details:
- polkit rule: installing desktop user (normally `pi`), local + active session only, unit `p2app.service`, verbs `start` / `stop` / `restart`
- sudoers rule: installing desktop user (normally `pi`), exact commands for `systemctl start|stop|restart|enable|disable p2app.service`

---

## Requirements

Packages needed to build/run the widget:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libgtk-3-dev \
  libayatana-appindicator3-dev ayatana-indicator-application
```

`sudo` is required. The installer adds a scoped sudoers entry for the allowed
`systemctl` verbs above, and older installs may also use the bundled polkit
rule as a fallback for live start/stop/restart control.

---

## Build

From the `p2app-control` directory:

```bash
make
```

The resulting binary will be:

```bash
./p2app-control
```

---

## Install (recommended)

This will:

* build the widget
* install `/usr/local/bin/p2app-control`
* create/update `/etc/systemd/system/p2app.service`
* install scoped privilege rules for service control
* enable + start/restart the service
* remove legacy desktop shortcut files (if present)
* install tray autostart (unless `ENABLE_TRAY_AUTOSTART=0`)

Run:

```bash
chmod +x install.sh
./install.sh
```

Disable tray autostart install:

```bash
ENABLE_TRAY_AUTOSTART=0 ./install.sh
```

You will be prompted for sudo once because the installer writes to `/etc/`.

---

## Usage

Run from a terminal:

```bash
/usr/local/bin/p2app-control
```

Run as panel/toolbar tray widget:

```bash
/usr/local/bin/p2app-control --tray
```

On Wayland desktops (for example labwc + wf-panel), the tray icon is provided
through AppIndicator support instead of deprecated `GtkStatusIcon`.
On Raspberry Pi OS Bookworm/Trixie with `wf-panel-pi`, this appears in the
panel tray/status area on the right side of the top panel, not in the launcher
strip on the left.
If the panel does not render the icon, the process may still be running and
controllable from the menu; launch `/usr/local/bin/p2app-control --window` as
an explicit fallback UI.

To check service state manually:

```bash
systemctl status p2app.service --no-pager
journalctl -u p2app.service -n 100 --no-pager
```

---

## Notes / Troubleshooting

### “Cannot open display” (SSH)

The GUI widget must be run inside the graphical session. Running it from a plain
SSH shell will not work unless you set up Wayland/X forwarding.

### Service binary not found

If `install.sh` errors because it cannot find:

`/home/pi/github/Saturn/sw_projects/P2_app/p2app`

build/install P2_app first, or adjust `P2APP_DIR` / `P2APP_BIN` inside
`install.sh`.

### `register write attempted before XDMA register device was opened`

This message means `p2app` tried to touch Saturn registers before
`/dev/xdma0_user` was available.

`install.sh` now waits for `/dev/xdma0_user` before launching `p2app` and
does not fail provisioning just because the XDMA register device has not
enumerated yet. If the device node still never appears, the service will keep
retrying and the installer will log that the problem is FPGA/XDMA device
enumeration rather than widget installation.

Useful checks:

```bash
ls -l /dev/xdma0_user /dev/xdma/card0 2>/dev/null
systemctl status p2app.service --no-pager
journalctl -u p2app.service -n 100 --no-pager
sudo bash /home/pi/github/Saturn/scripts/fix-xdma.sh
```

### Removing

* Remove launcher(s):

  * `rm -f ~/.config/autostart/P2_app-Control-tray.desktop`
* Remove binary:

  * `sudo rm -f /usr/local/bin/p2app-control`
* Remove privilege rules:

  * `sudo rm -f /etc/polkit-1/rules.d/49-p2app.rules /etc/sudoers.d/49-p2app-control && sudo systemctl restart polkit`
* Remove/disable service:

  * `sudo systemctl disable --now p2app.service`
  * `sudo rm -f /etc/systemd/system/p2app.service && sudo systemctl daemon-reload`
