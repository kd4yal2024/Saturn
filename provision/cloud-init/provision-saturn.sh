#!/usr/bin/env bash
set -Eeuo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck disable=SC1091
source "${SCRIPT_DIR}/../../scripts/saturn-lcd-lib.sh"

# Preserve explicit environment overrides so /etc/default does not clobber them.
declare -A _ENV_SATURN_OVERRIDES=()
while IFS='=' read -r _saturn_name _saturn_value; do
  case "$_saturn_name" in
    SATURN_*) _ENV_SATURN_OVERRIDES["$_saturn_name"]="$_saturn_value" ;;
  esac
done < <(env)

# Optional environment file written by cloud-init user-data.
if [[ -f /etc/default/saturn-provision ]]; then
  # shellcheck disable=SC1091
  source /etc/default/saturn-provision
fi

for _saturn_name in "${!_ENV_SATURN_OVERRIDES[@]}"; do
  export "${_saturn_name}=${_ENV_SATURN_OVERRIDES[$_saturn_name]}"
done
unset _saturn_name _saturn_value

SATURN_USER="${SATURN_USER:-pi}"
SATURN_GROUP="${SATURN_GROUP:-}"
SATURN_USER_RETRY_SECONDS="${SATURN_USER_RETRY_SECONDS:-30}"
SATURN_REPO_URL="${SATURN_REPO_URL:-https://github.com/kd4yal2024/Saturn.git}"
SATURN_REPO_BRANCH="${SATURN_REPO_BRANCH:-main}"
SATURN_REPO_DIR="${SATURN_REPO_DIR:-}"
SATURN_REPO_SYNC="${SATURN_REPO_SYNC:-1}"
SATURN_INSTALL_PROFILE="${SATURN_INSTALL_PROFILE:-appliance}"

SATURN_INSTALL_PACKAGES="${SATURN_INSTALL_PACKAGES:-1}"
SATURN_INSTALL_UPDATE_MANAGER="${SATURN_INSTALL_UPDATE_MANAGER:-1}"
SATURN_INSTALL_PIHPSDR="${SATURN_INSTALL_PIHPSDR:-0}"
SATURN_INSTALL_SATURN_BRIDGE="${SATURN_INSTALL_SATURN_BRIDGE:-1}"
SATURN_REQUIRE_SATURN_BRIDGE="${SATURN_REQUIRE_SATURN_BRIDGE:-1}"
SATURN_INSTALL_P2APP_CONTROL="${SATURN_INSTALL_P2APP_CONTROL:-1}"
SATURN_RUN_P2_TESTS="${SATURN_RUN_P2_TESTS:-1}"
SATURN_INSTALL_UDEV_RULES="${SATURN_INSTALL_UDEV_RULES:-1}"
SATURN_INSTALL_SHUTDOWN_WAITER="${SATURN_INSTALL_SHUTDOWN_WAITER:-1}"
SATURN_REBUILD_XDMA="${SATURN_REBUILD_XDMA:-1}"
SATURN_BUILD_OPTIONAL_TOOLS="${SATURN_BUILD_OPTIONAL_TOOLS:-1}"
SATURN_INSTALL_DEVELOPER_TOOLS="${SATURN_INSTALL_DEVELOPER_TOOLS:-1}"
SATURN_INSTALL_CLOUD_INIT="${SATURN_INSTALL_CLOUD_INIT:-0}"
SATURN_DETECT_FRONT_PANEL="${SATURN_DETECT_FRONT_PANEL:-1}"
SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT="${SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT:-auto}"
SATURN_FORCE_SYSTEM_ROLE="${SATURN_FORCE_SYSTEM_ROLE:-}"

SATURN_FLASH_FPGA="${SATURN_FLASH_FPGA:-0}"
SATURN_FLASH_IMAGE="${SATURN_FLASH_IMAGE:-latest}"
SATURN_FLASH_FALLBACK="${SATURN_FLASH_FALLBACK:-0}"
SATURN_FLASH_CONFIRM="${SATURN_FLASH_CONFIRM:-}"

SATURN_ADMIN_PASSWORD="${SATURN_ADMIN_PASSWORD:-}"
SATURN_ADMIN_PASSWORD_MIN_LEN=5
SATURN_NONINTERACTIVE="${SATURN_NONINTERACTIVE:-0}"
SATURN_VERIFY_MODE="${SATURN_VERIFY_MODE:-hardware}"
SATURN_RESUME="${SATURN_RESUME:-1}"
SATURN_HOST_SCHEMA_VERSION=2
SATURN_FORCE_REPROVISION="${SATURN_FORCE_REPROVISION:-0}"
SATURN_STATE_DIR="${SATURN_STATE_DIR:-/var/lib/saturn-provision}"
SATURN_LOG_FILE="${SATURN_LOG_FILE:-/var/log/saturn-provision.log}"
SATURN_DESKTOP_UI="${SATURN_DESKTOP_UI:-auto}"
SATURN_UI_TIMEOUT_SECONDS="${SATURN_UI_TIMEOUT_SECONDS:-2700}"
SATURN_UI_STATUS_FILE="${SATURN_UI_STATUS_FILE:-${SATURN_STATE_DIR}/ui-status}"
SATURN_UI_BINARY="${SATURN_UI_BINARY:-/usr/local/bin/saturn-provision-ui}"
SATURN_UI_SOURCE_FILE="${SATURN_UI_SOURCE_FILE:-${SCRIPT_DIR}/saturn-provision-ui.cpp}"
SATURN_UI_SHOW_LOG_DEFAULT="${SATURN_UI_SHOW_LOG_DEFAULT:-0}"
SATURN_UI_LAUNCHER="${SATURN_UI_LAUNCHER:-/usr/local/bin/saturn-provision-ui-launcher.sh}"
SATURN_UI_AUTOSTART_NAME="${SATURN_UI_AUTOSTART_NAME:-saturn-provision-ui.desktop}"
SATURN_UI_POWER_HELPER_SOURCE="${SATURN_UI_POWER_HELPER_SOURCE:-${SCRIPT_DIR}/saturn-provision-powerctl.sh}"
SATURN_UI_POWER_HELPER="${SATURN_UI_POWER_HELPER:-/usr/local/sbin/saturn-provision-powerctl}"
SATURN_UI_POWER_SUDOERS="${SATURN_UI_POWER_SUDOERS:-/etc/sudoers.d/49-saturn-provision-powerctl}"
SATURN_PIHPSDR_INSTALLER_ENABLED="${SATURN_PIHPSDR_INSTALLER_ENABLED:-1}"
SATURN_PIHPSDR_INSTALLER_BINARY="${SATURN_PIHPSDR_INSTALLER_BINARY:-/usr/local/bin/pihpsdr-installer-ui}"
SATURN_PIHPSDR_INSTALLER_SOURCE_FILE="${SATURN_PIHPSDR_INSTALLER_SOURCE_FILE:-${SCRIPT_DIR}/pihpsdr-installer-ui.cpp}"
SATURN_PIHPSDR_INSTALLER_RUNNER="${SATURN_PIHPSDR_INSTALLER_RUNNER:-/usr/local/bin/pihpsdr-installer-run.sh}"
SATURN_PIHPSDR_INSTALLER_RUNNER_SOURCE="${SATURN_PIHPSDR_INSTALLER_RUNNER_SOURCE:-${SCRIPT_DIR}/pihpsdr-installer-run.sh}"
SATURN_PIHPSDR_INSTALLER_LAUNCHER="${SATURN_PIHPSDR_INSTALLER_LAUNCHER:-/usr/local/bin/pihpsdr-installer-launcher.sh}"
SATURN_PIHPSDR_INSTALLER_SHORTCUT_NAME="${SATURN_PIHPSDR_INSTALLER_SHORTCUT_NAME:-piHPSDR-Installer.desktop}"
SATURN_PIHPSDR_INSTALLER_TITLE="${SATURN_PIHPSDR_INSTALLER_TITLE:-piHPSDR Installer}"
SATURN_PIHPSDR_INSTALLER_ICON_FILE="${SATURN_PIHPSDR_INSTALLER_ICON_FILE:-}"
SATURN_CLEAN_TMP_AFTER_PROVISION="${SATURN_CLEAN_TMP_AFTER_PROVISION:-1}"
SATURN_ENABLE_I2C="${SATURN_ENABLE_I2C:-1}"
SATURN_ENABLE_SSH="${SATURN_ENABLE_SSH:-1}"
SATURN_ENABLE_VNC="${SATURN_ENABLE_VNC:-1}"
SATURN_LCD_PROFILE="${SATURN_LCD_PROFILE:-auto}"
SATURN_LCD_SIZE_INCH="${SATURN_LCD_SIZE_INCH:-}"
SATURN_LCD_AUTO_DEFAULT_SIZE_INCH="${SATURN_LCD_AUTO_DEFAULT_SIZE_INCH:-7}"
SATURN_LCD_I2C_DETECT_ADDR="${SATURN_LCD_I2C_DETECT_ADDR:-0x45}"
SATURN_LCD_DETECT_ONLY="${SATURN_LCD_DETECT_ONLY:-0}"
SATURN_APT_LOCK_TIMEOUT_SECONDS="${SATURN_APT_LOCK_TIMEOUT_SECONDS:-120}"
SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS="${SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS:-3}"

apt_updated="${SATURN_APT_ALREADY_UPDATED:-0}"
SATURN_UI_STARTED=0
SATURN_UI_AUTOSTART_INSTALLED=0
PYTHON_GUARD_DIR=""
UPDATE_MANAGER_PASSWORD_FILE_DEFAULT="/var/lib/saturn-provision/update-manager-admin-password"
SATURN_FRONT_PANEL_TYPE=""
SATURN_FRONT_PANEL_STATE_FILE="${SATURN_FRONT_PANEL_STATE_FILE:-${SATURN_STATE_DIR}/front-panel-type}"
SATURN_XDMA_PRESENT=""
SATURN_SYSTEM_ROLE=""
SATURN_SYSTEM_ROLE_STATE_FILE="${SATURN_SYSTEM_ROLE_STATE_FILE:-${SATURN_STATE_DIR}/system-role}"
SATURN_PROFILE_ENV_FILE="${SATURN_PROFILE_ENV_FILE:-${SATURN_STATE_DIR}/profile.env}"

log() { printf '[%(%Y-%m-%d %H:%M:%S)T] %s\n' -1 "$*" >&2; }
write_ui_status() {
  local state="$1"
  local message="${2:-}"
  local timestamp
  timestamp="$(date '+%Y-%m-%d %H:%M:%S')"
  install -d -m 0755 "$SATURN_STATE_DIR" >/dev/null 2>&1 || true
  printf '%s|%s|%s\n' "$state" "$timestamp" "$message" >"$SATURN_UI_STATUS_FILE" 2>/dev/null || true
}
die() {
  write_ui_status "FAILED" "$*"
  log "ERROR: $*"
  exit 1
}

# Keep Python from dropping bytecode into source trees.
export PYTHONDONTWRITEBYTECODE=1
export PYTHONPYCACHEPREFIX="${PYTHONPYCACHEPREFIX:-/var/cache/saturn-python}"

bool_true() {
  case "${1:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

kernel_flavor() {
  local krel="${1:-$(uname -r)}"
  printf '%s\n' "${krel#*+rpt-}"
}

ensure_tmp_permissions() {
  if [[ ! -d /tmp ]]; then
    install -d -m 1777 /tmp
  fi
  chmod 1777 /tmp
  chown root:root /tmp 2>/dev/null || true
}

ensure_dir_present() {
  local dir="$1"
  local mode="${2:-0755}"
  if [[ -d "$dir" ]]; then
    return 0
  fi
  install -d -m "$mode" "$dir"
}

detect_display() {
  if [[ -n "${DISPLAY:-}" ]]; then
    printf '%s\n' "$DISPLAY"
    return 0
  fi
  if [[ -S /tmp/.X11-unix/X0 ]]; then
    printf '%s\n' ":0"
    return 0
  fi
  return 1
}

desktop_ui_enabled() {
  case "${SATURN_DESKTOP_UI:-auto}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    0|false|FALSE|no|NO|off|OFF) return 1 ;;
    auto|AUTO|"") return 0 ;;
    *)
      log "WARN: Unknown SATURN_DESKTOP_UI value '${SATURN_DESKTOP_UI}'; desktop UI disabled."
      return 1
      ;;
  esac
}

build_desktop_ui_binary() {
  [[ -f "$SATURN_UI_SOURCE_FILE" ]] || {
    log "WARN: Desktop UI source not found: $SATURN_UI_SOURCE_FILE"
    return 1
  }

  if [[ -x "$SATURN_UI_BINARY" && "$SATURN_UI_BINARY" -nt "$SATURN_UI_SOURCE_FILE" ]]; then
    return 0
  fi

  if ! command -v g++ >/dev/null 2>&1 || ! command -v pkg-config >/dev/null 2>&1; then
    log "WARN: g++/pkg-config missing; cannot build desktop UI binary."
    return 1
  fi

  local gtk_flags
  gtk_flags="$(pkg-config --cflags --libs gtk+-3.0 2>/dev/null || true)"
  if [[ -z "$gtk_flags" ]]; then
    log "WARN: pkg-config could not resolve gtk+-3.0; desktop UI disabled."
    return 1
  fi

  log "Building desktop provisioning UI binary: $SATURN_UI_BINARY"
  install -d -m 0755 "$(dirname "$SATURN_UI_BINARY")"
  # shellcheck disable=SC2086
  if ! g++ -std=c++17 -O2 -Wall -Wextra "$SATURN_UI_SOURCE_FILE" -o "$SATURN_UI_BINARY" $gtk_flags; then
    log "WARN: Failed to build desktop provisioning UI binary."
    return 1
  fi
  return 0
}

build_pihpsdr_installer_binary() {
  [[ -f "$SATURN_PIHPSDR_INSTALLER_SOURCE_FILE" ]] || {
    log "WARN: piHPSDR installer UI source not found: $SATURN_PIHPSDR_INSTALLER_SOURCE_FILE"
    return 1
  }

  if [[ -x "$SATURN_PIHPSDR_INSTALLER_BINARY" && "$SATURN_PIHPSDR_INSTALLER_BINARY" -nt "$SATURN_PIHPSDR_INSTALLER_SOURCE_FILE" ]]; then
    return 0
  fi

  if ! command -v g++ >/dev/null 2>&1 || ! command -v pkg-config >/dev/null 2>&1; then
    log "WARN: g++/pkg-config missing; cannot build piHPSDR installer UI binary."
    return 1
  fi

  local gtk_flags
  gtk_flags="$(pkg-config --cflags --libs gtk+-3.0 2>/dev/null || true)"
  if [[ -z "$gtk_flags" ]]; then
    log "WARN: pkg-config could not resolve gtk+-3.0; piHPSDR installer UI disabled."
    return 1
  fi

  log "Building piHPSDR installer UI binary: $SATURN_PIHPSDR_INSTALLER_BINARY"
  install -d -m 0755 "$(dirname "$SATURN_PIHPSDR_INSTALLER_BINARY")"
  # shellcheck disable=SC2086
  if ! g++ -std=c++17 -O2 -Wall -Wextra "$SATURN_PIHPSDR_INSTALLER_SOURCE_FILE" -o "$SATURN_PIHPSDR_INSTALLER_BINARY" $gtk_flags; then
    log "WARN: Failed to build piHPSDR installer UI binary."
    return 1
  fi
  return 0
}

read_desktop_entry_value() {
  local desktop_file="$1"
  local key="$2"
  local line value

  [[ -f "$desktop_file" ]] || return 1

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^[[:space:]]*${key}[[:space:]]*= ]] || continue
    value="${line#*=}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    printf '%s\n' "$value"
    return 0
  done < "$desktop_file"

  return 1
}

resolve_pihpsdr_icon() {
  local saturn_home="$1"
  local desktop_file icon_path

  if [[ -n "$SATURN_PIHPSDR_INSTALLER_ICON_FILE" && -f "$SATURN_PIHPSDR_INSTALLER_ICON_FILE" ]]; then
    printf '%s\n' "$SATURN_PIHPSDR_INSTALLER_ICON_FILE"
    return 0
  fi

  if [[ -f "${saturn_home}/github/pihpsdr/piHPSDR_logo.png" ]]; then
    printf '%s\n' "${saturn_home}/github/pihpsdr/piHPSDR_logo.png"
    return 0
  fi

  for desktop_file in \
    "${saturn_home}/.local/share/applications/pihpsdr.desktop" \
    "${saturn_home}/github/pihpsdr/pihpsdr.desktop" \
    "/usr/share/applications/pihpsdr.desktop"
  do
    [[ -f "$desktop_file" ]] || continue
    icon_path="$(read_desktop_entry_value "$desktop_file" "Icon" 2>/dev/null || true)"
    if [[ -n "$icon_path" && -f "$icon_path" ]]; then
      printf '%s\n' "$icon_path"
      return 0
    fi
  done

  printf '\n'
}

install_desktop_ui_autostart() {
  local saturn_home="$1"
  local autostart_dir desktop_file show_log_flag

  [[ "$SATURN_UI_AUTOSTART_INSTALLED" -eq 0 ]] || return 0
  desktop_ui_enabled || return 0
  if ! build_desktop_ui_binary; then
    return 0
  fi

  autostart_dir="${saturn_home}/.config/autostart"
  desktop_file="${autostart_dir}/${SATURN_UI_AUTOSTART_NAME}"
  if bool_true "$SATURN_UI_SHOW_LOG_DEFAULT"; then
    show_log_flag="1"
  else
    show_log_flag="0"
  fi

  install -d -m 0755 -o "$SATURN_USER" -g "$SATURN_GROUP" "$autostart_dir"

  cat > "$SATURN_UI_LAUNCHER" <<EOF
#!/usr/bin/env bash
set -Eeuo pipefail

if pgrep -u "\$(id -u)" -x saturn-provision-ui >/dev/null 2>&1; then
  exit 0
fi

args=(
  "$SATURN_UI_BINARY"
  "--log-file" "$SATURN_LOG_FILE"
  "--completion-file" "${SATURN_STATE_DIR}/complete"
  "--status-file" "$SATURN_UI_STATUS_FILE"
  "--timeout-seconds" "$SATURN_UI_TIMEOUT_SECONDS"
)

if [[ "$show_log_flag" == "1" ]]; then
  args+=("--show-log")
fi

exec "\${args[@]}"
EOF
  chmod 0755 "$SATURN_UI_LAUNCHER"

  cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=Saturn Provisioning Status
Comment=Shows Saturn provisioning progress
Exec=$SATURN_UI_LAUNCHER
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
  chown "$SATURN_USER:$SATURN_GROUP" "$desktop_file"
  chmod 0644 "$desktop_file"
  SATURN_UI_AUTOSTART_INSTALLED=1
  log "Installed desktop autostart for $SATURN_USER: $desktop_file"
}

remove_desktop_ui_autostart() {
  local saturn_home="$1"
  local desktop_file
  desktop_file="${saturn_home}/.config/autostart/${SATURN_UI_AUTOSTART_NAME}"
  if [[ -f "$desktop_file" ]]; then
    rm -f "$desktop_file"
    log "Removed desktop autostart after successful provisioning: $desktop_file"
  fi
}

install_pihpsdr_installer_shortcut() {
  local saturn_home="$1"
  local icon_file icon_line desktop_dir desktop_file

  bool_true "$SATURN_PIHPSDR_INSTALLER_ENABLED" || return 0

  if [[ ! -f "$SATURN_PIHPSDR_INSTALLER_RUNNER_SOURCE" ]]; then
    log "WARN: piHPSDR installer runner source not found: $SATURN_PIHPSDR_INSTALLER_RUNNER_SOURCE"
    return 0
  fi

  if [[ ! -f /opt/saturn-go/scripts/update-pihpsdr.py ]]; then
    log "WARN: Installed update-pihpsdr.py not found under /opt/saturn-go/scripts; skipping piHPSDR installer shortcut."
    return 0
  fi

  if ! build_pihpsdr_installer_binary; then
    return 0
  fi

  icon_file="$(resolve_pihpsdr_icon "$saturn_home")"
  if [[ -n "$icon_file" ]]; then
    icon_line="Icon=${icon_file}"
  else
    icon_line=""
    log "WARN: piHPSDR icon not found; standalone installer shortcut will use the default icon."
  fi

  install -d -m 0755 "$(dirname "$SATURN_PIHPSDR_INSTALLER_RUNNER")"
  install -m 0755 "$SATURN_PIHPSDR_INSTALLER_RUNNER_SOURCE" "$SATURN_PIHPSDR_INSTALLER_RUNNER"

  cat > "$SATURN_PIHPSDR_INSTALLER_LAUNCHER" <<EOF
#!/usr/bin/env bash
set -Eeuo pipefail

if pgrep -u "\$(id -u)" -f "$SATURN_PIHPSDR_INSTALLER_BINARY" >/dev/null 2>&1; then
  exit 0
fi

state_dir="\${XDG_STATE_HOME:-\$HOME/.local/state}/pihpsdr-installer"
shortcut_path="\$HOME/Desktop/$SATURN_PIHPSDR_INSTALLER_SHORTCUT_NAME"
mkdir -p "\$state_dir"

args=(
  "$SATURN_PIHPSDR_INSTALLER_BINARY"
  "--log-file" "\${state_dir}/install.log"
  "--status-file" "\${state_dir}/status"
  "--runner" "$SATURN_PIHPSDR_INSTALLER_RUNNER"
  "--desktop-shortcut" "\$shortcut_path"
  "--window-title" "$SATURN_PIHPSDR_INSTALLER_TITLE"
  "--show-log"
)

if [[ -n "$icon_file" ]]; then
  args+=("--icon-file" "$icon_file")
fi

exec "\${args[@]}"
EOF
  chmod 0755 "$SATURN_PIHPSDR_INSTALLER_LAUNCHER"

  desktop_dir="${saturn_home}/Desktop"
  desktop_file="${desktop_dir}/${SATURN_PIHPSDR_INSTALLER_SHORTCUT_NAME}"
  install -d -m 0755 -o "$SATURN_USER" -g "$SATURN_GROUP" "$desktop_dir"
  cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=$SATURN_PIHPSDR_INSTALLER_TITLE
Comment=Open the standalone piHPSDR installer
Exec=$SATURN_PIHPSDR_INSTALLER_LAUNCHER
Path=$saturn_home
Terminal=false
StartupNotify=true
${icon_line}
EOF
  chown "$SATURN_USER:$SATURN_GROUP" "$desktop_file"
  chmod 0755 "$desktop_file"
  log "Installed standalone piHPSDR installer shortcut for $SATURN_USER: $desktop_file"
}

launch_desktop_ui() {
  local saturn_home="$1"
  local display xauth user_uid runtime_dir dbus_address wayland_display ui_cmd_escaped
  local -a ui_cmd

  [[ "$SATURN_UI_STARTED" -eq 0 ]] || return 0
  desktop_ui_enabled || return 0

  display="$(detect_display 2>/dev/null || true)"
  if [[ -z "$display" ]]; then
    if [[ "$SATURN_DESKTOP_UI" == "1" || "$SATURN_DESKTOP_UI" == "true" || "$SATURN_DESKTOP_UI" == "TRUE" || "$SATURN_DESKTOP_UI" == "yes" || "$SATURN_DESKTOP_UI" == "YES" || "$SATURN_DESKTOP_UI" == "on" || "$SATURN_DESKTOP_UI" == "ON" ]]; then
      log "WARN: SATURN_DESKTOP_UI is forced on, but no desktop display was found."
    else
      log "Desktop session not active yet; UI will appear when $SATURN_USER logs in."
    fi
    return 0
  fi

  if ! build_desktop_ui_binary; then
    return 0
  fi

  xauth="${XAUTHORITY:-${saturn_home}/.Xauthority}"
  ui_cmd=(
    "$SATURN_UI_BINARY"
    "--log-file" "$SATURN_LOG_FILE"
    "--completion-file" "${SATURN_STATE_DIR}/complete"
    "--status-file" "$SATURN_UI_STATUS_FILE"
    "--timeout-seconds" "$SATURN_UI_TIMEOUT_SECONDS"
  )
  if bool_true "$SATURN_UI_SHOW_LOG_DEFAULT"; then
    ui_cmd+=("--show-log")
  fi

  user_uid="$(id -u "$SATURN_USER" 2>/dev/null || true)"
  runtime_dir="${XDG_RUNTIME_DIR:-/run/user/${user_uid}}"
  dbus_address="${DBUS_SESSION_BUS_ADDRESS:-unix:path=${runtime_dir}/bus}"
  wayland_display="${WAYLAND_DISPLAY:-}"
  if [[ -z "$wayland_display" && -S "${runtime_dir}/wayland-0" ]]; then
    wayland_display="wayland-0"
  fi
  printf -v ui_cmd_escaped '%q ' "${ui_cmd[@]}"

  if run_as_user "$saturn_home" env DISPLAY="$display" XAUTHORITY="$xauth" WAYLAND_DISPLAY="$wayland_display" XDG_RUNTIME_DIR="$runtime_dir" DBUS_SESSION_BUS_ADDRESS="$dbus_address" \
    bash -lc "if pgrep -u \"\$(id -u)\" -f '^$SATURN_UI_BINARY( |\$)' >/dev/null 2>&1; then exit 0; fi; nohup ${ui_cmd_escaped} >/tmp/saturn-provision-ui.log 2>&1 < /dev/null & ui_pid=\$!; sleep 1; kill -0 \"\$ui_pid\""; then
    SATURN_UI_STARTED=1
    log "Desktop provisioning UI launched (detached, DISPLAY=$display, WAYLAND_DISPLAY=${wayland_display:-none})."
  else
    log "WARN: Failed to launch desktop provisioning UI."
  fi
}

install_desktop_ui_power_helper() {
  local tmp_sudoers

  if [[ ! -f "$SATURN_UI_POWER_HELPER_SOURCE" ]]; then
    log "WARN: Desktop UI power helper source not found: $SATURN_UI_POWER_HELPER_SOURCE"
    return 1
  fi

  install -d -m 0755 "$(dirname "$SATURN_UI_POWER_HELPER")"
  install -m 0755 "$SATURN_UI_POWER_HELPER_SOURCE" "$SATURN_UI_POWER_HELPER"

  tmp_sudoers="$(mktemp)"
  cat > "$tmp_sudoers" <<EOF
Defaults!${SATURN_UI_POWER_HELPER} !requiretty
${SATURN_USER} ALL=(root) NOPASSWD: ${SATURN_UI_POWER_HELPER} reboot, ${SATURN_UI_POWER_HELPER} poweroff
EOF

  command -v visudo >/dev/null 2>&1 || \
    die "visudo is required to validate the Saturn provisioning power-helper sudoers entry."
  visudo -cf "$tmp_sudoers" >/dev/null || \
    die "Generated sudoers entry for Saturn provisioning power helper is invalid."

  install -m 0440 "$tmp_sudoers" "$SATURN_UI_POWER_SUDOERS"
  rm -f "$tmp_sudoers"
  log "Installed desktop UI power helper: $SATURN_UI_POWER_HELPER"
  log "Installed desktop UI power sudoers rule: $SATURN_UI_POWER_SUDOERS"
}

set_ui_stage() {
  local message="$1"
  write_ui_status "RUNNING" "$message"
}

handle_error() {
  local line="$1"
  local cmd="$2"
  write_ui_status "FAILED" "Line $line failed while running: $cmd"
  die "Line $line failed while running: $cmd"
}

generate_password() {
  local charset='abcdefghjkmnpqrstuvwxyz23456789'
  local charset_len="${#charset}"
  local password=""
  local hex idx

  # Avoid pipelines here: with `set -o pipefail`, `tr | head` can fail on SIGPIPE.
  while [[ ${#password} -lt $SATURN_ADMIN_PASSWORD_MIN_LEN ]]; do
    hex="$(od -An -N1 -tx1 /dev/urandom)"
    hex="${hex//[[:space:]]/}"
    [[ -n "$hex" ]] || continue
    idx=$((16#$hex % charset_len))
    password+="${charset:idx:1}"
  done

  printf '%s' "$password"
}

run_as_user() {
  local home="$1"
  shift
  sudo -u "$SATURN_USER" -H env HOME="$home" "$@"
}

ensure_root() {
  [[ "$(id -u)" -eq 0 ]] || die "Run as root."
}

ensure_user() {
  local saturn_home="" elapsed=0
  local retry_seconds max_wait_seconds
  retry_seconds="${SATURN_USER_RETRY_SECONDS:-30}"
  max_wait_seconds="${SATURN_USER_MAX_WAIT_SECONDS:-600}"
  if ! [[ "$retry_seconds" =~ ^[0-9]+$ ]] || (( retry_seconds < 1 )); then
    retry_seconds=30
  fi
  if ! [[ "$max_wait_seconds" =~ ^[0-9]+$ ]] || (( max_wait_seconds < 1 )); then
    max_wait_seconds=600
  fi
  while true; do
    if id -u "$SATURN_USER" >/dev/null 2>&1; then
      saturn_home="$(getent passwd "$SATURN_USER" | cut -d: -f6 || true)"
      if [[ -n "$saturn_home" ]]; then
        printf '%s\n' "$saturn_home"
        return 0
      fi
      log "User '$SATURN_USER' exists but home directory is unresolved; retrying in ${retry_seconds}s."
    else
      log "User '$SATURN_USER' not present yet; retrying in ${retry_seconds}s."
    fi
    if (( elapsed + retry_seconds >= max_wait_seconds )); then
      die "User '$SATURN_USER' was not available after ${max_wait_seconds}s. Set SATURN_USER to an existing login user."
    fi
    sleep "$retry_seconds"
    elapsed=$((elapsed + retry_seconds))
  done
}

assert_not_repo_python_script() {
  local script="$1"
  if [[ -n "${SATURN_REPO_DIR:-}" && "$script" == "$SATURN_REPO_DIR/"* && "$script" == *.py ]]; then
    die "Refusing to execute Python script from repo tree: $script"
  fi
}

install_python_guard_wrapper() {
  local wrapper_path="$1"
  local real_python="$2"
  cat > "$wrapper_path" <<EOF
#!/usr/bin/env bash
set -Eeuo pipefail

repo_dir="\${SATURN_REPO_DIR:-}"
repo_real=""
if [[ -n "\$repo_dir" ]]; then
  repo_real="\$(readlink -f "\$repo_dir" 2>/dev/null || true)"
fi

for arg in "\$@"; do
  [[ "\$arg" == *.py ]] || continue
  candidate="\$arg"
  if [[ "\$candidate" != /* ]]; then
    candidate="\$(pwd)/\$candidate"
  fi
  resolved="\$(readlink -f "\$candidate" 2>/dev/null || true)"
  if [[ -n "\$resolved" && -n "\$repo_real" && "\$resolved" == "\$repo_real/"* ]]; then
    echo "ERROR: Refusing Python script execution from repo tree: \$resolved" >&2
    exit 101
  fi
done

exec "$real_python" "\$@"
EOF
  chmod 0755 "$wrapper_path"
}

enable_python_repo_guard() {
  local python3_real python_real
  python3_real="$(command -v python3 || true)"
  [[ -n "$python3_real" ]] || die "python3 is required but not found in PATH"
  python_real="$(command -v python || true)"

  PYTHON_GUARD_DIR="$(mktemp -d /tmp/saturn-python-guard.XXXXXX)"
  install_python_guard_wrapper "$PYTHON_GUARD_DIR/python3" "$python3_real"
  if [[ -n "$python_real" ]]; then
    install_python_guard_wrapper "$PYTHON_GUARD_DIR/python" "$python_real"
  else
    ln -s python3 "$PYTHON_GUARD_DIR/python"
  fi
  export PATH="$PYTHON_GUARD_DIR:$PATH"
}

cleanup_python_guard() {
  if [[ -n "${PYTHON_GUARD_DIR:-}" && -d "$PYTHON_GUARD_DIR" ]]; then
    rm -rf "$PYTHON_GUARD_DIR" || true
  fi
}

apt_update_once() {
  if [[ "$apt_updated" -eq 0 ]]; then
    export DEBIAN_FRONTEND=noninteractive
    apt_run update -y
    apt_updated=1
  fi
}

apt_install() {
  apt_update_once
  export DEBIAN_FRONTEND=noninteractive
  apt_run install -y --no-install-recommends "$@"
}

apt_run() {
  local timeout retry_interval start_ts attempt output rc
  timeout="${SATURN_APT_LOCK_TIMEOUT_SECONDS:-120}"
  retry_interval="${SATURN_APT_LOCK_RETRY_INTERVAL_SECONDS:-3}"

  if ! [[ "$timeout" =~ ^[0-9]+$ ]] || (( timeout < 1 )); then
    timeout=120
  fi
  if ! [[ "$retry_interval" =~ ^[0-9]+$ ]] || (( retry_interval < 1 )); then
    retry_interval=3
  fi

  start_ts="$(date +%s)"
  attempt=1
  while true; do
    output="$(
      # Keep an expected apt failure in a conditional context so the inherited
      # ERR trap does not terminate this subshell before apt_run can inspect
      # the diagnostic and retry transient lock contention.
      if apt-get "$@" 2>&1; then
        rc=0
      else
        rc=$?
      fi
      printf '\n__SATURN_APT_EXIT_CODE__=%s\n' "$rc"
    )"
    rc="${output##*__SATURN_APT_EXIT_CODE__=}"
    output="${output%$'\n'__SATURN_APT_EXIT_CODE__=*}"
    printf '%s\n' "$output"

    if [[ "$rc" == "0" ]]; then
      return 0
    fi

    if [[ "$output" != *"Could not get lock"* && "$output" != *"Unable to lock directory"* ]]; then
      return "$rc"
    fi

    if (( $(date +%s) - start_ts >= timeout )); then
      log "apt-get $* timed out waiting for apt lock after ${timeout}s."
      return "$rc"
    fi

    log "apt lock is busy; retrying apt-get $* in ${retry_interval}s (attempt ${attempt})."
    attempt=$((attempt + 1))
    sleep "$retry_interval"
  done
}

ensure_packages() {
  local running_krel running_meta_pkg
  running_krel="$(uname -r)"
  running_meta_pkg="linux-headers-$(kernel_flavor "$running_krel")"

  log "Installing build/runtime dependencies"
  apt_install \
    git rsync curl wget ca-certificates sudo \
    build-essential pkg-config gcc g++ make dkms \
    python3 python3-venv python3-pip python3-psutil \
    gpiod libgpiod-dev libi2c-dev libgtk-3-dev libglib2.0-bin lxterminal \
    libasound2-dev libpulse-dev libopus0 libusb-1.0-0-dev libcurl4-openssl-dev \
    libfftw3-dev \
    desktop-file-utils xdg-user-dirs

  if bool_true "$SATURN_INSTALL_P2APP_CONTROL"; then
    apt_install libayatana-appindicator3-dev ayatana-indicator-application
  fi

  # The automatic desktop install, standalone installer, and Update Manager
  # all build piHPSDR locally. Install their native build dependencies up front
  # so the unprivileged GUI/web runners never need an interactive sudo prompt.
  if bool_true "$SATURN_INSTALL_PIHPSDR" \
    || bool_true "$SATURN_PIHPSDR_INSTALLER_ENABLED" \
    || bool_true "$SATURN_INSTALL_UPDATE_MANAGER"; then
    apt_install \
      libminiupnpc-dev libwebsockets-dev zlib1g-dev libopus-dev \
      libsqlite3-dev libssl-dev
  fi

  if bool_true "$SATURN_INSTALL_UPDATE_MANAGER"; then
    apt_install nginx apache2-utils nodejs npm binutils
  fi

  if bool_true "$SATURN_INSTALL_CLOUD_INIT"; then
    apt_install cloud-init
  fi

  if bool_true "$SATURN_ENABLE_I2C"; then
    apt_install i2c-tools
  fi

  if bool_true "$SATURN_ENABLE_SSH"; then
    apt_install openssh-server
  fi

  if (bool_true "$SATURN_ENABLE_I2C" || bool_true "$SATURN_ENABLE_SSH" || bool_true "$SATURN_ENABLE_VNC") \
    && ! command -v raspi-config >/dev/null 2>&1 \
    && apt-cache show raspi-config >/dev/null 2>&1; then
    apt_install raspi-config
  fi

  if bool_true "$SATURN_ENABLE_VNC"; then
    if apt-cache show realvnc-vnc-server >/dev/null 2>&1; then
      apt_install realvnc-vnc-server
    else
      log "WARN: realvnc-vnc-server package not available in apt sources; VNC enablement expects an installed VNC service."
    fi
  fi

  if ! command -v pinctrl >/dev/null 2>&1; then
    if apt-cache show raspi-utils >/dev/null 2>&1; then
      apt_install raspi-utils
    fi
    if ! command -v pinctrl >/dev/null 2>&1; then
      apt_install cmake device-tree-compiler libfdt-dev
    fi
  fi

  if apt-cache show "linux-headers-${running_krel}" >/dev/null 2>&1; then
    apt_install "linux-headers-${running_krel}"
  elif apt-cache show "${running_meta_pkg}" >/dev/null 2>&1; then
    apt_install "${running_meta_pkg}"
  elif apt-cache show raspberrypi-kernel-headers >/dev/null 2>&1; then
    apt_install raspberrypi-kernel-headers
  else
    log "WARN: No known Raspberry Pi kernel header package found in apt sources."
  fi
}

enable_service_if_available() {
  local service
  for service in "$@"; do
    if systemctl cat "$service" >/dev/null 2>&1; then
      log "Enabling service: $service"
      if systemctl enable --now "$service"; then
        return 0
      fi
      log "WARN: Failed to enable/start service: $service"
    fi
  done
  return 1
}

get_boot_cmdline_file() {
  local candidate
  for candidate in /boot/firmware/cmdline.txt /boot/cmdline.txt; do
    if [[ -f "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

configure_usb_boot_tweaks() {
  local boot_config boot_cmdline existing_cmdline

  boot_config="$(get_boot_config_file 2>/dev/null || true)"
  if [[ -n "$boot_config" ]]; then
    if grep -Eq '^[[:space:]]*dtoverlay=dwc2,dr_mode=host([[:space:]]*#.*)?$' "$boot_config"; then
      :
    elif grep -Eq '^[[:space:]]*#?[[:space:]]*dtoverlay=dwc2,dr_mode=host' "$boot_config"; then
      sed -i -E 's/^[[:space:]]*#?[[:space:]]*dtoverlay=dwc2,dr_mode=host.*/dtoverlay=dwc2,dr_mode=host/' "$boot_config" || true
    else
      printf '\n# enable USB\ndtoverlay=dwc2,dr_mode=host\n' >>"$boot_config"
    fi
    log "Ensured USB host overlay in $boot_config"
  else
    log "WARN: Could not locate /boot/firmware/config.txt or /boot/config.txt for USB overlay setup."
  fi

  boot_cmdline="$(get_boot_cmdline_file 2>/dev/null || true)"
  if [[ -n "$boot_cmdline" ]]; then
    if grep -Eq '(^|[[:space:]])usbhid\.mousepoll=0([[:space:]]|$)' "$boot_cmdline"; then
      :
    else
      existing_cmdline="$(tr -d '\n' < "$boot_cmdline")"
      printf '%s usbhid.mousepoll=0\n' "$existing_cmdline" >"$boot_cmdline"
    fi
    log "Ensured usbhid.mousepoll=0 in $boot_cmdline"
  else
    log "WARN: Could not locate /boot/firmware/cmdline.txt or /boot/cmdline.txt for mousepoll tuning."
  fi
}

install_vscode_extensions() {
  local saturn_home="$1"
  local ext
  local -a extensions=("ms-vscode.cpptools" "eamodio.gitlens")

  if ! command -v code >/dev/null 2>&1; then
    log "WARN: VS Code CLI not found; skipping extension install."
    return 0
  fi

  for ext in "${extensions[@]}"; do
    if run_as_user "$saturn_home" code --install-extension "$ext" --force >/dev/null 2>&1; then
      log "Installed VS Code extension: $ext"
    else
      log "WARN: Failed to install VS Code extension: $ext"
    fi
  done
}

install_desktop_dev_tools() {
  local saturn_home="$1"

  if ! bool_true "$SATURN_INSTALL_PACKAGES"; then
    log "Skipping developer-package installation (SATURN_INSTALL_PACKAGES=0)"
    install_vscode_extensions "$saturn_home"
    return 0
  fi

  apt_update_once
  export DEBIAN_FRONTEND=noninteractive

  if apt-cache show code >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends code
    install_vscode_extensions "$saturn_home"
  else
    log "WARN: VS Code package 'code' not available in apt sources; skipping editor and extension installation."
  fi

  if apt-cache show git-cola >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends git-cola
  else
    log "WARN: git-cola package not available in apt sources."
  fi
}

enable_i2c_fallback() {
  local boot_config
  boot_config="$(get_boot_config_file 2>/dev/null || true)"

  if [[ -n "$boot_config" ]]; then
    if grep -Eq '^[[:space:]]*dtparam=i2c_arm=on([[:space:]]*#.*)?$' "$boot_config"; then
      :
    elif grep -Eq '^[[:space:]]*#?[[:space:]]*dtparam=i2c_arm=on' "$boot_config"; then
      if ! sed -i -E 's/^[[:space:]]*#?[[:space:]]*dtparam=i2c_arm=on.*/dtparam=i2c_arm=on/' "$boot_config"; then
        log "WARN: Failed to update dtparam=i2c_arm=on in $boot_config"
      fi
    else
      if ! printf '\n# Enabled by Saturn provisioning\ndtparam=i2c_arm=on\n' >>"$boot_config"; then
        log "WARN: Failed to append dtparam=i2c_arm=on to $boot_config"
      fi
    fi
    log "Ensured I2C boot setting in $boot_config"
  else
    log "WARN: Could not locate /boot/firmware/config.txt or /boot/config.txt for I2C boot setting."
  fi

  install -d -m 0755 /etc/modules-load.d
  if ! printf 'i2c-dev\n' > /etc/modules-load.d/i2c-dev.conf; then
    log "WARN: Failed to write /etc/modules-load.d/i2c-dev.conf"
  fi
  modprobe i2c-dev >/dev/null 2>&1 || true
}

configure_i2c_vnc_ssh() {
  local has_raspi_config=0
  local i2c_done=0 ssh_done=0 vnc_done=0

  if command -v raspi-config >/dev/null 2>&1; then
    has_raspi_config=1
  fi

  if bool_true "$SATURN_ENABLE_I2C"; then
    if [[ "$has_raspi_config" -eq 1 ]]; then
      log "Enabling I2C via raspi-config"
      if raspi-config nonint do_i2c 0; then
        i2c_done=1
      else
        log "WARN: raspi-config failed to enable I2C; using fallback."
      fi
    fi
    if [[ "$i2c_done" -eq 0 ]]; then
      enable_i2c_fallback
    fi
  fi

  if bool_true "$SATURN_ENABLE_SSH"; then
    if [[ "$has_raspi_config" -eq 1 ]]; then
      log "Enabling SSH via raspi-config"
      if raspi-config nonint do_ssh 0; then
        ssh_done=1
      else
        log "WARN: raspi-config failed to enable SSH; trying systemd service fallback."
      fi
    fi
    if [[ "$ssh_done" -eq 0 ]]; then
      if ! enable_service_if_available ssh.service sshd.service; then
        log "WARN: Could not find an SSH service unit to enable."
      fi
    fi
  fi

  if bool_true "$SATURN_ENABLE_VNC"; then
    if [[ "$has_raspi_config" -eq 1 ]]; then
      log "Enabling VNC via raspi-config"
      if raspi-config nonint do_vnc 0; then
        vnc_done=1
      else
        log "WARN: raspi-config failed to enable VNC; trying systemd service fallback."
      fi
    fi
    if [[ "$vnc_done" -eq 0 ]]; then
      if ! enable_service_if_available vncserver-x11-serviced.service wayvnc.service x11vnc.service; then
        log "WARN: Could not find a VNC service unit to enable."
      fi
    fi
  fi
}

ensure_repo() {
  local saturn_home="$1"
  local repo_parent
  if [[ -z "$SATURN_REPO_DIR" ]]; then
    SATURN_REPO_DIR="${saturn_home}/github/Saturn"
  fi
  repo_parent="$(dirname "$SATURN_REPO_DIR")"

  install -d -m 0755 -o "$SATURN_USER" -g "$SATURN_GROUP" "$repo_parent"

  if [[ -d "${SATURN_REPO_DIR}/.git" ]]; then
    if ! bool_true "$SATURN_REPO_SYNC"; then
      log "Using existing repository without fetching: $SATURN_REPO_DIR"
      return 0
    fi
    log "Updating repo at $SATURN_REPO_DIR -> ${SATURN_REPO_BRANCH}"
    run_as_user "$saturn_home" git -C "$SATURN_REPO_DIR" fetch --depth 1 origin "$SATURN_REPO_BRANCH"
    run_as_user "$saturn_home" git -C "$SATURN_REPO_DIR" checkout -B "$SATURN_REPO_BRANCH" "origin/${SATURN_REPO_BRANCH}"
  else
    log "Cloning repo $SATURN_REPO_URL ($SATURN_REPO_BRANCH) into $SATURN_REPO_DIR"
    run_as_user "$saturn_home" git clone --depth 1 --branch "$SATURN_REPO_BRANCH" "$SATURN_REPO_URL" "$SATURN_REPO_DIR"
  fi
}

prepare_python_env() {
  local saturn_home="$1"
  local venv_dir="${saturn_home}/venv"

  log "Preparing Python virtual environment at $venv_dir"
  if [[ ! -d "$venv_dir" ]]; then
    run_as_user "$saturn_home" python3 -m venv "$venv_dir"
  fi
  run_as_user "$saturn_home" "$venv_dir/bin/pip" install --upgrade pip
  run_as_user "$saturn_home" "$venv_dir/bin/pip" install rich==13.8.1 psutil pyfiglet
}

build_dir() {
  local saturn_home="$1"
  local label="$2"
  local dir="$3"
  local nproc="$4"
  local required="${5:-1}"
  [[ -d "$dir" ]] || die "$label directory missing: $dir"
  log "Building $label ($dir)"
  if ! run_as_user "$saturn_home" make -C "$dir" -j"$nproc"; then
    if bool_true "$required"; then
      die "Build failed for required target: $label"
    fi
    log "WARN: Optional build failed for $label; continuing."
    return 1
  fi
  return 0
}

build_saturn_apps() {
  local saturn_home="$1"
  local nproc="$2"

  if bool_true "$SATURN_INSTALL_P2APP_CONTROL"; then
    if bool_true "$SATURN_RUN_P2_TESTS"; then
      log "Running Protocol 2 tests before installation"
      run_as_user "$saturn_home" make -C "$SATURN_REPO_DIR/sw_projects/P2_app" test
    fi
    build_dir "$saturn_home" "P2_app" "$SATURN_REPO_DIR/sw_projects/P2_app" "$nproc" 1
  else
    log "Skipping P2_app build (SATURN_INSTALL_P2APP_CONTROL=0)"
  fi
  log "Skipping P1_app build (not required in provisioning flow)."
  build_dir "$saturn_home" "audiotest"   "$SATURN_REPO_DIR/sw_projects/audiotest" "$nproc" 1
  build_dir "$saturn_home" "biascheck"   "$SATURN_REPO_DIR/sw_projects/biascheck" "$nproc" 1
  build_dir "$saturn_home" "codectest"   "$SATURN_REPO_DIR/sw_projects/codectest" "$nproc" 1
  build_dir "$saturn_home" "axi_rw"      "$SATURN_REPO_DIR/sw_tools/axi_rw" "$nproc" 1
  build_dir "$saturn_home" "flashwriter" "$SATURN_REPO_DIR/sw_tools/flashwriter" "$nproc" 1
  build_dir "$saturn_home" "load-FPGA"   "$SATURN_REPO_DIR/sw_tools/load-FPGA" "$nproc" 1
  build_dir "$saturn_home" "saturn-lcd-setup" "$SATURN_REPO_DIR/sw_tools/saturn-lcd-setup" "$nproc" 1
  build_dir "$saturn_home" "spiload"     "$SATURN_REPO_DIR/sw_tools/spiload" "$nproc" 1

  if bool_true "$SATURN_BUILD_OPTIONAL_TOOLS"; then
    build_dir "$saturn_home" "FPGAVersion"      "$SATURN_REPO_DIR/sw_tools/FPGAVersion" "$nproc" 0
    build_dir "$saturn_home" "IQdmatest"        "$SATURN_REPO_DIR/sw_tools/IQdmatest" "$nproc" 0
    build_dir "$saturn_home" "codecwrite"       "$SATURN_REPO_DIR/sw_tools/codecwrite" "$nproc" 0
    build_dir "$saturn_home" "spiadcread"       "$SATURN_REPO_DIR/sw_tools/spiadcread" "$nproc" 0
    build_dir "$saturn_home" "linuxdriver tools" "$SATURN_REPO_DIR/linuxdriver/tools" "$nproc" 0
  fi
}

build_and_install_xdma() {
  local saturn_home="$1"
  local install_script="$SATURN_REPO_DIR/scripts/install-xdma-dkms.sh"

  [[ -x "$install_script" ]] || die "XDMA DKMS installer not found/executable: $install_script"
  assert_not_repo_python_script "$install_script"

  log "Building and installing XDMA kernel module through DKMS"
  env \
    HOME="$saturn_home" \
    SUDO_USER="$SATURN_USER" \
    SATURN_USER="$SATURN_USER" \
    SATURN_REPO_DIR="$SATURN_REPO_DIR" \
    bash "$install_script"

  install -d -m 0755 /etc/modules-load.d
  printf 'xdma\n' > /etc/modules-load.d/xdma.conf
  modprobe xdma
  udevadm control --reload-rules
  udevadm trigger --subsystem-match=xdma || true
  log "DKMS-managed XDMA module installed, loaded, and configured for boot"
}

install_desktop_shortcuts() {
  local saturn_home="$1"
  local script="$SATURN_REPO_DIR/scripts/update-desktop-apps.sh"
  if [[ -x "$script" ]]; then
    assert_not_repo_python_script "$script"
    log "Installing/repairing desktop launchers"
    run_as_user "$saturn_home" env SATURN_ROOT="$SATURN_REPO_DIR" SATURN_SKIP_P2APP_BUILD=1 bash "$script"
  else
    log "WARN: Missing script: $script"
  fi
}

install_udev_rules() {
  local script=""
  # Prefer SATURN_REPO_DIR when already set; fall back to SCRIPT_DIR-relative
  # path so this function works before ensure_repo populates SATURN_REPO_DIR.
  if [[ -n "$SATURN_REPO_DIR" && -x "$SATURN_REPO_DIR/rules/install-rules.sh" ]]; then
    script="$SATURN_REPO_DIR/rules/install-rules.sh"
  elif [[ -x "${SCRIPT_DIR}/../../rules/install-rules.sh" ]]; then
    script="${SCRIPT_DIR}/../../rules/install-rules.sh"
  fi
  if [[ -n "$script" ]]; then
    assert_not_repo_python_script "$script"
    if [[ -n "$SATURN_FRONT_PANEL_TYPE" ]]; then
      log "Installing udev rules for front panel '$SATURN_FRONT_PANEL_TYPE'"
    else
      log "Installing udev rules"
    fi
    SATURN_USER="$SATURN_USER" \
      SATURN_FRONT_PANEL_TYPE="$SATURN_FRONT_PANEL_TYPE" \
      bash "$script"
  else
    log "WARN: Missing udev script (checked SATURN_REPO_DIR='${SATURN_REPO_DIR:-}' and SCRIPT_DIR='$SCRIPT_DIR')"
  fi
}

install_shutdown_waiter() {
  local script="$SATURN_REPO_DIR/scripts/install-shutdown-waiter-service.sh"
  if [[ -x "$script" ]]; then
    log "Installing saturn-shutdown-waiter.service"
    SATURN_USER="$SATURN_USER" \
      SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT="$SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT" \
      bash "$script"
  else
    log "WARN: Missing shutdown waiter installer: $script"
  fi
}

install_led_power_button_fix() {
  local script="$SATURN_REPO_DIR/scripts/fix-LED-power-button.sh"
  if [[ -x "$script" ]]; then
    log "Configuring power-switch LED and shutdown color state"
    if ! SATURN_INSTALL_PACKAGES=0 bash "$script"; then
      log "WARN: LED/power-button repair helper failed: $script"
    fi
  else
    log "WARN: Missing LED/power-button repair helper: $script"
  fi
}

install_p2app_control() {
  local saturn_home="$1"
  local script="$SATURN_REPO_DIR/sw_tools/p2app-control/install.sh"
  if [[ -x "$script" ]]; then
    assert_not_repo_python_script "$script"
    log "Installing p2app-control and p2app.service"
    env HOME="$saturn_home" SUDO_USER="$SATURN_USER" SATURN_USER="$SATURN_USER" \
      SATURN_FRONT_PANEL_TYPE="$SATURN_FRONT_PANEL_TYPE" bash "$script"
  else
    log "WARN: Missing p2app-control installer: $script"
  fi
}

write_front_panel_state() {
  install -d -m 0755 "$(dirname "$SATURN_FRONT_PANEL_STATE_FILE")"
  printf '%s\n' "$SATURN_FRONT_PANEL_TYPE" >"$SATURN_FRONT_PANEL_STATE_FILE"
  chmod 0644 "$SATURN_FRONT_PANEL_STATE_FILE"
}

run_front_panel_detector() {
  local mode="${1:-post-udev}"
  local script="" detected=""
  # Prefer SATURN_REPO_DIR when already set; fall back to SCRIPT_DIR-relative
  # path so this function works before ensure_repo populates SATURN_REPO_DIR.
  if [[ -n "$SATURN_REPO_DIR" && -x "$SATURN_REPO_DIR/scripts/detect-front-panel.sh" ]]; then
    script="$SATURN_REPO_DIR/scripts/detect-front-panel.sh"
  elif [[ -x "${SCRIPT_DIR}/../../scripts/detect-front-panel.sh" ]]; then
    script="${SCRIPT_DIR}/../../scripts/detect-front-panel.sh"
  fi

  if [[ -z "$script" ]]; then
    log "WARN: Missing front-panel detector (checked SATURN_REPO_DIR='${SATURN_REPO_DIR:-}' and SCRIPT_DIR='$SCRIPT_DIR')"
    return 1
  fi

  detected="$(SATURN_PANEL_DETECT_MODE="$mode" bash "$script" 2>/dev/null | tr -d '\r\n' || true)"
  printf '%s\n' "$detected"
}

detect_front_panel() {
  local mode="${1:-post-udev}"
  local detected=""

  detected="$(run_front_panel_detector "$mode" || true)"
  case "$detected" in
    G2V1|G2V2|NONE)
      SATURN_FRONT_PANEL_TYPE="$detected"
      write_front_panel_state
      log "Detected front panel (${mode}): $SATURN_FRONT_PANEL_TYPE"
      if [[ "$mode" == "post-udev" && "$SATURN_FRONT_PANEL_TYPE" == "NONE" && ! -e /dev/serial/by-id/g2-front-9600 ]]; then
        log "Front-panel serial alias /dev/serial/by-id/g2-front-9600 not present after udev install."
      fi
      ;;
    *)
      log "WARN: Front-panel detector (${mode}) returned unexpected result: ${detected:-<empty>}"
      ;;
  esac
}

verify_front_panel_after_udev() {
  local detected="" pre_detected="${SATURN_FRONT_PANEL_TYPE:-unknown}"

  detected="$(run_front_panel_detector post-udev || true)"
  case "$detected" in
    G2V1|G2V2|NONE)
      if [[ "$detected" == "$pre_detected" ]]; then
        log "Verified front panel after udev: $detected"
      elif [[ -z "$pre_detected" || "$pre_detected" == "unknown" || "$pre_detected" == "NONE" ]]; then
        SATURN_FRONT_PANEL_TYPE="$detected"
        write_front_panel_state
        log "Detected front panel (post-udev): $detected (updated from pre-udev '${pre_detected:-unknown}')"
      else
        log "WARN: Front-panel post-udev verification disagrees with pre-udev result (pre='$pre_detected', post='$detected'). Keeping pre-udev result."
      fi
      if [[ "$detected" == "NONE" && ! -e /dev/serial/by-id/g2-front-9600 ]]; then
        log "Front-panel serial alias /dev/serial/by-id/g2-front-9600 not present after udev install."
      fi
      ;;
    *)
      log "WARN: Front-panel detector (post-udev verification) returned unexpected result: ${detected:-<empty>}"
      ;;
  esac
}

write_system_role_state() {
  install -d -m 0755 "$(dirname "$SATURN_SYSTEM_ROLE_STATE_FILE")"
  printf '%s\n' "$SATURN_SYSTEM_ROLE" >"$SATURN_SYSTEM_ROLE_STATE_FILE"
  chmod 0644 "$SATURN_SYSTEM_ROLE_STATE_FILE"
}

detect_xdma_presence() {
  if [[ -e /dev/xdma0_user || -e /dev/xdma/card0/user ]]; then
    printf '1\n'
    return
  fi
  if command -v lspci >/dev/null 2>&1 && lspci 2>/dev/null | grep -qi xilinx; then
    printf '1\n'
    return
  fi
  printf '0\n'
}

resolve_system_role() {
  local module_family front_panel_type xdma_present forced_role
  module_family="$(detect_module_family 2>/dev/null || true)"
  front_panel_type="${SATURN_FRONT_PANEL_TYPE:-}"
  xdma_present="$(detect_xdma_presence)"
  forced_role="${SATURN_FORCE_SYSTEM_ROLE:-}"

  if [[ -z "$front_panel_type" && -f "$SATURN_FRONT_PANEL_STATE_FILE" ]]; then
    front_panel_type="$(tr -d '\r\n' < "$SATURN_FRONT_PANEL_STATE_FILE" 2>/dev/null || true)"
  fi

  case "$front_panel_type" in
    ""|G2V1|G2V2|NONE)
      ;;
    *)
      log "WARN: Ignoring unexpected front-panel state '${front_panel_type}' from ${SATURN_FRONT_PANEL_STATE_FILE}."
      front_panel_type=""
      ;;
  esac

  case "$forced_role" in
    local_saturn|remotehead_candidate|unknown)
      SATURN_SYSTEM_ROLE="$forced_role"
      ;;
    "")
      if [[ "$xdma_present" == "1" ]]; then
        SATURN_SYSTEM_ROLE="local_saturn"
      elif [[ "$module_family" == "cm5" && "$front_panel_type" == "G2V2" ]]; then
        SATURN_SYSTEM_ROLE="remotehead_candidate"
      else
        SATURN_SYSTEM_ROLE="unknown"
      fi
      ;;
    *)
      log "WARN: Ignoring unsupported SATURN_FORCE_SYSTEM_ROLE='$forced_role'; expected local_saturn|remotehead_candidate|unknown."
      if [[ "$xdma_present" == "1" ]]; then
        SATURN_SYSTEM_ROLE="local_saturn"
      elif [[ "$module_family" == "cm5" && "$front_panel_type" == "G2V2" ]]; then
        SATURN_SYSTEM_ROLE="remotehead_candidate"
      else
        SATURN_SYSTEM_ROLE="unknown"
      fi
      ;;
  esac

  SATURN_XDMA_PRESENT="$xdma_present"
  write_system_role_state
  log "Resolved system role: ${SATURN_SYSTEM_ROLE} (module=${module_family:-unknown}, front_panel=${front_panel_type:-unknown}, xdma_present=${SATURN_XDMA_PRESENT})"
}

device_alias_present() {
  local path="$1"
  if [[ -e "$path" ]]; then
    printf '1\n'
  else
    printf '0\n'
  fi
}

first_existing_device_path() {
  local path
  for path in "$@"; do
    if [[ -e "$path" ]]; then
      printf '%s\n' "$path"
      return 0
    fi
  done
  printf 'none\n'
}

resolve_discovered_processor() {
  local platform_vendor="${1:-unknown}"
  local module_family="${2:-unknown}"
  case "${platform_vendor}:${module_family}" in
    raspberrypi:cm4) printf 'pi CM4\n' ;;
    raspberrypi:cm5) printf 'pi CM5\n' ;;
    *) printf 'other\n' ;;
  esac
}

resolve_front_panel_device_path() {
  local front_panel_type="${1:-unknown}"
  case "$front_panel_type" in
    G2V1)
      if [[ -e /dev/i2c-1 ]]; then
        printf '/dev/i2c-1\n'
      else
        printf 'none\n'
      fi
      ;;
    G2V2)
      first_existing_device_path /dev/serial/by-id/g2-front-9600 /dev/serial/by-id/g2-front-115200
      ;;
    *)
      printf 'none\n'
      ;;
  esac
}

resolve_front_panel_device_addr() {
  local front_panel_type="${1:-unknown}"
  case "$front_panel_type" in
    G2V1) printf '0x20\n' ;;
    *) printf 'none\n' ;;
  esac
}

resolve_expected_display_type() {
  local front_panel_type="${1:-unknown}"
  local system_role="${2:-unknown}"
  if [[ "$system_role" == "remotehead_candidate" ]]; then
    printf '8\n'
    return 0
  fi

  case "$front_panel_type" in
    G2V1) printf '7\n' ;;
    G2V2) printf '8\n' ;;
    *) printf 'none\n' ;;
  esac
}

resolve_radio_profile() {
  local module_family="${1:-unknown}"
  local front_panel_type="${2:-unknown}"
  local xdma_present="${3:-unknown}"
  local system_role="${4:-unknown}"
  local ganymede_present="${5:-0}"

  if [[ "$ganymede_present" == "1" ]]; then
    printf 'G2-1K\n'
    return 0
  fi

  if [[ "$system_role" == "remotehead_candidate" ]]; then
    printf 'RemoteHead\n'
    return 0
  fi

  if [[ "$xdma_present" == "1" || "$front_panel_type" == "G2V1" || "$front_panel_type" == "G2V2" ]]; then
    if [[ "$module_family" == "cm5" && "$front_panel_type" == "G2V2" ]]; then
      printf 'G2 Ultra\n'
    else
      printf 'G2\n'
    fi
    return 0
  fi

  printf 'unknown\n'
}

write_profile_env_line() {
  local key="$1"
  local value="${2:-unknown}"
  printf '%s=%q\n' "$key" "$value"
}

write_profile_env_state() {
  local hardware_model hardware_platform_vendor hardware_module_family hardware_storage_variant
  local boot_config profile_raw lcd_profile display_size_inch lcd_profile_source overlay_raw uart_overlay panel_overlay
  local discovered_processor radio_profile expected_display_type configured_display_type
  local front_panel_device_path front_panel_device_addr
  local ganymede_present ganymede_device_path aries_present aries_device_path
  local pa_protection atu

  hardware_model="$(read_device_tree_model 2>/dev/null || true)"
  hardware_platform_vendor="$(detect_platform_vendor 2>/dev/null || true)"
  hardware_module_family="$(detect_module_family 2>/dev/null || true)"
  hardware_storage_variant="$(detect_module_storage_variant 2>/dev/null || true)"
  discovered_processor="$(resolve_discovered_processor "${hardware_platform_vendor:-unknown}" "${hardware_module_family:-unknown}")"

  boot_config="$(get_boot_config_file 2>/dev/null || true)"
  profile_raw=""
  if [[ -n "$boot_config" ]]; then
    profile_raw="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"
  fi

  lcd_profile="unknown"
  display_size_inch="unknown"
  lcd_profile_source="unknown"
  if [[ -n "$profile_raw" ]]; then
    IFS='|' read -r lcd_profile display_size_inch lcd_profile_source <<<"$profile_raw"
    lcd_profile="${lcd_profile:-unknown}"
    display_size_inch="${display_size_inch:-unknown}"
    lcd_profile_source="${lcd_profile_source:-unknown}"
  fi

  overlay_raw="$(recommended_overlays_for_profile "$lcd_profile" 2>/dev/null || true)"
  uart_overlay="unknown"
  panel_overlay="unknown"
  if [[ -n "$overlay_raw" ]]; then
    IFS='|' read -r uart_overlay panel_overlay <<<"$overlay_raw"
    uart_overlay="${uart_overlay:-unknown}"
    panel_overlay="${panel_overlay:-unknown}"
  fi

  front_panel_device_path="$(resolve_front_panel_device_path "${SATURN_FRONT_PANEL_TYPE:-unknown}")"
  front_panel_device_addr="$(resolve_front_panel_device_addr "${SATURN_FRONT_PANEL_TYPE:-unknown}")"
  expected_display_type="$(resolve_expected_display_type "${SATURN_FRONT_PANEL_TYPE:-unknown}" "${SATURN_SYSTEM_ROLE:-unknown}")"
  configured_display_type="${display_size_inch:-unknown}"

  ganymede_device_path="$(first_existing_device_path /dev/serial/by-path/g2-ganymede-9600)"
  aries_device_path="$(first_existing_device_path /dev/serial/by-id/aries-atu-115200)"
  ganymede_present="$(device_alias_present /dev/serial/by-path/g2-ganymede-9600)"
  aries_present="$(device_alias_present /dev/serial/by-id/aries-atu-115200)"
  if [[ "$ganymede_present" == "1" ]]; then
    pa_protection="Ganymede"
  else
    pa_protection="none"
  fi
  if [[ "$aries_present" == "1" ]]; then
    atu="Aries"
  else
    atu="none"
  fi
  radio_profile="$(resolve_radio_profile "${hardware_module_family:-unknown}" "${SATURN_FRONT_PANEL_TYPE:-unknown}" "${SATURN_XDMA_PRESENT:-unknown}" "${SATURN_SYSTEM_ROLE:-unknown}" "$ganymede_present")"

  install -d -m 0755 "$(dirname "$SATURN_PROFILE_ENV_FILE")"
  {
    write_profile_env_line "profile_format_version" "1"
    write_profile_env_line "profile_generated_at" "$(date --iso-8601=seconds)"
    write_profile_env_line "radio_profile" "$radio_profile"
    write_profile_env_line "radio_profile_source" "heuristic"
    write_profile_env_line "discovered_processor" "$discovered_processor"
    write_profile_env_line "hardware_model" "${hardware_model:-unknown}"
    write_profile_env_line "hardware_platform_vendor" "${hardware_platform_vendor:-unknown}"
    write_profile_env_line "hardware_module_family" "${hardware_module_family:-unknown}"
    write_profile_env_line "hardware_storage_variant" "${hardware_storage_variant:-unknown}"
    write_profile_env_line "front_panel_type" "${SATURN_FRONT_PANEL_TYPE:-unknown}"
    write_profile_env_line "front_panel_device_path" "$front_panel_device_path"
    write_profile_env_line "front_panel_device_addr" "$front_panel_device_addr"
    write_profile_env_line "xdma_present" "${SATURN_XDMA_PRESENT:-unknown}"
    write_profile_env_line "system_role" "${SATURN_SYSTEM_ROLE:-unknown}"
    write_profile_env_line "expected_display_type" "$expected_display_type"
    write_profile_env_line "configured_display_type" "$configured_display_type"
    write_profile_env_line "lcd_profile" "$lcd_profile"
    write_profile_env_line "display_size_inch" "$display_size_inch"
    write_profile_env_line "lcd_profile_source" "$lcd_profile_source"
    write_profile_env_line "uart_overlay" "$uart_overlay"
    write_profile_env_line "panel_overlay" "$panel_overlay"
    write_profile_env_line "pa_protection" "$pa_protection"
    write_profile_env_line "ganymede_present" "$ganymede_present"
    write_profile_env_line "ganymede_device_path" "$ganymede_device_path"
    write_profile_env_line "atu" "$atu"
    write_profile_env_line "aries_present" "$aries_present"
    write_profile_env_line "aries_device_path" "$aries_device_path"
  } >"$SATURN_PROFILE_ENV_FILE"
  chmod 0644 "$SATURN_PROFILE_ENV_FILE"
  log "Wrote provisioning profile: $SATURN_PROFILE_ENV_FILE"
}

install_update_manager() {
  local saturn_home="$1"
  local script="$SATURN_REPO_DIR/update_manager/install_saturn_go_nginx.sh"
  local p2app_panel_enabled=0
  case "${SATURN_FRONT_PANEL_TYPE:-}" in
    G2V1|G2V2) p2app_panel_enabled=1 ;;
  esac
  if [[ ! -x "$script" ]]; then
    die "Update manager installer not found/executable: $script"
  fi
  assert_not_repo_python_script "$script"
  log "Installing Saturn Update Manager"
  env \
    HOME="$saturn_home" \
    SUDO_USER="$SATURN_USER" \
    SATURN_SERVICE_USER="$SATURN_USER" \
    SATURN_SERVICE_GROUP="$(id -gn "$SATURN_USER")" \
    SATURN_REPO_ROOT="$SATURN_REPO_DIR" \
    SATURN_UPDATE_MANAGER_SOURCE_DIR="$SATURN_REPO_DIR/update_manager" \
    SATURN_INSTALL_PACKAGES=0 \
    SATURN_INSTALL_LEGACY_XDMA_HOOK=0 \
    SATURN_ADMIN_PASSWORD="$SATURN_ADMIN_PASSWORD" \
    SATURN_INSTALL_BRIDGE=0 \
    SATURN_REQUIRE_BRIDGE=0 \
    SATURN_READY_REQUIRE_BRIDGE="$SATURN_REQUIRE_SATURN_BRIDGE" \
    SATURN_P2APP_PANEL_ENABLED="$p2app_panel_enabled" \
    SATURN_DEFER_FINAL_READINESS=1 \
    bash "$script"
}

install_pihpsdr_runtime() {
  local saturn_home="$1"
  local script="/opt/saturn-go/scripts/update-pihpsdr.py"

  [[ -f "$script" ]] || die "Installed piHPSDR updater not found: $script"
  log "Installing piHPSDR runtime and native DSP libraries required by Saturn Remote"
  env \
    HOME="$saturn_home" \
    SUDO_USER="$SATURN_USER" \
    PYTHONDONTWRITEBYTECODE=1 \
    /usr/bin/python3 "$script" -y --verbose

  if [[ -d "$saturn_home/github/pihpsdr" ]]; then
    chown -R "$SATURN_USER:$SATURN_GROUP" "$saturn_home/github/pihpsdr" || true
  fi

  [[ -f "$saturn_home/github/pihpsdr/wdsp/libwdsp.a" ]] || die "piHPSDR build did not produce wdsp/libwdsp.a"
  [[ -f "$saturn_home/github/pihpsdr/rnnoise/librnnoise.a" ]] || die "piHPSDR build did not produce rnnoise/librnnoise.a"
  [[ -f "$saturn_home/github/pihpsdr/libspecbleach/libspecbleach.a" ]] || die "piHPSDR build did not produce libspecbleach/libspecbleach.a"
}

install_saturn_bridge_runtime() {
  local saturn_home="$1"
  local script="$SATURN_REPO_DIR/update_manager/scripts/install-saturn-bridge.sh"

  [[ -x "$script" ]] || die "Saturn Bridge installer not found/executable: $script"
  log "Installing Saturn Bridge backend for Saturn Remote"
  env \
    HOME="$saturn_home" \
    SUDO_USER="$SATURN_USER" \
    SATURN_USER="$SATURN_USER" \
    SATURN_REPO_ROOT="$SATURN_REPO_DIR" \
    SATURN_BRIDGE_WDSP_FLAVOR=wdsp2 \
    SATURN_BRIDGE_SOURCE_DIR="$SATURN_REPO_DIR/update_manager/saturn-bridge" \
    SATURN_GO_ROOT="/opt/saturn-go" \
    SATURN_INSTALL_PACKAGES=0 \
    SATURN_BRIDGE_RF_TX_ENABLED="${SATURN_BRIDGE_RF_TX_ENABLED:-1}" \
    SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED="${SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED:-1}" \
    bash "$script"
}

verify_saturn_go_target_readiness() {
  local expected_commit ready_url healthy=0
  expected_commit="$(git -C "$SATURN_REPO_DIR" rev-parse HEAD 2>/dev/null || true)"
  [[ "$expected_commit" =~ ^[0-9a-fA-F]{40}$ ]] \
    || die "Could not resolve the full Saturn commit for final readiness verification"
  expected_commit="${expected_commit,,}"
  ready_url="http://127.0.0.1:8080/readyz?expected_commit=${expected_commit}"

  log "Verifying target-aware Saturn Go readiness after dependent services are installed"
  for _ in {1..40}; do
    if curl -fsS --max-time 3 "$ready_url" >/dev/null 2>&1; then
      healthy=1
      break
    fi
    sleep 0.25
  done
  if (( healthy == 0 )); then
    systemctl --no-pager --full status saturn-go.service saturn-bridge.service || true
    journalctl -u saturn-go.service -n 40 --no-pager || true
    die "Saturn Go readiness failed for commit ${expected_commit} after dependency installation"
  fi
  log "Saturn Go is ready at commit ${expected_commit}"
}

ensure_update_manager_admin_password() {
  local password_file password_helper status_output
  local current_umask
  local prompted_password generated_password=0
  password_file="${SATURN_UPDATE_MANAGER_PASSWORD_FILE:-$UPDATE_MANAGER_PASSWORD_FILE_DEFAULT}"

  if [[ -n "$SATURN_ADMIN_PASSWORD" ]]; then
    if [[ ${#SATURN_ADMIN_PASSWORD} -lt $SATURN_ADMIN_PASSWORD_MIN_LEN ]]; then
      die "SATURN_ADMIN_PASSWORD must be at least ${SATURN_ADMIN_PASSWORD_MIN_LEN} characters."
    fi
    install -d -m 0755 "$(dirname "$password_file")"
    current_umask="$(umask)"
    umask 077
    printf '%s\n' "$SATURN_ADMIN_PASSWORD" >"$password_file"
    umask "$current_umask"
    chmod 0600 "$password_file"
    log "Using SATURN_ADMIN_PASSWORD from environment/config."
    log "Stored update-manager admin password at $password_file (root-only)."
    return 0
  fi

  # An upgrade may have a valid existing htpasswd/drop-in but no recoverable
  # plaintext. Do not reset that credential merely to satisfy provisioning.
  # The nested Saturn Go installer will reuse both existing auth backends.
  password_helper="$SATURN_REPO_DIR/scripts/saturn-admin-password.sh"
  if [[ -s /etc/nginx/.htpasswd \
      && -s /etc/systemd/system/saturn-go.service.d/10-remote-auth.conf \
      && -x "$password_helper" ]]; then
    status_output="$("$password_helper" status 2>/dev/null || true)"
    if grep -q '^sync_state=in_sync$' <<<"$status_output"; then
      SATURN_ADMIN_PASSWORD=""
      log "Reusing synchronized Saturn Go credentials without changing the password."
      return 0
    fi
    log "Existing Saturn Go credential backends are not synchronized; selecting a replacement password."
  fi
  if [[ -s /etc/nginx/.htpasswd ]]; then
    log "Existing nginx credentials are missing the Saturn Remote counterpart; a new five-character password will repair both backends."
  fi

  if [[ -s "$password_file" ]]; then
    SATURN_ADMIN_PASSWORD="$(head -n 1 "$password_file")"
    if [[ ${#SATURN_ADMIN_PASSWORD} -lt $SATURN_ADMIN_PASSWORD_MIN_LEN ]]; then
      log "Ignoring an inactive legacy password file shorter than ${SATURN_ADMIN_PASSWORD_MIN_LEN} characters."
      SATURN_ADMIN_PASSWORD=""
    else
      log "Reusing existing update-manager admin password from $password_file."
      return 0
    fi
  fi

  if ! bool_true "$SATURN_NONINTERACTIVE" && [[ -t 0 ]]; then
    printf 'Choose a Saturn password (at least five characters), or press Enter to generate one: ' >&2
    IFS= read -r prompted_password
    if [[ -n "$prompted_password" ]]; then
      if [[ ${#prompted_password} -lt $SATURN_ADMIN_PASSWORD_MIN_LEN ]]; then
        die "The Saturn password must be at least ${SATURN_ADMIN_PASSWORD_MIN_LEN} characters."
      fi
      SATURN_ADMIN_PASSWORD="$prompted_password"
    fi
  fi
  if [[ -z "$SATURN_ADMIN_PASSWORD" ]]; then
    SATURN_ADMIN_PASSWORD="$(generate_password)"
    generated_password=1
  fi
  install -d -m 0755 "$(dirname "$password_file")"
  current_umask="$(umask)"
  umask 077
  printf '%s\n' "$SATURN_ADMIN_PASSWORD" >"$password_file"
  umask "$current_umask"
  chmod 0600 "$password_file"
  log "Stored initial update-manager admin password at $password_file (root-only)."
  if (( generated_password )); then
    log "Generated a device-specific Saturn Go password; retrieve it from $password_file as root."
  fi
}

maybe_flash_fpga() {
  local flash_script="$SATURN_REPO_DIR/update_manager/scripts/flash_fpga.sh"
  [[ -x "$flash_script" ]] || die "flash_fpga.sh not found/executable: $flash_script"
  assert_not_repo_python_script "$flash_script"
  [[ -n "$SATURN_FLASH_CONFIRM" ]] || die "SATURN_FLASH_CONFIRM is required when SATURN_FLASH_FPGA=1"

  local cmd=(bash "$flash_script" --confirm "$SATURN_FLASH_CONFIRM")
  if [[ "$SATURN_FLASH_IMAGE" == "latest" ]]; then
    cmd+=(--latest)
  else
    cmd+=(--image "$SATURN_FLASH_IMAGE")
  fi
  if bool_true "$SATURN_FLASH_FALLBACK"; then
    cmd+=(--fallback)
  else
    cmd+=(--primary)
  fi

  log "Flashing FPGA using load-FPGA"
  "${cmd[@]}"
}

cleanup_python_artifacts_in_repo() {
  [[ -n "${SATURN_REPO_DIR:-}" && -d "$SATURN_REPO_DIR" ]] || return 0
  find "$SATURN_REPO_DIR" -type d -name "__pycache__" -prune -exec rm -rf {} + 2>/dev/null || true
  find "$SATURN_REPO_DIR" -type f \( -name "*.pyc" -o -name "*.pyo" \) -delete 2>/dev/null || true
}

cleanup_tmp_artifacts() {
  if ! bool_true "$SATURN_CLEAN_TMP_AFTER_PROVISION"; then
    return 0
  fi

  log "Cleaning Saturn temporary artifacts under /tmp"
  find /tmp -mindepth 1 -maxdepth 1 \
    \( -name 'saturn-*' -o -name 'p2app-*' -o -name 'p3-*' -o -name 'xdma_make.log' -o -name 'update-desktop-apps-test.log' \) \
    -exec rm -rf {} + 2>/dev/null || true
}

verify_install() {
  local mode="${SATURN_VERIFY_MODE:-hardware}"
  local failed=0

  case "$mode" in
    none)
      log "Install verification disabled (SATURN_VERIFY_MODE=none)."
      return 0
      ;;
    software|hardware) ;;
    *) die "Unknown SATURN_VERIFY_MODE '$mode' (use software, hardware, or none)." ;;
  esac

  verify_file() {
    local path="$1" label="$2"
    if [[ ! -e "$path" ]]; then
      log "VERIFY FAIL: $label is missing: $path"
      failed=1
    fi
  }
  verify_enabled() {
    local unit="$1"
    if ! systemctl is-enabled --quiet "$unit"; then
      log "VERIFY FAIL: $unit is not enabled"
      failed=1
    fi
  }

  if bool_true "$SATURN_REBUILD_XDMA"; then
    if ! command -v dkms >/dev/null 2>&1 \
        || ! dkms status -m saturn-xdma 2>/dev/null | grep -F "$(uname -r)" | grep -q 'installed'; then
      log "VERIFY FAIL: DKMS does not report Saturn XDMA installed for $(uname -r)"
      failed=1
    fi
    if [[ -e /etc/kernel/postinst.d/saturn-xdma || -L /etc/kernel/postinst.d/saturn-xdma ]]; then
      log "VERIFY FAIL: legacy XDMA post-install hook is active alongside DKMS"
      failed=1
    fi
    if ! id -nG "$SATURN_USER" 2>/dev/null | tr ' ' '\n' | grep -Fxq saturn-radio; then
      log "VERIFY FAIL: $SATURN_USER is not a member of saturn-radio for local XDMA applications"
      failed=1
    fi
  fi

  if bool_true "$SATURN_INSTALL_CLOUD_INIT" \
      && ! command -v cloud-init >/dev/null 2>&1; then
    log "VERIFY FAIL: cloud-init is required by the image-factory profile"
    failed=1
  fi

  if bool_true "$SATURN_INSTALL_P2APP_CONTROL"; then
    verify_file /opt/saturn-radio/bin/p2app "production P2 binary"
    verify_file /etc/systemd/system/p2app.service "P2 systemd unit"
    verify_file /usr/local/lib/saturn-go/scripts/saturn-p2-deploy.sh "trusted P2 deploy helper"
    verify_enabled p2app.service
    if [[ -x /opt/saturn-radio/bin/p2app ]] \
        && [[ "$(stat -c '%U:%G' /opt/saturn-radio/bin/p2app)" != root:root ]]; then
      log "VERIFY FAIL: production P2 binary is not root-owned"
      failed=1
    fi
  fi

  if bool_true "$SATURN_INSTALL_UPDATE_MANAGER"; then
    verify_file /opt/saturn-go/bin/saturn-go "Saturn Go binary"
    verify_file /etc/systemd/system/saturn-go.service "Saturn Go systemd unit"
    verify_file /etc/nginx/.htpasswd "Saturn Go authentication file"
    verify_file /etc/sudoers.d/saturn-go-maintenance "Saturn Go sudoers policy"
    verify_enabled saturn-go.service
    if ! nginx -t >/dev/null 2>&1; then
      log "VERIFY FAIL: nginx configuration test failed"
      failed=1
    fi
    if command -v visudo >/dev/null 2>&1 \
        && ! visudo -cf /etc/sudoers.d/saturn-go-maintenance >/dev/null 2>&1; then
      log "VERIFY FAIL: Saturn Go sudoers policy is invalid"
      failed=1
    fi
    if [[ ! -x /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh ]] \
        || ! /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh status \
          | grep -q '^sync_state=in_sync$'; then
      log "VERIFY FAIL: nginx and Saturn Remote credentials are not synchronized"
      failed=1
    fi
  fi

  if bool_true "$SATURN_INSTALL_SATURN_BRIDGE"; then
    verify_file /opt/saturn-go/bin/saturn-bridge "Saturn Bridge binary"
    verify_file /etc/systemd/system/saturn-bridge.service "Saturn Bridge systemd unit"
    verify_enabled saturn-bridge.service
  fi

  if [[ "$mode" == hardware ]]; then
    if bool_true "$SATURN_REBUILD_XDMA" \
        && [[ ! -c /dev/xdma0_user && ! -c /dev/xdma/card0/user ]]; then
      log "VERIFY FAIL: XDMA user device is unavailable"
      failed=1
    fi
    if bool_true "$SATURN_INSTALL_P2APP_CONTROL" \
        && ! systemctl is-active --quiet p2app.service; then
      log "VERIFY FAIL: p2app.service is not active"
      failed=1
    fi
    if bool_true "$SATURN_INSTALL_UPDATE_MANAGER"; then
      systemctl is-active --quiet saturn-go.service || {
        log "VERIFY FAIL: saturn-go.service is not active"
        failed=1
      }
      curl -fsS --max-time 3 http://127.0.0.1:8080/readyz >/dev/null || {
        log "VERIFY FAIL: Saturn Go readiness endpoint failed"
        failed=1
      }
    fi
    if bool_true "$SATURN_INSTALL_SATURN_BRIDGE"; then
      systemctl is-active --quiet saturn-bridge.service || {
        log "VERIFY FAIL: saturn-bridge.service is not active"
        failed=1
      }
      if command -v ss >/dev/null 2>&1 && ! ss -ltn | grep -q ':50001 '; then
        log "VERIFY FAIL: Saturn Bridge TCI listener is unavailable"
        failed=1
      fi
    fi
  fi

  (( failed == 0 )) || die "$mode installation verification failed"
  log "$mode installation verification passed"
}

write_completion_state() {
  local saturn_home="$1"
  local commit contract hardware_model hardware_platform_vendor hardware_module_family hardware_storage_variant
  commit="$(run_as_user "$saturn_home" git -C "$SATURN_REPO_DIR" rev-parse --short HEAD 2>/dev/null || true)"
  contract="$(installer_contract_hash)"
  hardware_model="$(read_device_tree_model 2>/dev/null || true)"
  hardware_platform_vendor="$(detect_platform_vendor 2>/dev/null || true)"
  hardware_module_family="$(detect_module_family 2>/dev/null || true)"
  hardware_storage_variant="$(detect_module_storage_variant 2>/dev/null || true)"
  install -d -m 0755 "$SATURN_STATE_DIR"
  cat > "${SATURN_STATE_DIR}/complete" <<EOF
completed_at=$(date --iso-8601=seconds)
installer_version=${SATURN_HOST_SCHEMA_VERSION}
install_contract=${contract}
install_profile=${SATURN_INSTALL_PROFILE}
verification_mode=${SATURN_VERIFY_MODE}
saturn_user=${SATURN_USER}
repo_url=${SATURN_REPO_URL}
repo_branch=${SATURN_REPO_BRANCH}
repo_ref=${SATURN_REPO_REF:-${SATURN_REPO_BRANCH}}
repo_dir=${SATURN_REPO_DIR}
repo_commit=${commit:-unknown}
hardware_model=${hardware_model:-unknown}
hardware_platform_vendor=${hardware_platform_vendor:-unknown}
hardware_module_family=${hardware_module_family:-unknown}
hardware_storage_variant=${hardware_storage_variant:-unknown}
front_panel_type=${SATURN_FRONT_PANEL_TYPE:-unknown}
xdma_present=${SATURN_XDMA_PRESENT:-unknown}
system_role=${SATURN_SYSTEM_ROLE:-unknown}
xdma_policy=dkms
EOF
  cat > /etc/saturn-release <<EOF
host_schema=${SATURN_HOST_SCHEMA_VERSION}
repo_commit=${commit:-unknown}
install_profile=${SATURN_INSTALL_PROFILE}
installed_at=$(date --iso-8601=seconds)
EOF
  chmod 0644 /etc/saturn-release
}

installer_contract_hash() {
  local commit
  commit="$(git -C "$SATURN_REPO_DIR" rev-parse HEAD 2>/dev/null || printf unknown)"
  {
    printf '%s\n' \
      "installer=$SATURN_HOST_SCHEMA_VERSION" \
      "commit=$commit" \
      "kernel=$(uname -r)" \
      "user=$SATURN_USER" \
      "group=$SATURN_GROUP" \
      "repo_dir=$SATURN_REPO_DIR" \
      "profile=$SATURN_INSTALL_PROFILE" \
      "packages=$SATURN_INSTALL_PACKAGES" \
      "driver=$SATURN_REBUILD_XDMA" \
      "p2=$SATURN_INSTALL_P2APP_CONTROL" \
      "p2_tests=$SATURN_RUN_P2_TESTS" \
      "go=$SATURN_INSTALL_UPDATE_MANAGER" \
      "bridge=$SATURN_INSTALL_SATURN_BRIDGE" \
      "bridge_verify=${SATURN_BRIDGE_VERIFY_RUNTIME:-1}" \
      "pihpsdr=$SATURN_INSTALL_PIHPSDR" \
      "pihpsdr_installer=$SATURN_PIHPSDR_INSTALLER_ENABLED" \
      "udev=$SATURN_INSTALL_UDEV_RULES" \
      "shutdown_waiter=$SATURN_INSTALL_SHUTDOWN_WAITER" \
      "developer_tools=$SATURN_INSTALL_DEVELOPER_TOOLS" \
      "cloud_init=$SATURN_INSTALL_CLOUD_INIT" \
      "optional_tools=$SATURN_BUILD_OPTIONAL_TOOLS" \
      "i2c=$SATURN_ENABLE_I2C" \
      "ssh=$SATURN_ENABLE_SSH" \
      "vnc=$SATURN_ENABLE_VNC" \
      "lcd=$SATURN_LCD_PROFILE" \
      "front_panel_detect=$SATURN_DETECT_FRONT_PANEL" \
      "forced_role=$SATURN_FORCE_SYSTEM_ROLE" \
      "flash=$SATURN_FLASH_FPGA" \
      "flash_image=$SATURN_FLASH_IMAGE" \
      "flash_fallback=$SATURN_FLASH_FALLBACK" \
      "verify=$SATURN_VERIFY_MODE"
  } | sha256sum | awk '{print substr($1,1,16)}'
}

phase_contract_hash() {
  local commit
  commit="$(git -C "$SATURN_REPO_DIR" rev-parse HEAD 2>/dev/null || printf unknown)"
  {
    printf '%s\n' \
      "installer=$SATURN_HOST_SCHEMA_VERSION" \
      "commit=$commit" \
      "kernel=$(uname -r)" \
      "user=$SATURN_USER" \
      "group=$SATURN_GROUP" \
      "repo_dir=$SATURN_REPO_DIR" \
      "profile=$SATURN_INSTALL_PROFILE" \
      "packages=$SATURN_INSTALL_PACKAGES" \
      "driver=$SATURN_REBUILD_XDMA" \
      "p2=$SATURN_INSTALL_P2APP_CONTROL" \
      "p2_tests=$SATURN_RUN_P2_TESTS" \
      "go=$SATURN_INSTALL_UPDATE_MANAGER" \
      "bridge=$SATURN_INSTALL_SATURN_BRIDGE" \
      "bridge_verify=${SATURN_BRIDGE_VERIFY_RUNTIME:-1}" \
      "pihpsdr=$SATURN_INSTALL_PIHPSDR" \
      "pihpsdr_installer=$SATURN_PIHPSDR_INSTALLER_ENABLED" \
      "cloud_init=$SATURN_INSTALL_CLOUD_INIT" \
      "i2c=$SATURN_ENABLE_I2C" \
      "ssh=$SATURN_ENABLE_SSH" \
      "vnc=$SATURN_ENABLE_VNC" \
      "optional_tools=$SATURN_BUILD_OPTIONAL_TOOLS"
  } | sha256sum | awk '{print substr($1,1,16)}'
}

prepare_phase_state() {
  local contract
  contract="$(phase_contract_hash)"
  SATURN_PHASE_RUN_DIR="${SATURN_STATE_DIR}/phases/${contract}"
  if bool_true "$SATURN_FORCE_REPROVISION"; then
    rm -rf "$SATURN_PHASE_RUN_DIR"
  fi
  install -d -m 0755 "$SATURN_PHASE_RUN_DIR"
  export SATURN_PHASE_RUN_DIR
}

run_phase() {
  local name="$1" label="$2"
  shift 2
  local marker="${SATURN_PHASE_RUN_DIR}/${name}.complete"
  local current="${SATURN_STATE_DIR}/current-phase"

  if bool_true "$SATURN_RESUME" && [[ -f "$marker" ]]; then
    log "Resume: phase '$name' already completed; skipping."
    return 0
  fi

  set_ui_stage "$label"
  printf '%s|%s\n' "$name" "$(date --iso-8601=seconds)" >"$current"
  "$@"
  printf 'completed_at=%s\n' "$(date --iso-8601=seconds)" >"$marker"
  rm -f "$current"
}

phase_packages() {
  if bool_true "$SATURN_INSTALL_PACKAGES"; then
    ensure_packages
  else
    log "Skipping package installation (SATURN_INSTALL_PACKAGES=0)"
  fi
}

phase_repository() {
  local saturn_home="$1"
  ensure_repo "$saturn_home"
}

phase_build() {
  local saturn_home="$1" nproc="$2"
  enable_python_repo_guard
  prepare_python_env "$saturn_home"
  build_saturn_apps "$saturn_home" "$nproc"
}

phase_driver() {
  local saturn_home="$1"
  if bool_true "$SATURN_REBUILD_XDMA"; then
    build_and_install_xdma "$saturn_home"
  else
    log "Skipping XDMA installation (SATURN_REBUILD_XDMA=0)"
  fi
}

phase_p2() {
  local saturn_home="$1"
  if bool_true "$SATURN_INSTALL_P2APP_CONTROL"; then
    install_p2app_control "$saturn_home"
  else
    log "Skipping Protocol 2 runtime installation"
  fi
}

phase_saturn_go() {
  local saturn_home="$1"
  if ! bool_true "$SATURN_INSTALL_UPDATE_MANAGER"; then
    log "Skipping Saturn Go installation"
    return 0
  fi

  ensure_update_manager_admin_password
  install_update_manager "$saturn_home"
  if bool_true "$SATURN_INSTALL_PIHPSDR"; then
    install_pihpsdr_runtime "$saturn_home"
  fi
  if bool_true "$SATURN_INSTALL_SATURN_BRIDGE"; then
    if bool_true "$SATURN_REQUIRE_SATURN_BRIDGE"; then
      install_saturn_bridge_runtime "$saturn_home"
    elif ! ( install_saturn_bridge_runtime "$saturn_home" ); then
      log "WARN: Saturn Bridge install failed and SATURN_REQUIRE_SATURN_BRIDGE=0; continuing provisioning."
    fi
  fi
  verify_saturn_go_target_readiness
  install_pihpsdr_installer_shortcut "$saturn_home"
}

main() {
  local log_dir saturn_home nproc installed_contract current_contract
  ensure_root
  ensure_tmp_permissions
  log_dir="$(dirname "$SATURN_LOG_FILE")"
  ensure_dir_present "$log_dir" 0755
  ensure_dir_present "$SATURN_STATE_DIR" 0755
  ensure_dir_present "$PYTHONPYCACHEPREFIX" 0755
  touch "$SATURN_LOG_FILE"
  exec > >(tee -a "$SATURN_LOG_FILE") 2>&1

  trap cleanup_python_guard EXIT
  trap 'handle_error "$LINENO" "${BASH_COMMAND}"' ERR

  set_ui_stage "Resolving Saturn user account"
  saturn_home="$(ensure_user)"
  if [[ -z "$SATURN_GROUP" ]]; then
    SATURN_GROUP="$(id -gn "$SATURN_USER")"
  fi
  getent group "$SATURN_GROUP" >/dev/null || die "group does not exist: $SATURN_GROUP"
  if [[ -z "$SATURN_REPO_DIR" ]]; then
    SATURN_REPO_DIR="${saturn_home}/github/Saturn"
  fi
  export SATURN_GROUP SATURN_REPO_DIR
  nproc="$(nproc 2>/dev/null || echo 1)"

  current_contract="$(installer_contract_hash)"
  if [[ -f "${SATURN_STATE_DIR}/complete" ]] && ! bool_true "$SATURN_FORCE_REPROVISION"; then
    installed_contract="$(awk -F= '$1 == "install_contract" {print $2; exit}' "${SATURN_STATE_DIR}/complete")"
    if [[ -n "$installed_contract" && "$installed_contract" == "$current_contract" ]]; then
      remove_desktop_ui_autostart "$saturn_home"
      write_ui_status "SKIPPED" "This installer contract is already applied."
      log "This Saturn installer contract is already applied; no provisioning run is needed."
      exit 0
    fi
    log "Installed contract differs from the current host/repository/options; applying it now."
  fi

  prepare_phase_state

  run_phase packages "Installing build/runtime dependencies" phase_packages
  if desktop_ui_enabled; then
    set_ui_stage "Installing desktop power helper"
    install_desktop_ui_power_helper
  else
    log "Skipping desktop provisioning power helper (desktop UI disabled)"
  fi
  set_ui_stage "Preparing desktop provisioning UI autostart"
  install_desktop_ui_autostart "$saturn_home"
  set_ui_stage "Launching desktop provisioning interface"
  launch_desktop_ui "$saturn_home"

  log "Starting Saturn provisioning for user '$SATURN_USER' (home: $saturn_home)"
  set_ui_stage "Configuring USB boot and input tuning"
  configure_usb_boot_tweaks
  set_ui_stage "Configuring I2C, SSH, and VNC"
  configure_i2c_vnc_ssh
  if bool_true "$SATURN_DETECT_FRONT_PANEL"; then
    set_ui_stage "Detecting front panel"
    detect_front_panel pre-udev
  fi
  if bool_true "$SATURN_INSTALL_UDEV_RULES"; then
    set_ui_stage "Installing udev rules"
    install_udev_rules
  fi
  if bool_true "$SATURN_DETECT_FRONT_PANEL" && bool_true "$SATURN_INSTALL_UDEV_RULES"; then
    verify_front_panel_after_udev
  fi
  set_ui_stage "Resolving hardware role"
  resolve_system_role
  set_ui_stage "Applying LCD boot profile"
  configure_lcd_profile
  if bool_true "$SATURN_INSTALL_DEVELOPER_TOOLS"; then
    set_ui_stage "Installing developer desktop tools"
    install_desktop_dev_tools "$saturn_home"
  fi
  if [[ "$SATURN_UI_STARTED" -eq 0 ]]; then
    launch_desktop_ui "$saturn_home"
  fi

  run_phase repository "Preparing Saturn repository" phase_repository "$saturn_home"
  run_phase build "Building Saturn applications and tools" phase_build "$saturn_home" "$nproc"
  set_ui_stage "Installing desktop launchers"
  install_desktop_shortcuts "$saturn_home"
  if bool_true "$SATURN_INSTALL_SHUTDOWN_WAITER"; then
    set_ui_stage "Installing shutdown waiter service"
    install_shutdown_waiter
  fi
  set_ui_stage "Configuring power-switch LED"
  install_led_power_button_fix

  run_phase driver "Installing DKMS-managed XDMA module" phase_driver "$saturn_home"
  run_phase p2 "Installing Protocol 2 runtime" phase_p2 "$saturn_home"
  run_phase saturn-go "Installing Saturn Go and Saturn Bridge" phase_saturn_go "$saturn_home"
  if bool_true "$SATURN_FLASH_FPGA"; then
    set_ui_stage "Flashing FPGA image"
    maybe_flash_fpga
  fi

  set_ui_stage "Verifying installed appliance"
  verify_install
  set_ui_stage "Finalizing provisioning state"
  cleanup_python_artifacts_in_repo
  write_profile_env_state
  write_completion_state "$saturn_home"
  remove_desktop_ui_autostart "$saturn_home"
  set_ui_stage "Cleaning temporary files"
  cleanup_tmp_artifacts
  write_ui_status "SUCCESS" "Provisioning completed successfully"
  log "Saturn provisioning completed successfully."
  log "State file: ${SATURN_STATE_DIR}/complete"
  log "Provision log: $SATURN_LOG_FILE"
}

main "$@"
