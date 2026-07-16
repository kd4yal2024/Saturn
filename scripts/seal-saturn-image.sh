#!/usr/bin/env bash
# Remove per-device identity before capturing a Saturn golden image.

set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONFIRM=""
POWEROFF=1
KEEP_BUILD_CACHE=0
STATE_DIR="/var/lib/saturn-state"
PROVISION_STATE_DIR="/var/lib/saturn-provision"
FIRST_BOOT_INSTALL="/usr/local/lib/saturn-go/scripts/saturn-first-boot.sh"
FIRST_BOOT_UNIT="/etc/systemd/system/saturn-first-boot.service"
PASSWORD_HELPER="/usr/local/lib/saturn-go/scripts/saturn-admin-password.sh"
SEALED_HOSTNAME_FILE="$PROVISION_STATE_DIR/sealed-hostname"

info(){ printf '[saturn-image-seal] %s\n' "$*"; }
die(){ printf '[saturn-image-seal] ERROR: %s\n' "$*" >&2; exit 1; }

usage(){
  cat <<'EOF'
Usage: sudo scripts/seal-saturn-image.sh --confirm SEAL [options]

Options:
  --no-poweroff       Leave the system running after sealing (image builders only)
  --keep-build-cache  Retain Cargo/npm/native build caches
  -h, --help          Show this help

The next boot generates unique machine/SSH identity and a five-character
Saturn Go password. The login is written to /var/lib/saturn-state/initial-login.txt.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --confirm) [[ $# -ge 2 ]] || die "--confirm requires a value"; CONFIRM="$2"; shift 2 ;;
    --no-poweroff) POWEROFF=0; shift ;;
    --keep-build-cache) KEEP_BUILD_CACHE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[[ ${EUID:-$(id -u)} -eq 0 ]] || die "run as root"
[[ "$CONFIRM" == SEAL ]] || die "refusing to seal without --confirm SEAL"
[[ -f "$PROVISION_STATE_DIR/complete" ]] || die "Saturn provisioning completion state is missing"
install_profile="$(awk -F= '$1 == "install_profile" {print $2; exit}' "$PROVISION_STATE_DIR/complete")"
[[ "$install_profile" == image-factory ]] || \
  die "the completed install profile is '${install_profile:-unknown}', not image-factory"
command -v cloud-init >/dev/null 2>&1 || die "cloud-init is required for golden-image sealing"
[[ -x "$PASSWORD_HELPER" ]] || die "Saturn password helper is missing; install Saturn Go before sealing"
command -v usermod >/dev/null 2>&1 || die "usermod is required for golden-image sealing"

saturn_user="$(awk -F= '$1 == "saturn_user" {print $2; exit}' "$PROVISION_STATE_DIR/complete")"
[[ -n "$saturn_user" ]] || saturn_user=pi
id -u "$saturn_user" >/dev/null 2>&1 || die "provisioned Saturn user no longer exists: $saturn_user"
saturn_group="$(id -gn "$saturn_user" 2>/dev/null || printf '%s' "$saturn_user")"
saturn_home="$(getent passwd "$saturn_user" | cut -d: -f6)"
[[ -n "$saturn_home" && -d "$saturn_home" ]] || die "Saturn user home is unavailable: $saturn_user"

info "Stopping Saturn and identity-bearing services"
systemctl stop saturn-bridge.service p2app.service saturn-go.service nginx.service tailscaled.service bluetooth.service >/dev/null 2>&1 || true
# Stopping this unit writes its final seed now; the file is removed below so
# shutdown cannot repopulate a shared seed after sealing.
systemctl stop systemd-random-seed.service >/dev/null 2>&1 || true

install -D -m 0755 -o root -g root "$REPO_ROOT/scripts/saturn-first-boot.sh" "$FIRST_BOOT_INSTALL"
printf 'sealed_at=%s\n' "$(date --iso-8601=seconds)" >"$PROVISION_STATE_DIR/golden-image"
chmod 0644 "$PROVISION_STATE_DIR/golden-image"
cat >"$FIRST_BOOT_UNIT" <<EOF
[Unit]
Description=Saturn Golden Image First-Boot Personalization
After=local-fs.target cloud-config.service
Before=saturn-go.service nginx.service ssh.service tailscaled.service
ConditionPathExists=!${STATE_DIR}/first-boot.complete

[Service]
Type=oneshot
ExecStart=${FIRST_BOOT_INSTALL}
Environment=SATURN_SERVICE_USER=${saturn_user}
Environment=SATURN_SERVICE_GROUP=${saturn_group}
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF
chmod 0644 "$FIRST_BOOT_UNIT"
systemctl daemon-reload
systemctl enable saturn-first-boot.service >/dev/null

info "Removing per-device credentials and identity"
hostname >"$SEALED_HOSTNAME_FILE"
chmod 0644 "$SEALED_HOSTNAME_FILE"
rm -f \
  "$STATE_DIR/first-boot.complete" \
  "$STATE_DIR/initial-login.txt" \
  "$STATE_DIR/custom_scripts.json" \
  "$STATE_DIR/remote_profiles.json" \
  "$STATE_DIR/remote_settings.json" \
  "$STATE_DIR/saturngo_deploy_status.json" \
  "$STATE_DIR/update_state.json" \
  "$PROVISION_STATE_DIR/update-manager-admin-password" \
  /etc/nginx/.htpasswd \
  /etc/systemd/system/saturn-go.service.d/10-remote-auth.conf \
  /etc/ssh/ssh_host_*
rm -rf "$STATE_DIR/remote-tls"
rm -rf "$STATE_DIR/snapshots" "$STATE_DIR/repo-staging"
find /var/lib/tailscale -mindepth 1 -maxdepth 1 -exec rm -rf -- {} + 2>/dev/null || true
if [[ -f /etc/default/saturn-provision ]]; then
  sed -i -E 's/^SATURN_ADMIN_PASSWORD=.*/SATURN_ADMIN_PASSWORD=/' /etc/default/saturn-provision
fi

# Do not publish the image builder's wireless credentials or cloud-init network
# seed. Ethernet DHCP remains available; recipients can inject their own Wi-Fi
# settings with Raspberry Pi Imager or a new cloud-init seed.
if [[ -d /etc/NetworkManager/system-connections ]]; then
  for network_profile in /etc/NetworkManager/system-connections/*; do
    [[ -f "$network_profile" ]] || continue
    if grep -qE '^type=(wifi|802-11-wireless)$|^\[wifi\]$' "$network_profile"; then
      rm -f "$network_profile"
    fi
  done
fi
rm -rf /var/lib/iwd /var/lib/bluetooth
rm -f \
  /var/lib/systemd/random-seed \
  /var/lib/private/systemd/random-seed \
  /etc/wpa_supplicant/wpa_supplicant.conf \
  /boot/wpa_supplicant.conf \
  /boot/firmware/wpa_supplicant.conf \
  /boot/network-config \
  /boot/firmware/network-config

if [[ -n "$saturn_home" ]]; then
  rm -rf \
    "$saturn_home/.ssh" \
    "$saturn_home/.cache" \
    "$saturn_home/.config/gh" \
    "$saturn_home/.config/chromium" \
    "$saturn_home/.config/google-chrome" \
    "$saturn_home/.config/Code" \
    "$saturn_home/.mozilla" \
    "$saturn_home/.local/share/keyrings"
  rm -f \
    "$saturn_home/.bash_history" \
    "$saturn_home/.python_history" \
    "$saturn_home/.git-credentials" \
    "$saturn_home/.netrc" \
    "$saturn_home/.npmrc" \
    "$saturn_home/.cargo/credentials" \
    "$saturn_home/.cargo/credentials.toml"
  if [[ -d "$saturn_home/saturn-logs" ]]; then
    find "$saturn_home/saturn-logs" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
  fi
fi
rm -rf /root/.ssh /root/.config/gh
rm -f \
  /root/.bash_history \
  /root/.python_history \
  /root/.git-credentials \
  /root/.netrc \
  /root/.npmrc \
  /root/.cargo/credentials \
  /root/.cargo/credentials.toml \
  /var/log/saturn-provision.log \
  /var/log/saturn-cloudinit-bootstrap.log

# Erase password hashes rather than merely prefix-locking them; first boot
# assigns a device-unique login to the normal Saturn user. Root remains locked.
usermod --password '!' "$saturn_user"
usermod --password '!' root

if (( ! KEEP_BUILD_CACHE )); then
  info "Removing reproducible build caches"
  rm -rf \
    "$REPO_ROOT/update_manager/rust-server/target-local" \
    "$REPO_ROOT/update_manager/saturn-bridge/target-local" \
    "$REPO_ROOT/update_manager/remote-web/node_modules"
fi

find /tmp -mindepth 1 -maxdepth 1 -exec rm -rf {} +
journalctl --rotate >/dev/null 2>&1 || true
journalctl --vacuum-time=1s >/dev/null 2>&1 || true
cloud-init clean --logs --seed --machine-id
sync

info "Image sealed. The next boot will create unique identity and credentials."
if (( POWEROFF )); then
  info "Powering off now; capture the image before booting it again."
  systemctl poweroff
else
  info "WARNING: power off before capturing the image. Do not reboot this source image first."
fi
