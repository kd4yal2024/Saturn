#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"

UNIT_NAME="p2app.service"
UNIT_PATH="/etc/systemd/system/${UNIT_NAME}"

P2APP_DIR="${REPO_ROOT}/sw_projects/P2_app"
P2APP_BIN="${P2APP_DIR}/p2app"

BIN_LOCAL="${HERE}/p2app-control"
BIN_INSTALL="/usr/local/bin/p2app-control"

DESKTOP_NAME="P2_app-Control.desktop"
DESKTOP_DESK="${HOME}/Desktop/${DESKTOP_NAME}"
DESKTOP_APPS="${HOME}/.local/share/applications/${DESKTOP_NAME}"
LEGACY_P2APP_LAUNCHER="P2app.desktop"
LEGACY_P2APP_DESK="${HOME}/Desktop/${LEGACY_P2APP_LAUNCHER}"
LEGACY_P2APP_APPS="${HOME}/.local/share/applications/${LEGACY_P2APP_LAUNCHER}"
AUTOSTART_DIR="${HOME}/.config/autostart"
AUTOSTART_NAME="P2_app-Control-tray.desktop"
AUTOSTART_FILE="${AUTOSTART_DIR}/${AUTOSTART_NAME}"
ENABLE_TRAY_AUTOSTART="${ENABLE_TRAY_AUTOSTART:-1}"

POLKIT_RULE="/etc/polkit-1/rules.d/49-p2app.rules"

echo "[*] Repo root: ${REPO_ROOT}"

echo "[*] Building widget..."
make -C "$HERE"

echo "[*] Installing widget binary -> ${BIN_INSTALL}"
sudo install -D -m 0755 "$BIN_LOCAL" "$BIN_INSTALL"

echo "[*] Ensuring systemd unit exists/updated -> ${UNIT_PATH}"

if [[ ! -x "${P2APP_BIN}" ]]; then
  echo "[!] ERROR: Expected P2_app binary not found or not executable:"
  echo "    ${P2APP_BIN}"
  exit 1
fi

TMP_UNIT="$(mktemp)"
cat > "$TMP_UNIT" <<EOF2
[Unit]
Description=P2_app Service for ANAN G2
After=network-online.target
Wants=network-online.target
Documentation=https://github.com/laurencebarker/Saturn

[Service]
WorkingDirectory=${P2APP_DIR}
ExecStart=${P2APP_BIN} -s -p
User=root
Group=root
Restart=always
RestartSec=5
TimeoutStopSec=30
Environment=LD_LIBRARY_PATH=/usr/local/lib:/usr/lib
Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
StandardOutput=journal
StandardError=journal
SyslogIdentifier=p2app
LimitNOFILE=4096

[Install]
WantedBy=multi-user.target
EOF2

if [[ ! -f "${UNIT_PATH}" ]] || ! sudo cmp -s "$TMP_UNIT" "$UNIT_PATH"; then
  echo "    -> writing unit file"
  sudo install -D -m 0644 "$TMP_UNIT" "$UNIT_PATH"
else
  echo "    -> unit already matches (no change)"
fi
rm -f "$TMP_UNIT"

echo "[*] Installing polkit rule (no password for start/stop/restart of p2app.service)"
TMP_RULE="$(mktemp)"
cat > "$TMP_RULE" <<'EOF3'
polkit.addRule(function(action, subject) {
    if (action.id === "org.freedesktop.systemd1.manage-units" &&
        subject.user === "pi" &&
        subject.active === true &&
        subject.local === true) {

        var unit = action.lookup("unit");
        var verb = action.lookup("verb");

        if (unit === "p2app.service" &&
            (verb === "start" || verb === "stop" || verb === "restart")) {
            return polkit.Result.YES;
        }
    }
});
EOF3

if [[ ! -f "${POLKIT_RULE}" ]] || ! sudo cmp -s "$TMP_RULE" "$POLKIT_RULE"; then
  sudo install -D -m 0644 "$TMP_RULE" "$POLKIT_RULE"
  sudo systemctl restart polkit || true
fi
rm -f "$TMP_RULE"

echo "[*] Reloading systemd + enabling service"
sudo systemctl daemon-reload
sudo systemctl enable "${UNIT_NAME}" >/dev/null

if sudo systemctl is-active --quiet "${UNIT_NAME}"; then
  sudo systemctl restart "${UNIT_NAME}"
else
  sudo systemctl start "${UNIT_NAME}"
fi

echo "[*] Removing legacy desktop shortcuts (window mode launcher no longer installed)"
rm -f "$DESKTOP_APPS" "$DESKTOP_DESK" "$LEGACY_P2APP_APPS" "$LEGACY_P2APP_DESK"

if [[ "$ENABLE_TRAY_AUTOSTART" == "1" ]]; then
  TARGET_USER="${SUDO_USER:-${SATURN_USER:-}}"
  if [[ -z "$TARGET_USER" && ${EUID:-$UID} -eq 0 ]]; then
    TARGET_USER="$(basename "$HOME")"
  fi

  echo "[*] Installing tray autostart entry"
  mkdir -p "$AUTOSTART_DIR"
  TMP_AUTOSTART="$(mktemp)"
  cat > "$TMP_AUTOSTART" <<EOF5
[Desktop Entry]
Type=Application
Name=P2_app Control Tray
Comment=Panel tray control for p2app.service
Exec=${BIN_INSTALL} --tray
Icon=utilities-terminal
Terminal=false
Categories=Utility;System;
X-GNOME-Autostart-enabled=true
EOF5
  install -m 0644 "$TMP_AUTOSTART" "$AUTOSTART_FILE"
  if [[ ${EUID:-$UID} -eq 0 && -n "$TARGET_USER" ]] && id -u "$TARGET_USER" >/dev/null 2>&1; then
    chown "$TARGET_USER:$TARGET_USER" "$AUTOSTART_DIR" "$AUTOSTART_FILE" || true
  fi
  rm -f "$TMP_AUTOSTART"
else
  echo "[*] Tray autostart disabled by ENABLE_TRAY_AUTOSTART=0"
  rm -f "$AUTOSTART_FILE"
fi

command -v update-desktop-database >/dev/null 2>&1 && \
  update-desktop-database "${HOME}/.local/share/applications" >/dev/null 2>&1 || true

echo
echo "[✓] Done."
echo "    Widget:   ${BIN_INSTALL}"
echo "    Desktop:  (legacy shortcut removed)"
if [[ "$ENABLE_TRAY_AUTOSTART" == "1" ]]; then
  echo "    Autostart:${AUTOSTART_FILE}"
fi
echo "    Unit:     ${UNIT_PATH}"
echo
echo "Service status:"
sudo systemctl status "${UNIT_NAME}" --no-pager | sed -n '1,12p'
