#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"

UNIT_NAME="p2app.service"
UNIT_PATH="/etc/systemd/system/${UNIT_NAME}"
XDMA_READY_UNIT_NAME="saturn-xdma-ready.service"
XDMA_READY_UNIT_PATH="/etc/systemd/system/${XDMA_READY_UNIT_NAME}"
XDMA_REG_DEV="/dev/xdma0_user"
P2APP_START_TIMEOUT_SECONDS="${P2APP_START_TIMEOUT_SECONDS:-30}"

P2APP_DIR="${REPO_ROOT}/sw_projects/P2_app"
P2APP_SOURCE_BIN="${P2APP_DIR}/p2app"
P2APP_RUNTIME_ROOT="${P2APP_RUNTIME_ROOT:-/opt/saturn-radio}"
P2APP_BIN="${P2APP_RUNTIME_ROOT}/bin/p2app"
P2APP_SERVICE_USER="${P2APP_SERVICE_USER:-saturn-radio}"
P2APP_SERVICE_GROUP="${P2APP_SERVICE_GROUP:-saturn-radio}"
XDMA_DOCTOR_LOCAL="${REPO_ROOT}/scripts/saturn-xdma-doctor.sh"
XDMA_DOCTOR_INSTALL="/usr/local/bin/saturn-xdma-doctor.sh"
XDMA_READY_LOCAL="${REPO_ROOT}/scripts/saturn-xdma-ready.sh"
XDMA_READY_INSTALL="/usr/local/bin/saturn-xdma-ready.sh"
FIX_XDMA_LOCAL="${REPO_ROOT}/scripts/fix-xdma.sh"
FIX_XDMA_INSTALL="/usr/local/bin/saturn-fix-xdma.sh"
XDMA_POSTINST_LOCAL="${REPO_ROOT}/scripts/saturn-xdma-kernel-postinst.sh"
XDMA_POSTINST_INSTALL="/usr/local/bin/saturn-xdma-kernel-postinst.sh"
XDMA_POSTINST_HOOK_PATH="/etc/kernel/postinst.d/saturn-xdma"
APP_INFO_LOCAL="${REPO_ROOT}/update_manager/scripts/g2-version-info.sh"
APP_INFO_INSTALL="/usr/local/bin/saturn-g2-version-info.sh"

BIN_LOCAL="${HERE}/p2app-control"
BIN_INSTALL="/usr/local/bin/p2app-control"
ICON_LOCAL="${HERE}/p2appcontrol.png"
ICON_PIXMAP_INSTALL="/usr/local/share/pixmaps/p2appcontrol.png"
ICON_THEME_DIR="/usr/local/share/icons/hicolor/32x32/apps"
ICON_THEME_INSTALL="${ICON_THEME_DIR}/p2appcontrol.png"
ICON_THEME_ROOT="/usr/local/share/icons/hicolor"

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
FRONT_PANEL_STATE_FILE="${SATURN_FRONT_PANEL_STATE_FILE:-/var/lib/saturn-provision/front-panel-type}"
P2APP_ENABLE_CONTROL_PANEL="${P2APP_ENABLE_CONTROL_PANEL:-auto}"

POLKIT_RULE="/etc/polkit-1/rules.d/49-p2app.rules"
SUDOERS_RULE="/etc/sudoers.d/49-p2app-control"
SYSTEMCTL_BIN="/bin/systemctl"
CONTROL_USER="${SUDO_USER:-${SATURN_USER:-${USER:-pi}}}"

if [[ -z "${CONTROL_USER}" || "${CONTROL_USER}" == "root" ]]; then
  CONTROL_USER="${SATURN_USER:-pi}"
fi

if [[ "$P2APP_ENABLE_CONTROL_PANEL" == "auto" ]]; then
  front_panel_type="${SATURN_FRONT_PANEL_TYPE:-}"
  if [[ -z "$front_panel_type" && -f "$FRONT_PANEL_STATE_FILE" ]]; then
    front_panel_type="$(tr -d '[:space:]' < "$FRONT_PANEL_STATE_FILE" 2>/dev/null || true)"
  fi
  case "$front_panel_type" in
    G2V1|G2V2) P2APP_ENABLE_CONTROL_PANEL=1 ;;
    NONE) P2APP_ENABLE_CONTROL_PANEL=0 ;;
    *) P2APP_ENABLE_CONTROL_PANEL=1 ;;
  esac
fi

P2APP_EXEC_ARGS="-s"
P2APP_PANEL_MODE="off"
case "$P2APP_ENABLE_CONTROL_PANEL" in
  1|true|TRUE|yes|YES|on|ON)
    P2APP_EXEC_ARGS="-s -p"
    P2APP_PANEL_MODE="enabled"
    ;;
esac

TMP_XDMA_READY_UNIT=""
TMP_UNIT=""
TMP_RULE=""
TMP_SUDOERS=""
TMP_AUTOSTART=""
TMP_POSTINST_HOOK=""
trap 'rm -f "$TMP_XDMA_READY_UNIT" "$TMP_UNIT" "$TMP_RULE" "$TMP_SUDOERS" "$TMP_AUTOSTART" "$TMP_POSTINST_HOOK" 2>/dev/null || true' EXIT

echo "[*] Repo root: ${REPO_ROOT}"

service_is_running() {
  local active_state sub_state
  active_state="$(sudo systemctl show -p ActiveState --value "${UNIT_NAME}" 2>/dev/null || true)"
  sub_state="$(sudo systemctl show -p SubState --value "${UNIT_NAME}" 2>/dev/null || true)"
  [[ "${active_state}" == "active" && "${sub_state}" == "running" ]]
}

ensure_radio_service_account() {
  if ! getent group "$P2APP_SERVICE_GROUP" >/dev/null 2>&1; then
    sudo groupadd --system "$P2APP_SERVICE_GROUP"
  fi
  if ! id -u "$P2APP_SERVICE_USER" >/dev/null 2>&1; then
    sudo useradd --system --gid "$P2APP_SERVICE_GROUP" \
      --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin \
      "$P2APP_SERVICE_USER"
  fi
  local group
  for group in i2c gpio dialout; do
    if getent group "$group" >/dev/null 2>&1; then
      sudo usermod -a -G "$group" "$P2APP_SERVICE_USER"
    fi
  done
}

wait_for_service_running() {
  local elapsed=0
  while (( elapsed < P2APP_START_TIMEOUT_SECONDS )); do
    if service_is_running; then
      return 0
    fi
    sleep 1
    ((elapsed+=1))
  done
  return 1
}

echo "[*] Building widget..."
make -C "$HERE"

echo "[*] Installing widget binary -> ${BIN_INSTALL}"
sudo install -D -m 0755 "$BIN_LOCAL" "$BIN_INSTALL"

if [[ ! -f "${ICON_LOCAL}" ]]; then
  echo "[!] ERROR: Expected icon file not found:"
  echo "    ${ICON_LOCAL}"
  exit 1
fi

echo "[*] Installing p2app-control icon -> ${ICON_PIXMAP_INSTALL}"
sudo install -D -m 0644 "${ICON_LOCAL}" "${ICON_PIXMAP_INSTALL}"
echo "[*] Installing p2app-control theme icon -> ${ICON_THEME_INSTALL}"
sudo install -D -m 0644 "${ICON_LOCAL}" "${ICON_THEME_INSTALL}"
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  echo "[*] Refreshing icon cache -> ${ICON_THEME_ROOT}"
  sudo gtk-update-icon-cache -f -t "${ICON_THEME_ROOT}" >/dev/null 2>&1 || \
    echo "[!] WARN: gtk-update-icon-cache failed for ${ICON_THEME_ROOT}"
fi

if [[ ! -x "${XDMA_DOCTOR_LOCAL}" ]]; then
  echo "[!] ERROR: XDMA doctor script not found or not executable:"
  echo "    ${XDMA_DOCTOR_LOCAL}"
  exit 1
fi
if [[ ! -x "${XDMA_READY_LOCAL}" ]]; then
  echo "[!] ERROR: XDMA readiness script not found or not executable:"
  echo "    ${XDMA_READY_LOCAL}"
  exit 1
fi
if [[ ! -x "${FIX_XDMA_LOCAL}" ]]; then
  echo "[!] ERROR: fix-xdma script not found or not executable:"
  echo "    ${FIX_XDMA_LOCAL}"
  exit 1
fi
if [[ ! -x "${XDMA_POSTINST_LOCAL}" ]]; then
  echo "[!] ERROR: XDMA kernel postinst helper not found or not executable:"
  echo "    ${XDMA_POSTINST_LOCAL}"
  exit 1
fi
if [[ ! -x "${APP_INFO_LOCAL}" ]]; then
  echo "[!] ERROR: App info helper not found or not executable:"
  echo "    ${APP_INFO_LOCAL}"
  exit 1
fi

echo "[*] Installing XDMA support helpers"
sudo install -D -m 0755 "${XDMA_DOCTOR_LOCAL}" "${XDMA_DOCTOR_INSTALL}"
sudo install -D -m 0755 "${XDMA_READY_LOCAL}" "${XDMA_READY_INSTALL}"
sudo install -D -m 0755 "${FIX_XDMA_LOCAL}" "${FIX_XDMA_INSTALL}"
sudo install -D -m 0755 "${XDMA_POSTINST_LOCAL}" "${XDMA_POSTINST_INSTALL}"
sudo install -D -m 0755 "${APP_INFO_LOCAL}" "${APP_INFO_INSTALL}"

if command -v dkms >/dev/null 2>&1 && [[ -n "$(dkms status -m saturn-xdma 2>/dev/null || true)" ]]; then
  echo "[*] DKMS manages Saturn XDMA; leaving the legacy kernel postinst hook disabled"
  if [[ -e "${XDMA_POSTINST_HOOK_PATH}" || -L "${XDMA_POSTINST_HOOK_PATH}" ]]; then
    sudo mv -f "${XDMA_POSTINST_HOOK_PATH}" "${XDMA_POSTINST_HOOK_PATH}.disabled-by-dkms"
  fi
else
  echo "[*] Ensuring kernel postinst hook exists/updated -> ${XDMA_POSTINST_HOOK_PATH}"
  TMP_POSTINST_HOOK="$(mktemp)"
  cat > "$TMP_POSTINST_HOOK" <<EOF1
#!/bin/sh
set -eu
HELPER="${XDMA_POSTINST_INSTALL}"
if [ -x "\$HELPER" ]; then
  "\$HELPER" "\$@" || true
fi
exit 0
EOF1

  if [[ ! -f "${XDMA_POSTINST_HOOK_PATH}" ]] || ! sudo cmp -s "$TMP_POSTINST_HOOK" "$XDMA_POSTINST_HOOK_PATH"; then
    echo "    -> writing kernel postinst hook"
    sudo install -D -m 0755 "$TMP_POSTINST_HOOK" "$XDMA_POSTINST_HOOK_PATH"
  else
    echo "    -> kernel postinst hook already matches (no change)"
  fi
  rm -f "$TMP_POSTINST_HOOK"
fi

echo "[*] Ensuring systemd unit exists/updated -> ${UNIT_PATH}"

if [[ ! -x "${P2APP_SOURCE_BIN}" ]]; then
  echo "[!] ERROR: Expected P2_app binary not found or not executable:"
  echo "    ${P2APP_SOURCE_BIN}"
  exit 1
fi

echo "[*] Installing root-owned p2app runtime -> ${P2APP_BIN}"
ensure_radio_service_account
sudo install -d -m 0755 -o root -g root "${P2APP_RUNTIME_ROOT}/bin"
sudo install -m 0755 -o root -g root "${P2APP_SOURCE_BIN}" "${P2APP_BIN}"

echo "[*] Ensuring XDMA readiness unit exists/updated -> ${XDMA_READY_UNIT_PATH}"
TMP_XDMA_READY_UNIT="$(mktemp)"
cat > "$TMP_XDMA_READY_UNIT" <<EOF2
[Unit]
Description=Saturn XDMA Readiness Gate
After=systemd-udevd.service
Wants=systemd-udevd.service

[Service]
Type=oneshot
ExecStart=${XDMA_READY_INSTALL}
User=root
Group=root
Environment=SATURN_XDMA_DOCTOR_PATH=${XDMA_DOCTOR_INSTALL}
StandardOutput=journal
StandardError=journal
SyslogIdentifier=saturn-xdma-ready
EOF2

if [[ ! -f "${XDMA_READY_UNIT_PATH}" ]] || ! sudo cmp -s "$TMP_XDMA_READY_UNIT" "$XDMA_READY_UNIT_PATH"; then
  echo "    -> writing XDMA readiness unit file"
  sudo install -D -m 0644 "$TMP_XDMA_READY_UNIT" "$XDMA_READY_UNIT_PATH"
else
  echo "    -> XDMA readiness unit already matches (no change)"
fi
rm -f "$TMP_XDMA_READY_UNIT"

TMP_UNIT="$(mktemp)"
cat > "$TMP_UNIT" <<EOF3
[Unit]
Description=P2_app Service for ANAN G2
After=network-online.target ${XDMA_READY_UNIT_NAME}
Wants=network-online.target
Requires=${XDMA_READY_UNIT_NAME}
Documentation=https://github.com/kd4yal2024/Saturn

[Service]
WorkingDirectory=${P2APP_RUNTIME_ROOT}
ExecStart=${P2APP_BIN} ${P2APP_EXEC_ARGS}
User=${P2APP_SERVICE_USER}
Group=${P2APP_SERVICE_GROUP}
Restart=always
RestartSec=5
TimeoutStopSec=30
Environment=LD_LIBRARY_PATH=/usr/local/lib:/usr/lib
Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
Environment=SATURN_P23_SELECTED_APP=p2
Environment=SATURN_P23_STARTUP_MODE=service-default
Environment=SATURN_FRONT_PANEL_MODE=${P2APP_PANEL_MODE}
StandardOutput=journal
StandardError=journal
SyslogIdentifier=p2app
LimitNOFILE=4096
UMask=0027
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictNamespaces=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
CapabilityBoundingSet=CAP_SYS_NICE
AmbientCapabilities=CAP_SYS_NICE

[Install]
WantedBy=multi-user.target
EOF3

if [[ ! -f "${UNIT_PATH}" ]] || ! sudo cmp -s "$TMP_UNIT" "$UNIT_PATH"; then
  echo "    -> writing unit file"
  sudo install -D -m 0644 "$TMP_UNIT" "$UNIT_PATH"
else
  echo "    -> unit already matches (no change)"
fi
rm -f "$TMP_UNIT"

echo "[*] Installing polkit rule (no password for start/stop/restart of p2app.service)"
TMP_RULE="$(mktemp)"
cat > "$TMP_RULE" <<EOF3
polkit.addRule(function(action, subject) {
    if (action.id === "org.freedesktop.systemd1.manage-units" &&
        subject.user === "${CONTROL_USER}" &&
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

echo "[*] Installing sudoers rule (no password for start/stop/restart/enable/disable of p2app.service)"
TMP_SUDOERS="$(mktemp)"
cat > "$TMP_SUDOERS" <<EOF4
${CONTROL_USER} ALL=(root) NOPASSWD: ${SYSTEMCTL_BIN} start ${UNIT_NAME}
${CONTROL_USER} ALL=(root) NOPASSWD: ${SYSTEMCTL_BIN} stop ${UNIT_NAME}
${CONTROL_USER} ALL=(root) NOPASSWD: ${SYSTEMCTL_BIN} restart ${UNIT_NAME}
${CONTROL_USER} ALL=(root) NOPASSWD: ${SYSTEMCTL_BIN} enable ${UNIT_NAME}
${CONTROL_USER} ALL=(root) NOPASSWD: ${SYSTEMCTL_BIN} disable ${UNIT_NAME}
${CONTROL_USER} ALL=(root) NOPASSWD: ${APP_INFO_INSTALL}
EOF4

if command -v visudo >/dev/null 2>&1; then
  sudo visudo -cf "$TMP_SUDOERS" >/dev/null
fi

if [[ ! -f "${SUDOERS_RULE}" ]] || ! sudo cmp -s "$TMP_SUDOERS" "$SUDOERS_RULE"; then
  sudo install -D -m 0440 "$TMP_SUDOERS" "$SUDOERS_RULE"
fi
rm -f "$TMP_SUDOERS"

echo "[*] Reloading systemd + enabling service"
sudo systemctl daemon-reload
sudo systemctl enable "${UNIT_NAME}" >/dev/null

start_cmd_rc=0
if sudo systemctl is-active --quiet "${UNIT_NAME}"; then
  sudo systemctl restart "${UNIT_NAME}" || start_cmd_rc=$?
else
  sudo systemctl start "${UNIT_NAME}" || start_cmd_rc=$?
fi

start_rc=0
if [[ "${start_cmd_rc}" -ne 0 ]]; then
  echo "[!] WARN: initial systemctl start for ${UNIT_NAME} returned ${start_cmd_rc}."
fi
if ! wait_for_service_running; then
  echo "[!] WARN: ${UNIT_NAME} did not reach active/running within ${P2APP_START_TIMEOUT_SECONDS}s."
  if [[ ! -e "${XDMA_REG_DEV}" ]]; then
    echo "[!] WARN: ${XDMA_REG_DEV} is not present."
    echo "[!] WARN: XDMA may be loaded but the FPGA/register device is not enumerated yet."
    echo "[!] WARN: Provisioning will continue; p2app.service will keep retrying in the background."
  else
    echo "[!] ERROR: ${XDMA_REG_DEV} exists, but ${UNIT_NAME} is still not active."
    start_rc=1
  fi
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
Icon=${ICON_PIXMAP_INSTALL}
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
echo "    Icon:     ${ICON_PIXMAP_INSTALL}"
echo "    ThemeIcon:${ICON_THEME_INSTALL}"
echo "    Doctor:   ${XDMA_DOCTOR_INSTALL}"
echo "    XDMA gate:${XDMA_READY_INSTALL}"
echo "    Fix XDMA: ${FIX_XDMA_INSTALL}"
echo "    Postinst: ${XDMA_POSTINST_INSTALL}"
echo "    App Info: ${APP_INFO_INSTALL}"
echo "    Desktop:  (legacy shortcut removed)"
if [[ "$ENABLE_TRAY_AUTOSTART" == "1" ]]; then
  echo "    Autostart:${AUTOSTART_FILE}"
fi
echo "    KernelHook:${XDMA_POSTINST_HOOK_PATH}"
echo "    ReadyUnit:${XDMA_READY_UNIT_PATH}"
echo "    Unit:     ${UNIT_PATH}"
echo "    Sudoers:  ${SUDOERS_RULE}"
echo
echo "Service status:"
status_output="$(sudo systemctl status "${UNIT_NAME}" --no-pager 2>&1 || true)"
printf '%s\n' "$status_output" | sed -n '1,12p'

if [[ "${start_rc}" -ne 0 ]]; then
  exit "${start_rc}"
fi
