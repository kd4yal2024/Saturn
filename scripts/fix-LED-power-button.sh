#!/usr/bin/env bash
# fix-LED-power-button.sh  (Bookworm & Trixie)
# Front-panel LED is on BCM15:
#   pinctrl set 15 op dh  -> RED
#   pinctrl set 15 op dl  -> WHITE
# This script:
#   1) (optionally) pins early-boot default to RED via config.txt
#   2) installs a systemd unit that sets RED at boot, WHITE on shutdown
# Logs under: journalctl -t fix-LED-power-button
#
# Env knobs:
#   EARLY_DEFAULT=0   # skip touching config.txt (default is 1)
#   SERVICE_NAME=gpio15-setup.service  # keep legacy name by default
# Written by: Jerry DeLong kd4yal

set -euo pipefail

SCRIPT_SELF="$(readlink -f "${BASH_SOURCE[0]:-$0}" 2>/dev/null || printf '%s\n' "${BASH_SOURCE[0]:-$0}")"
PRIVILEGED_SCRIPT_PATH="${SATURN_PRIVILEGED_SCRIPT_PATH:-/usr/local/lib/saturn-go/scripts/$(basename "$SCRIPT_SELF")}"
SERVICE_NAME="${SERVICE_NAME:-gpio15-setup.service}"
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}"
EARLY_DEFAULT="${EARLY_DEFAULT:-1}"
RASPBERRYPI_UTILS_REF="${RASPBERRYPI_UTILS_REF:-5edd399260b5081f9c1c96fc7f369b920d6732d1}"
SATURN_INSTALL_PACKAGES="${SATURN_INSTALL_PACKAGES:-1}"

log(){ echo "$1" | systemd-cat -t fix-LED-power-button; }
die(){ echo "$1" >&2; exit 1; }
has_tty(){ [[ -t 0 || -t 1 ]]; }
flag_enabled(){
  case "${1:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --service-name)
        shift
        [[ $# -gt 0 ]] || { echo "Missing value for --service-name" >&2; exit 1; }
        SERVICE_NAME="$1"
        SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}"
        shift
        ;;
      --early-default)
        shift
        [[ $# -gt 0 ]] || { echo "Missing value for --early-default" >&2; exit 1; }
        EARLY_DEFAULT="$1"
        shift
        ;;
      -h|--help)
        cat <<'EOF'
Usage: fix-LED-power-button.sh [options]
  --service-name <name>   Override systemd unit name (default: gpio15-setup.service)
  --early-default <0|1>   Control config.txt gpio=15 default (default: 1)
EOF
        exit 0
        ;;
      *)
        echo "Unknown argument: $1" >&2
        exit 1
        ;;
    esac
  done
}

parse_args "$@"
[[ "$SERVICE_NAME" =~ ^[a-zA-Z0-9][a-zA-Z0-9_.-]*\.service$ ]] || die "Invalid service name: $SERVICE_NAME"
[[ "$EARLY_DEFAULT" =~ ^[01]$ ]] || die "Invalid --early-default value: $EARLY_DEFAULT"
SCRIPT_ARGS=(--service-name "$SERVICE_NAME" --early-default "$EARLY_DEFAULT")

require_root() {
  if [ "$(id -u)" -ne 0 ]; then
    if ! command -v sudo >/dev/null 2>&1; then
      echo "Root privileges required. Install sudo or run this script as root." >&2
      log "error: sudo not available"
      exit 1
    fi

    local target="$PRIVILEGED_SCRIPT_PATH"
    if [[ ! -x "$target" ]]; then
      if has_tty; then
        target="$SCRIPT_SELF"
      else
        echo "Root privileges required. Installed privileged copy not found at $PRIVILEGED_SCRIPT_PATH." >&2
        log "error: privileged helper missing"
        exit 1
      fi
    fi

    if has_tty; then
      exec sudo "$target" "${SCRIPT_ARGS[@]}"
    fi
    exec sudo -n "$target" "${SCRIPT_ARGS[@]}"
  fi
}
require_root

# ----- locate config.txt for Bookworm/Trixie -----
CONFIG_TXT="/boot/firmware/config.txt"
[ -f "$CONFIG_TXT" ] || CONFIG_TXT="/boot/config.txt"
if [ ! -f "$CONFIG_TXT" ]; then
  log "warning: no config.txt found at /boot/firmware or /boot (continuing without early default)"
  EARLY_DEFAULT=0
fi

# ----- ensure pinctrl exists (/usr/bin/pinctrl) -----
ensure_pinctrl() {
  if command -v pinctrl >/dev/null 2>&1; then
    log "pinctrl present at $(command -v pinctrl)"
    return 0
  fi
  log "pinctrl missing; building from raspberrypi/utils (this takes a minute)"
  if flag_enabled "$SATURN_INSTALL_PACKAGES"; then
    apt-get update -y
    apt-get install -y --no-install-recommends git cmake build-essential device-tree-compiler libfdt-dev
  else
    local missing=()
    command -v git >/dev/null 2>&1 || missing+=(git)
    command -v cmake >/dev/null 2>&1 || missing+=(cmake)
    command -v make >/dev/null 2>&1 || missing+=(build-essential)
    command -v dtc >/dev/null 2>&1 || missing+=(device-tree-compiler)
    dpkg-query -W -f='${Status}' libfdt-dev 2>/dev/null | grep -q '^install ok installed$' \
      || missing+=(libfdt-dev)
    (( ${#missing[@]} == 0 )) || \
      die "Missing pinctrl build dependencies while SATURN_INSTALL_PACKAGES=0: ${missing[*]}"
  fi
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT
  git init -q "$tmpdir/utils"
  git -C "$tmpdir/utils" remote add origin https://github.com/raspberrypi/utils.git
  git -C "$tmpdir/utils" fetch --depth=1 origin "$RASPBERRYPI_UTILS_REF"
  git -C "$tmpdir/utils" checkout -q --detach FETCH_HEAD
  ( cd "$tmpdir/utils/pinctrl" && cmake . && make -j"$(nproc)" && make install )
  hash -r
  if ! command -v pinctrl >/dev/null 2>&1; then
    log "error: pinctrl install failed"
    exit 1
  fi
  log "pinctrl installed to $(command -v pinctrl)"
}
ensure_pinctrl

# ----- optional: set early-boot default so LED is RED before userspace -----
if [ "$EARLY_DEFAULT" = "1" ]; then
  if grep -Eq '^\s*gpio=15=' "$CONFIG_TXT"; then
    # normalize whatever was there to op,dh
    sed -i 's/^\s*gpio=15=.*/gpio=15=op,dh/' "$CONFIG_TXT"
    log "normalized existing gpio=15=… → gpio=15=op,dh in $(basename "$CONFIG_TXT")"
  else
    echo 'gpio=15=op,dh' >> "$CONFIG_TXT"
    log "appended gpio=15=op,dh to $(basename "$CONFIG_TXT")"
  fi
else
  log "EARLY_DEFAULT=0: leaving $(basename "$CONFIG_TXT") unchanged"
fi

# Don’t touch unrelated overlays (e.g., gpio-poweroff on 20/21) here.

# ----- install systemd unit: RED while up, WHITE on shutdown -----
cat > "$SERVICE_FILE" <<'EOF'
[Unit]
Description=Set Saturn front-panel LED on BCM15 (dh=red, dl=white)
DefaultDependencies=yes
After=local-fs.target
Before=shutdown.target halt.target poweroff.target

[Service]
Type=oneshot
RemainAfterExit=yes
# RED for normal operation
ExecStart=/usr/bin/pinctrl set 15 op dh
# WHITE when we are going down
ExecStopPost=/usr/bin/pinctrl set 15 op dl

[Install]
WantedBy=multi-user.target
EOF
chmod 0644 "$SERVICE_FILE"
log "installed $SERVICE_FILE"

# ----- enable & start -----
systemctl daemon-reload
systemctl enable --now "$SERVICE_NAME"

# ----- verify quickly -----
state="$(pinctrl get 15 || true)"
log "pinctrl get 15 → $state"

echo
echo "Done."
echo "• Early default set in: $CONFIG_TXT (gpio=15=op,dh)  [EARLY_DEFAULT=$EARLY_DEFAULT]"
echo "• Service: $SERVICE_NAME (RED on boot; WHITE on shutdown)"
echo "Check:  sudo systemctl status $SERVICE_NAME ; pinctrl get 15"
echo "If you want instant early RED from firmware, reboot once."
