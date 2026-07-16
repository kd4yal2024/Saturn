#!/usr/bin/env bash
# One-time identity and five-character credential setup for cloned golden images.

set -Eeuo pipefail

STATE_DIR="${SATURN_STATE_DIR:-/var/lib/saturn-state}"
MARKER="$STATE_DIR/first-boot.complete"
LOGIN_FILE="$STATE_DIR/initial-login.txt"
PASSWORD_HELPER="${SATURN_PASSWORD_HELPER:-/usr/local/lib/saturn-go/scripts/saturn-admin-password.sh}"
PROVISION_CONFIG="${SATURN_PROVISION_CONFIG:-/etc/default/saturn-provision}"
SEALED_HOSTNAME_FILE="${SATURN_SEALED_HOSTNAME_FILE:-/var/lib/saturn-provision/sealed-hostname}"
SERVICE_USER="${SATURN_SERVICE_USER:-pi}"
SERVICE_GROUP="${SATURN_SERVICE_GROUP:-$(id -gn "$SERVICE_USER" 2>/dev/null || printf pi)}"

log(){ printf '[saturn-first-boot] %s\n' "$*"; }
die(){ printf '[saturn-first-boot] ERROR: %s\n' "$*" >&2; exit 1; }

[[ ${EUID:-$(id -u)} -eq 0 ]] || die "run as root"
[[ -f "$MARKER" ]] && exit 0
[[ -x "$PASSWORD_HELPER" ]] || die "password helper is missing: $PASSWORD_HELPER"
id -u "$SERVICE_USER" >/dev/null 2>&1 || die "service user does not exist: $SERVICE_USER"
getent group "$SERVICE_GROUP" >/dev/null 2>&1 || die "service group does not exist: $SERVICE_GROUP"
command -v chpasswd >/dev/null 2>&1 || die "chpasswd is required for first-boot login setup"
command -v passwd >/dev/null 2>&1 || die "passwd is required for first-boot login setup"

generate_password(){
  local charset='abcdefghjkmnpqrstuvwxyz23456789'
  local password='' byte index
  while [[ ${#password} -lt 5 ]]; do
    byte="$(od -An -N1 -tu1 /dev/urandom | tr -d '[:space:]')"
    [[ -n "$byte" ]] || continue
    index=$((byte % ${#charset}))
    password+="${charset:index:1}"
  done
  printf '%s' "$password"
}

personalize_hostname(){
  local sealed_hostname current_hostname machine_id new_hostname
  [[ -s "$SEALED_HOSTNAME_FILE" ]] || return 0
  sealed_hostname="$(tr -d '\r\n' <"$SEALED_HOSTNAME_FILE")"
  current_hostname="$(hostname)"
  if [[ -z "$sealed_hostname" || "$current_hostname" != "$sealed_hostname" ]]; then
    log "Preserving customized hostname: $current_hostname"
    return 0
  fi

  machine_id="$(tr -dc 'a-fA-F0-9' </etc/machine-id 2>/dev/null || true)"
  if [[ ${#machine_id} -lt 8 ]]; then
    machine_id="$(od -An -N4 -tx1 /dev/urandom | tr -d '[:space:]')"
  fi
  new_hostname="saturn-${machine_id:0:8}"
  hostnamectl set-hostname "$new_hostname"
  if grep -qE '^127\.0\.1\.1[[:space:]]' /etc/hosts; then
    sed -i -E "s/^127\\.0\\.1\\.1[[:space:]].*/127.0.1.1\\t${new_hostname}/" /etc/hosts
  else
    printf '\n127.0.1.1\t%s\n' "$new_hostname" >>/etc/hosts
  fi
  log "Generated unique hostname: $new_hostname"
}

install -d -m 0750 -o "$SERVICE_USER" -g "$SERVICE_GROUP" "$STATE_DIR"
personalize_hostname
ssh-keygen -A
password="$(generate_password)"
printf '%s\n' "$password" | "$PASSWORD_HELPER" set --restart none
linux_password_state="$(passwd -S "$SERVICE_USER" | awk '{print $2}')"
linux_password_generated=0
case "$linux_password_state" in
  L|LK|NP)
    printf '%s:%s\n' "$SERVICE_USER" "$password" | chpasswd --crypt-method YESCRYPT
    linux_password_generated=1
    ;;
esac

{
  printf 'Saturn initial login\n'
  printf 'Hostname: %s\n' "$(hostname)"
  printf 'Saturn Go username: admin\n'
  printf 'Saturn Go password: %s\n' "$password"
  if (( linux_password_generated )); then
    printf 'Linux username: %s\n' "$SERVICE_USER"
    printf 'Linux initial password: %s\n' "$password"
  else
    printf 'Linux login: preserved from image/cloud-init customization\n'
  fi
  printf 'created: %s\n' "$(date --iso-8601=seconds)"
  printf 'change Saturn Go: sudo /usr/local/lib/saturn-go/scripts/saturn-admin-password.sh reset\n'
  printf 'change Linux login: passwd\n'
} >"$LOGIN_FILE"
chown root:"$SERVICE_GROUP" "$LOGIN_FILE"
chmod 0640 "$LOGIN_FILE"

# A cloud-init seed may have carried the image-builder's temporary password.
# Keep later manual installer runs from restoring that shared credential.
if [[ -f "$PROVISION_CONFIG" ]]; then
  sed -i -E 's/^SATURN_ADMIN_PASSWORD=.*/SATURN_ADMIN_PASSWORD=/' "$PROVISION_CONFIG"
fi

printf 'completed_at=%s\n' "$(date --iso-8601=seconds)" >"$MARKER"
chmod 0644 "$MARKER"
log "Initial login written to $LOGIN_FILE"
logger -t saturn-first-boot "Initial Saturn Go login is available in $LOGIN_FILE" || true

systemctl daemon-reload
systemctl try-restart nginx.service >/dev/null 2>&1 || true
