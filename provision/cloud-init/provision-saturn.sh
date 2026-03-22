#!/usr/bin/env bash
set -Eeuo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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
SATURN_USER_RETRY_SECONDS="${SATURN_USER_RETRY_SECONDS:-30}"
SATURN_REPO_URL="${SATURN_REPO_URL:-https://github.com/kd4yal2024/Saturn.git}"
SATURN_REPO_BRANCH="${SATURN_REPO_BRANCH:-main}"
SATURN_REPO_DIR="${SATURN_REPO_DIR:-}"

SATURN_INSTALL_UPDATE_MANAGER="${SATURN_INSTALL_UPDATE_MANAGER:-1}"
SATURN_INSTALL_P2APP_CONTROL="${SATURN_INSTALL_P2APP_CONTROL:-1}"
SATURN_INSTALL_UDEV_RULES="${SATURN_INSTALL_UDEV_RULES:-1}"
SATURN_INSTALL_SHUTDOWN_WAITER="${SATURN_INSTALL_SHUTDOWN_WAITER:-1}"
SATURN_REBUILD_XDMA="${SATURN_REBUILD_XDMA:-1}"
SATURN_BUILD_OPTIONAL_TOOLS="${SATURN_BUILD_OPTIONAL_TOOLS:-1}"
SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT="${SATURN_SHUTDOWN_WAITER_ENABLED_DEFAULT:-auto}"

SATURN_FLASH_FPGA="${SATURN_FLASH_FPGA:-0}"
SATURN_FLASH_IMAGE="${SATURN_FLASH_IMAGE:-latest}"
SATURN_FLASH_FALLBACK="${SATURN_FLASH_FALLBACK:-0}"
SATURN_FLASH_CONFIRM="${SATURN_FLASH_CONFIRM:-}"

SATURN_ADMIN_PASSWORD="${SATURN_ADMIN_PASSWORD:-admin}"
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

apt_updated=0
SATURN_UI_STARTED=0
SATURN_UI_AUTOSTART_INSTALLED=0
PYTHON_GUARD_DIR=""
UPDATE_MANAGER_PASSWORD_FILE_DEFAULT="/var/lib/saturn-provision/update-manager-admin-password"

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

  install -d -m 0755 -o "$SATURN_USER" -g "$SATURN_USER" "$autostart_dir"

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
  chown "$SATURN_USER:$SATURN_USER" "$desktop_file"
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

launch_desktop_ui() {
  local saturn_home="$1"
  local display xauth user_uid runtime_dir dbus_address ui_cmd_escaped
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
  printf -v ui_cmd_escaped '%q ' "${ui_cmd[@]}"

  if run_as_user "$saturn_home" env DISPLAY="$display" XAUTHORITY="$xauth" XDG_RUNTIME_DIR="$runtime_dir" DBUS_SESSION_BUS_ADDRESS="$dbus_address" \
    bash -lc "if pgrep -x saturn-provision-ui >/dev/null 2>&1; then exit 0; fi; nohup ${ui_cmd_escaped} >/tmp/saturn-provision-ui.log 2>&1 < /dev/null &"; then
    SATURN_UI_STARTED=1
    log "Desktop provisioning UI launched (detached, DISPLAY=$display)."
  else
    log "WARN: Failed to launch desktop provisioning UI."
  fi
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
  local charset='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789@#%^+=_'
  local charset_len="${#charset}"
  local password=""
  local hex idx

  # Avoid pipelines here: with `set -o pipefail`, `tr | head` can fail on SIGPIPE.
  while [[ ${#password} -lt 24 ]]; do
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
  local saturn_home=""
  local retry_seconds
  retry_seconds="${SATURN_USER_RETRY_SECONDS:-30}"
  if ! [[ "$retry_seconds" =~ ^[0-9]+$ ]] || (( retry_seconds < 1 )); then
    retry_seconds=30
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
    sleep "$retry_seconds"
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
      set +e
      apt-get "$@" 2>&1
      rc=$?
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

ensure_ui_packages() {
  log "Installing desktop provisioning UI prerequisites"
  apt_install g++ pkg-config libgtk-3-dev
}

ensure_packages() {
  local running_krel running_meta_pkg
  running_krel="$(uname -r)"
  running_meta_pkg="linux-headers-$(kernel_flavor "$running_krel")"

  log "Installing build/runtime dependencies"
  apt_install \
    git rsync curl wget ca-certificates sudo \
    build-essential pkg-config gcc g++ make \
    python3 python3-venv python3-pip python3-psutil \
    gpiod libgpiod-dev libi2c-dev libgtk-3-dev libglib2.0-bin lxterminal \
    libasound2-dev libpulse-dev libusb-1.0-0-dev libcurl4-openssl-dev \
    desktop-file-utils xdg-user-dirs

  if bool_true "$SATURN_INSTALL_P2APP_CONTROL"; then
    apt_install libayatana-appindicator3-dev ayatana-indicator-application
  fi

  if bool_true "$SATURN_INSTALL_UPDATE_MANAGER"; then
    apt_install nginx apache2-utils
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

ensure_kernel_headers() {
  local krel build_dir meta_pkg
  krel="$(uname -r)"
  build_dir="/lib/modules/${krel}/build"
  meta_pkg="linux-headers-$(kernel_flavor "$krel")"
  if [[ -d "$build_dir" ]]; then
    log "Kernel headers already present for $krel"
    return
  fi

  log "Installing kernel headers for $krel"
  apt_update_once
  export DEBIAN_FRONTEND=noninteractive
  if apt-cache show "linux-headers-${krel}" >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends "linux-headers-${krel}"
  elif apt-cache show "${meta_pkg}" >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends "${meta_pkg}"
  elif apt-cache show raspberrypi-kernel-headers >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends raspberrypi-kernel-headers
  else
    die "No suitable kernel header package found for $krel"
  fi
  [[ -d "$build_dir" ]] || die "Kernel headers still missing at $build_dir"
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

get_boot_config_file() {
  local candidate
  for candidate in /boot/firmware/config.txt /boot/config.txt; do
    if [[ -f "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
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

detect_compute_module_generation() {
  local model
  model="$(tr -d '\0' < /proc/device-tree/model 2>/dev/null || true)"
  case "$model" in
    *"Compute Module 5"*) printf 'cm5\n' ;;
    *"Compute Module 4"*) printf 'cm4\n' ;;
    *) return 1 ;;
  esac
}

detect_lcd_size_from_config() {
  local boot_config="$1"
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-800x480([[:space:]]*,.*)?$' "$boot_config"; then
    printf '7\n'
    return
  fi
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,.*' "$boot_config"; then
    printf '7\n'
    return
  fi
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,.*' "$boot_config"; then
    printf '8\n'
    return
  fi
}

detect_lcd_profile_from_config() {
  local boot_config="$1"

  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1([[:space:]]*#.*)?$' "$boot_config" \
    && grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0([[:space:]]*#.*)?$' "$boot_config"; then
    printf 'cm5-7-g2-dual-dsi\n'
    return
  fi
}

i2c_address_detected() {
  local bus="$1"
  local addr="${2:-0x45}"
  local addr_dec addr_hex out

  if ! command -v i2cdetect >/dev/null 2>&1; then
    printf '0\n'
    return
  fi
  if [[ ! -e "/dev/i2c-${bus}" ]]; then
    printf '0\n'
    return
  fi

  if ! addr_dec=$((addr)); then
    printf '0\n'
    return
  fi
  if (( addr_dec < 0x08 || addr_dec > 0x77 )); then
    printf '0\n'
    return
  fi
  addr_hex="$(printf '%02x' "$addr_dec")"
  out="$(i2cdetect -y "$bus" "$addr_dec" "$addr_dec" 2>/dev/null || true)"
  if grep -Eq "(^|[[:space:]])(UU|${addr_hex})([[:space:]]|$)" <<<"$out"; then
    printf '1\n'
  else
    printf '0\n'
  fi
}

detect_lcd_size_from_i2c_probe() {
  local detect_addr="${SATURN_LCD_I2C_DETECT_ADDR:-0x45}"
  local bus0_has=0 bus1_has=0 bus10_has=0

  if [[ "$(i2c_address_detected 0 "$detect_addr")" == "1" ]]; then
    bus0_has=1
  fi
  if [[ "$(i2c_address_detected 1 "$detect_addr")" == "1" ]]; then
    bus1_has=1
  fi
  if [[ "$(i2c_address_detected 10 "$detect_addr")" == "1" ]]; then
    bus10_has=1
  fi

  if [[ "$bus1_has" -eq 1 && "$bus0_has" -eq 0 && "$bus10_has" -eq 0 ]]; then
    printf '8\n'
    return
  fi
  if [[ "$bus10_has" -eq 1 && "$bus1_has" -eq 0 ]]; then
    printf '7\n'
    return
  fi
  if [[ "$bus0_has" -eq 1 && "$bus1_has" -eq 0 && "$bus10_has" -eq 0 ]]; then
    printf '7\n'
    return
  fi
}

detect_lcd_size_auto() {
  local boot_config="$1"
  local size=""

  size="${SATURN_LCD_SIZE_INCH:-}"
  case "$size" in
    7|8)
      printf '%s|env\n' "$size"
      return 0
      ;;
    "")
      ;;
    *)
      log "WARN: Invalid SATURN_LCD_SIZE_INCH='$size'; expected 7 or 8."
      ;;
  esac

  if [[ -n "$boot_config" ]]; then
    size="$(detect_lcd_size_from_config "$boot_config" 2>/dev/null || true)"
    case "$size" in
      7|8)
        printf '%s|config\n' "$size"
        return 0
        ;;
    esac
  fi

  size="$(detect_lcd_size_from_i2c_probe 2>/dev/null || true)"
  case "$size" in
    7|8)
      printf '%s|i2c-probe\n' "$size"
      return 0
      ;;
  esac

  size="${SATURN_LCD_AUTO_DEFAULT_SIZE_INCH:-}"
  case "$size" in
    7|8)
      printf '%s|default\n' "$size"
      return
      ;;
  esac

  return
}

resolve_lcd_profile() {
  local boot_config="$1"
  local requested cm size size_source auto_detect_result
  requested="${SATURN_LCD_PROFILE,,}"

  case "$requested" in
    none|"")
      return 1
      ;;
    cm4-7|cm4-8|cm5-7|cm5-7-g2-dual-dsi|cm5-8)
      printf '%s\n' "$requested"
      return 0
      ;;
    auto)
      requested="$(detect_lcd_profile_from_config "$boot_config" 2>/dev/null || true)"
      if [[ -n "$requested" ]]; then
        log "Auto-selected LCD profile from existing config: '$requested'"
        printf '%s\n' "$requested"
        return 0
      fi
      cm="$(detect_compute_module_generation 2>/dev/null || true)"
      auto_detect_result="$(detect_lcd_size_auto "$boot_config" 2>/dev/null || true)"
      if [[ -n "$auto_detect_result" ]]; then
        IFS='|' read -r size size_source <<<"$auto_detect_result"
      else
        size=""
        size_source=""
      fi
      if [[ -z "$cm" || -z "$size" ]]; then
        log "WARN: SATURN_LCD_PROFILE=auto could not resolve a unique profile (cm='${cm:-unknown}', size='${size:-unknown}'). Set SATURN_LCD_PROFILE explicitly."
        return 1
      fi
      log "Auto-selected LCD profile inputs: cm='$cm', size='${size}' (source=${size_source:-unknown})"
      printf '%s-%s\n' "$cm" "$size"
      return 0
      ;;
    *)
      log "WARN: Unknown SATURN_LCD_PROFILE='$SATURN_LCD_PROFILE'; skipping LCD boot configuration."
      return 1
      ;;
  esac
}

render_lcd_profile_block() {
  local profile="$1"
  local uart_line panel_line

  case "$profile" in
    cm4-7)
      uart_line='dtoverlay=uart3'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-800x480'
      ;;
    cm4-8)
      uart_line='dtoverlay=uart3'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1'
      ;;
    cm5-7)
      uart_line='dtoverlay=uart2-pi5'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-800x480'
      ;;
    cm5-7-g2-dual-dsi)
      uart_line='dtoverlay=uart2-pi5'
      panel_line=$'dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1\ndtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0'
      ;;
    cm5-8)
      uart_line='dtoverlay=uart2-pi5'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1'
      ;;
    *)
      return 1
      ;;
  esac

  cat <<EOF
# Saturn managed LCD profile: $profile
dtparam=i2c_arm=on
dtparam=audio=on
auto_initramfs=1
dtoverlay=vc4-kms-v3d
max_framebuffers=2
disable_fw_kms_setup=1
arm_64bit=1
disable_overscan=1
arm_boost=1

[cm4]
otg_mode=1

[cm5]
dtoverlay=dwc2,dr_mode=host

[all]
dtparam=uart0=on
$uart_line
$panel_line
usb_max_current_enable=1
EOF
}

configure_lcd_profile() {
  local boot_config profile block

  if ! boot_config="$(get_boot_config_file 2>/dev/null)"; then
    log "WARN: Could not locate /boot/firmware/config.txt or /boot/config.txt for LCD profile setup."
    return 0
  fi

  profile="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"
  [[ -n "$profile" ]] || return 0

  if bool_true "$SATURN_LCD_DETECT_ONLY"; then
    log "SATURN_LCD_DETECT_ONLY=1 set; auto-detection resolved profile '$profile' and no config.txt changes were made."
    return 0
  fi

  if ! block="$(render_lcd_profile_block "$profile")"; then
    log "WARN: Failed to render LCD block for profile '$profile'; skipping."
    return 0
  fi

  # Remove legacy/foreign active panel overlays before applying managed block.
  sed -i -E '/^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare(-800x480|-panel,.*)[[:space:]]*$/d' "$boot_config"

  sed -i '/^# BEGIN SATURN LCD PROFILE$/,/^# END SATURN LCD PROFILE$/d' "$boot_config"
  {
    printf '\n# BEGIN SATURN LCD PROFILE\n'
    printf '# Managed by Saturn provisioning (non-destructive append)\n'
    printf '%s\n' "$block"
    printf '# END SATURN LCD PROFILE\n'
  } >>"$boot_config"

  log "Applied SATURN_LCD_PROFILE='$profile' to $boot_config"
  log "HDMI settings preserved (existing HDMI lines were not removed)."
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

  install -d -m 0755 -o "$SATURN_USER" -g "$SATURN_USER" "$repo_parent"

  if [[ -d "${SATURN_REPO_DIR}/.git" ]]; then
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

  build_dir "$saturn_home" "P2_app"      "$SATURN_REPO_DIR/sw_projects/P2_app" "$nproc" 1
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
  local fix_script="$SATURN_REPO_DIR/scripts/fix-xdma.sh"

  [[ -x "$fix_script" ]] || die "XDMA rebuild helper not found/executable: $fix_script"
  assert_not_repo_python_script "$fix_script"

  log "Building and installing XDMA kernel module via fix-xdma.sh"
  env \
    HOME="$saturn_home" \
    SUDO_USER="$SATURN_USER" \
    SATURN_USER="$SATURN_USER" \
    SATURN_REPO_DIR="$SATURN_REPO_DIR" \
    bash "$fix_script"

  install -d -m 0755 /etc/modules-load.d
  printf 'xdma\n' > /etc/modules-load.d/xdma.conf
  log "XDMA module installed, loaded, and configured for boot"
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
  local script="$SATURN_REPO_DIR/rules/install-rules.sh"
  if [[ -x "$script" ]]; then
    assert_not_repo_python_script "$script"
    log "Installing udev rules"
    bash "$script"
  else
    log "WARN: Missing udev script: $script"
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

install_p2app_control() {
  local saturn_home="$1"
  local script="$SATURN_REPO_DIR/sw_tools/p2app-control/install.sh"
  if [[ -x "$script" ]]; then
    assert_not_repo_python_script "$script"
    log "Installing p2app-control and p2app.service"
    env HOME="$saturn_home" SUDO_USER="$SATURN_USER" SATURN_USER="$SATURN_USER" bash "$script"
  else
    log "WARN: Missing p2app-control installer: $script"
  fi
}

install_update_manager() {
  local saturn_home="$1"
  local script="$SATURN_REPO_DIR/update_manager/install_saturn_go_nginx.sh"
  if [[ ! -x "$script" ]]; then
    die "Update manager installer not found/executable: $script"
  fi
  assert_not_repo_python_script "$script"
  log "Installing Saturn Update Manager"
  env \
    HOME="$saturn_home" \
    SUDO_USER="$SATURN_USER" \
    SATURN_SERVICE_USER="$SATURN_USER" \
    SATURN_ADMIN_PASSWORD="$SATURN_ADMIN_PASSWORD" \
    bash "$script"
}

ensure_update_manager_admin_password() {
  local password_file
  local current_umask
  password_file="${SATURN_UPDATE_MANAGER_PASSWORD_FILE:-$UPDATE_MANAGER_PASSWORD_FILE_DEFAULT}"

  if [[ -n "$SATURN_ADMIN_PASSWORD" ]]; then
    if [[ ${#SATURN_ADMIN_PASSWORD} -lt 5 ]]; then
      die "SATURN_ADMIN_PASSWORD must be at least 5 characters."
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

  if [[ -s "$password_file" ]]; then
    SATURN_ADMIN_PASSWORD="$(head -n 1 "$password_file")"
    if [[ ${#SATURN_ADMIN_PASSWORD} -lt 5 ]]; then
      die "Existing update-manager password file is invalid (too short): $password_file"
    fi
    log "Reusing existing update-manager admin password from $password_file."
    return 0
  fi

  SATURN_ADMIN_PASSWORD="$(generate_password)"
  install -d -m 0755 "$(dirname "$password_file")"
  current_umask="$(umask)"
  umask 077
  printf '%s\n' "$SATURN_ADMIN_PASSWORD" >"$password_file"
  umask "$current_umask"
  chmod 0600 "$password_file"
  log "Generated update-manager admin password and stored at $password_file (root-only)."
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

write_completion_state() {
  local saturn_home="$1"
  local commit
  commit="$(run_as_user "$saturn_home" git -C "$SATURN_REPO_DIR" rev-parse --short HEAD 2>/dev/null || true)"
  install -d -m 0755 "$SATURN_STATE_DIR"
  cat > "${SATURN_STATE_DIR}/complete" <<EOF
completed_at=$(date --iso-8601=seconds)
saturn_user=${SATURN_USER}
repo_url=${SATURN_REPO_URL}
repo_branch=${SATURN_REPO_BRANCH}
repo_dir=${SATURN_REPO_DIR}
repo_commit=${commit:-unknown}
EOF
}

main() {
  local log_dir
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

  if [[ -f "${SATURN_STATE_DIR}/complete" ]] && ! bool_true "$SATURN_FORCE_REPROVISION"; then
    local existing_home
    existing_home="$(getent passwd "$SATURN_USER" | cut -d: -f6 || true)"
    if [[ -n "$existing_home" ]]; then
      remove_desktop_ui_autostart "$existing_home"
    fi
    write_ui_status "SKIPPED" "Already provisioned. No provisioning run was executed."
    log "Provisioning already completed. Set SATURN_FORCE_REPROVISION=1 to run again."
    exit 0
  fi

  local saturn_home nproc
  set_ui_stage "Resolving Saturn user account"
  saturn_home="$(ensure_user)"
  nproc="$(nproc 2>/dev/null || echo 1)"

  set_ui_stage "Installing desktop provisioning UI prerequisites"
  ensure_ui_packages
  set_ui_stage "Preparing desktop provisioning UI autostart"
  install_desktop_ui_autostart "$saturn_home"
  set_ui_stage "Launching desktop provisioning interface"
  launch_desktop_ui "$saturn_home"

  log "Starting Saturn provisioning for user '$SATURN_USER' (home: $saturn_home)"
  set_ui_stage "Installing build/runtime dependencies"
  ensure_packages
  set_ui_stage "Configuring USB boot and input tuning"
  configure_usb_boot_tweaks
  set_ui_stage "Configuring I2C, SSH, and VNC"
  configure_i2c_vnc_ssh
  set_ui_stage "Applying LCD boot profile"
  configure_lcd_profile
  set_ui_stage "Installing developer desktop tools"
  install_desktop_dev_tools "$saturn_home"
  if [[ "$SATURN_UI_STARTED" -eq 0 ]]; then
    launch_desktop_ui "$saturn_home"
  fi

  set_ui_stage "Syncing Saturn repository"
  ensure_repo "$saturn_home"
  set_ui_stage "Enabling Python repo guard"
  enable_python_repo_guard
  set_ui_stage "Preparing Python virtual environment"
  prepare_python_env "$saturn_home"
  set_ui_stage "Building Saturn applications and tools"
  build_saturn_apps "$saturn_home" "$nproc"
  set_ui_stage "Installing desktop launchers"
  install_desktop_shortcuts "$saturn_home"
  if bool_true "$SATURN_INSTALL_SHUTDOWN_WAITER"; then
    set_ui_stage "Installing shutdown waiter service"
    install_shutdown_waiter
  fi

  if bool_true "$SATURN_REBUILD_XDMA"; then
    set_ui_stage "Building and installing XDMA module"
    build_and_install_xdma "$saturn_home"
  fi
  if bool_true "$SATURN_INSTALL_UDEV_RULES"; then
    set_ui_stage "Installing udev rules"
    install_udev_rules
  fi
  if bool_true "$SATURN_INSTALL_P2APP_CONTROL"; then
    set_ui_stage "Installing p2app-control service"
    install_p2app_control "$saturn_home"
  fi
  if bool_true "$SATURN_INSTALL_UPDATE_MANAGER"; then
    set_ui_stage "Installing Saturn update manager"
    ensure_update_manager_admin_password
    install_update_manager "$saturn_home"
  fi
  if bool_true "$SATURN_FLASH_FPGA"; then
    set_ui_stage "Flashing FPGA image"
    maybe_flash_fpga
  fi

  set_ui_stage "Finalizing provisioning state"
  cleanup_python_artifacts_in_repo
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
