# p2app-control

Small GTK desktop control widget for starting/stopping the system `p2app.service`
(P2_app for ANAN G2).

This tool supports:
- window mode (Start / Stop / Restart buttons + status)
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

The service runs `P2_app` as root using the repo build:
- Working directory:
  - `/home/pi/github/Saturn/sw_projects/P2_app`
- ExecStart:
  - `/home/pi/github/Saturn/sw_projects/P2_app/p2app -s -p`

### polkit rule (no password prompts)
To allow user `pi` to start/stop/restart *only* `p2app.service` without password
prompts, the installer adds a polkit rule:
- `/etc/polkit-1/rules.d/49-p2app.rules`

This rule is intentionally scoped to:
- user: `pi`
- local + active session only
- unit: `p2app.service`
- verbs: start / stop / restart

---

## Requirements

Packages needed to build/run the widget:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libgtk-3-dev \
  libayatana-appindicator3-dev ayatana-indicator-application
```

`pkexec` / polkit support is normally present on Raspberry Pi Desktop, but the
installer uses a direct polkit rule instead of prompting.

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
* install the polkit rule
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

### Removing

* Remove launcher(s):

  * `rm -f ~/.config/autostart/P2_app-Control-tray.desktop`
* Remove binary:

  * `sudo rm -f /usr/local/bin/p2app-control`
* Remove polkit rule:

  * `sudo rm -f /etc/polkit-1/rules.d/49-p2app.rules && sudo systemctl restart polkit`
* Remove/disable service:

  * `sudo systemctl disable --now p2app.service`
  * `sudo rm -f /etc/systemd/system/p2app.service && sudo systemctl daemon-reload`
