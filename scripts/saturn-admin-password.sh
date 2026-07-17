#!/usr/bin/env bash
# saturn-admin-password.sh
# Single source of truth for the Saturn admin password.
#
# The admin password lives in two backends that must never drift:
#   1. /etc/nginx/.htpasswd (LAN nginx basic auth; nginx reads it per-request,
#      so changes apply immediately with no reload)
#   2. the saturn-go systemd drop-in carrying SATURN_REMOTE_BASIC_AUTH (the
#      TLS listener caches this at first use, so saturn-go must restart to
#      pick up a change)
#
# Subcommands:
#   set [--restart deferred|now|none]
#       Read the new password from stdin (first line) and apply it to both
#       backends. Default --restart deferred schedules the saturn-go restart
#       ~2s out so a calling HTTP handler can finish its response first.
#   reset [--restart now|deferred|none]
#       Console recovery: generate a simple five-character password, apply it to both
#       backends, and print it. Physical/console access is the trust anchor;
#       there is no remote reset path by design.
#   status
#       Report backend presence and whether the two backends agree.

set -euo pipefail

ADMIN_USER="${SATURN_ADMIN_USER:-admin}"
HTPASSWD_FILE="${SATURN_ADMIN_HTPASSWD_FILE:-/etc/nginx/.htpasswd}"
SERVICE_NAME="${SATURN_ADMIN_SERVICE:-saturn-go.service}"
DROPIN_DIR="${SATURN_ADMIN_DROPIN_DIR:-/etc/systemd/system/${SERVICE_NAME}.d}"
DROPIN_FILE="$DROPIN_DIR/10-remote-auth.conf"
PASSWORD_MIN_LEN=5
GENERATED_PASSWORD_LEN=5
SKIP_SYSTEMD="${SATURN_ADMIN_SKIP_SYSTEMD:-0}"
HTPASSWD_OWNER="${SATURN_ADMIN_HTPASSWD_OWNER:-root:www-data}"
INITIAL_LOGIN_FILE="${SATURN_ADMIN_INITIAL_LOGIN_FILE:-/var/lib/saturn-state/initial-login.txt}"

info(){ printf '[INFO] %s\n' "$*"; }
ok(){ printf '[ OK ] %s\n' "$*"; }
warn(){ printf '[WARN] %s\n' "$*" >&2; }
die(){ printf '[ERR ] %s\n' "$*" >&2; exit 1; }

usage(){
  cat <<EOF
Usage:
  printf '%s\n' "new-password" | sudo saturn-admin-password.sh set
  sudo saturn-admin-password.sh reset
  sudo saturn-admin-password.sh status

Subcommands:
  set     Read password from stdin; update nginx htpasswd AND the Saturn
          Remote TLS auth drop-in together. --restart deferred (default),
          now, or none controls the saturn-go restart that applies the
          TLS-side change.
  reset   Generate a simple five-character password, apply it, and print it. Intended
          for console recovery when the password is forgotten.
  status  Show backend state and whether both backends hold the same
          password.

Rules: at least ${PASSWORD_MIN_LEN} characters; no composition rules. Control
characters are rejected because the drop-in is a systemd Environment= line.
EOF
}

# Escape values for a quoted systemd Environment= assignment.
systemd_env_escape(){
  local v="$1"
  v="${v//\\/\\\\}"
  v="${v//\"/\\\"}"
  v="${v//%/%%}"
  printf '%s' "$v"
}

# Inverse of systemd_env_escape (reverse order of operations).
systemd_env_unescape(){
  local v="$1"
  v="${v//%%/%}"
  v="${v//\\\"/\"}"
  v="${v//\\\\/\\}"
  printf '%s' "$v"
}

require_root(){
  [[ "$(id -u)" -eq 0 ]] || die "Run as root (or via the saturn-go sudoers policy)."
}

validate_password(){
  local pw="$1"
  [[ "${#pw}" -ge "$PASSWORD_MIN_LEN" ]] || die "Password must be at least ${PASSWORD_MIN_LEN} characters."
  if [[ "$pw" == *[[:cntrl:]]* ]]; then
    die "Password must not contain control characters."
  fi
}

write_htpasswd(){
  local pw="$1"
  local flags=(-i)
  [[ -f "$HTPASSWD_FILE" ]] || flags=(-i -c)
  printf '%s\n' "$pw" | htpasswd "${flags[@]}" "$HTPASSWD_FILE" "$ADMIN_USER" >/dev/null 2>&1 \
    || { warn "htpasswd update failed for $HTPASSWD_FILE"; return 1; }
  chown "$HTPASSWD_OWNER" "$HTPASSWD_FILE" 2>/dev/null || true
  chmod 0640 "$HTPASSWD_FILE"
}

write_dropin(){
  local pw="$1"
  local escaped
  escaped="$(systemd_env_escape "$pw")"
  install -d -m 0755 -o root -g root "$DROPIN_DIR" || return 1
  (
    umask 0177
    cat > "$DROPIN_FILE" <<EOF
# Managed by saturn-admin-password.sh
# Saturn Remote TLS listener basic-auth credentials. Kept aligned with
# $HTPASSWD_FILE so LAN and TLS accept the same admin password.
[Service]
Environment="SATURN_REMOTE_BASIC_AUTH=${ADMIN_USER}:${escaped}"
EOF
  ) || { warn "drop-in write failed for $DROPIN_FILE"; return 1; }
  chmod 0600 "$DROPIN_FILE"
  chown root:root "$DROPIN_FILE"
}

apply_restart(){
  local mode="$1"
  [[ "$SKIP_SYSTEMD" -eq 1 ]] && { warn "SATURN_ADMIN_SKIP_SYSTEMD=1; skipping daemon-reload/restart."; return 0; }
  systemctl daemon-reload || { warn "systemctl daemon-reload failed"; return 1; }
  case "$mode" in
    deferred)
      # Let a calling HTTP handler finish its response before the service
      # restarts under it. try-restart is a no-op if saturn-go is not running.
      systemd-run --collect --on-active=2 systemctl try-restart "$SERVICE_NAME" >/dev/null 2>&1 \
        || systemctl try-restart "$SERVICE_NAME" \
        || { warn "could not schedule $SERVICE_NAME restart"; return 1; }
      info "saturn-go restart scheduled (~2s) to apply the TLS auth change."
      ;;
    now)
      systemctl try-restart "$SERVICE_NAME" \
        || { warn "could not restart $SERVICE_NAME"; return 1; }
      ;;
    none)
      warn "Restart skipped; the TLS listener keeps the OLD password until $SERVICE_NAME restarts."
      ;;
    *) die "Unknown --restart mode: $mode" ;;
  esac
}

dropin_password(){
  # Extract and unescape the plaintext from the drop-in; empty output if absent.
  local line
  line="$(grep -o "SATURN_REMOTE_BASIC_AUTH=${ADMIN_USER}:.*\"" "$DROPIN_FILE" 2>/dev/null | head -1)" || true
  [[ -n "$line" ]] || return 0
  line="${line#SATURN_REMOTE_BASIC_AUTH="${ADMIN_USER}":}"
  line="${line%\"}"
  systemd_env_unescape "$line"
}

cmd_set(){
  local restart_mode="deferred"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --restart) [[ $# -ge 2 ]] || die "--restart requires a mode"; restart_mode="$2"; shift 2 ;;
      *) die "Unknown option for set: $1" ;;
    esac
  done
  require_root
  command -v htpasswd >/dev/null 2>&1 || die "htpasswd not found (install apache2-utils)."

  local pw
  IFS= read -r pw || true
  validate_password "$pw"

  # All-or-nothing: snapshot both backends, restore both on any failure so
  # LAN and TLS can never end up holding different passwords.
  local backup_dir
  backup_dir="$(mktemp -d)"
  [[ -f "$HTPASSWD_FILE" ]] && cp -a "$HTPASSWD_FILE" "$backup_dir/htpasswd"
  [[ -f "$DROPIN_FILE" ]] && cp -a "$DROPIN_FILE" "$backup_dir/dropin"

  rollback_auth(){
    warn "Rolling back to the previous password state."
    if [[ -f "$backup_dir/htpasswd" ]]; then
      cp -a "$backup_dir/htpasswd" "$HTPASSWD_FILE"
    else
      rm -f "$HTPASSWD_FILE"
    fi
    if [[ -f "$backup_dir/dropin" ]]; then
      cp -a "$backup_dir/dropin" "$DROPIN_FILE"
    else
      rm -f "$DROPIN_FILE"
    fi
    [[ "$SKIP_SYSTEMD" -eq 1 ]] || systemctl daemon-reload || true
    rm -rf "$backup_dir"
  }

  if ! write_htpasswd "$pw"; then
    rollback_auth
    die "htpasswd update failed; previous password still active everywhere."
  fi
  ok "nginx htpasswd updated ($HTPASSWD_FILE); effective immediately on LAN."

  if ! write_dropin "$pw"; then
    rollback_auth
    die "TLS drop-in update failed; previous password restored everywhere."
  fi
  ok "TLS auth drop-in updated ($DROPIN_FILE)."

  if ! apply_restart "$restart_mode"; then
    rollback_auth
    die "systemd reload/restart failed; previous password restored everywhere."
  fi

  rm -rf "$backup_dir"
  rm -f "$INITIAL_LOGIN_FILE"
  ok "Admin password set for user '$ADMIN_USER'."
}

cmd_reset(){
  local restart_mode="now"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --restart) [[ $# -ge 2 ]] || die "--restart requires a mode"; restart_mode="$2"; shift 2 ;;
      *) die "Unknown option for reset: $1" ;;
    esac
  done
  require_root
  command -v htpasswd >/dev/null 2>&1 || die "htpasswd not found (install apache2-utils)."

  local charset='abcdefghjkmnpqrstuvwxyz23456789'
  local pw='' byte index
  while [[ ${#pw} -lt $GENERATED_PASSWORD_LEN ]]; do
    byte="$(od -An -N1 -tu1 /dev/urandom | tr -d '[:space:]')"
    [[ -n "$byte" ]] || continue
    index=$((byte % ${#charset}))
    pw+="${charset:index:1}"
  done

  printf '%s\n' "$pw" | SATURN_ADMIN_SKIP_SYSTEMD="$SKIP_SYSTEMD" \
    SATURN_ADMIN_USER="$ADMIN_USER" \
    SATURN_ADMIN_HTPASSWD_FILE="$HTPASSWD_FILE" \
    SATURN_ADMIN_SERVICE="$SERVICE_NAME" \
    SATURN_ADMIN_DROPIN_DIR="$DROPIN_DIR" \
    "$0" set --restart "$restart_mode"

  printf '\n'
  ok "New admin password (user '$ADMIN_USER'):"
  printf '\n    %s\n\n' "$pw"
  info "Write this down. Change it in Saturn Go: System -> Custom Scripts -> Change Password."
}

cmd_status(){
  local htpasswd_exists=false dropin_exists=false sync_state="unknown"
  [[ -f "$HTPASSWD_FILE" ]] && grep -q "^${ADMIN_USER}:" "$HTPASSWD_FILE" 2>/dev/null && htpasswd_exists=true
  [[ -f "$DROPIN_FILE" ]] && dropin_exists=true

  if [[ "$htpasswd_exists" == true && "$dropin_exists" == true ]] \
      && command -v htpasswd >/dev/null 2>&1; then
    local pw
    pw="$(dropin_password)"
    if [[ -n "$pw" ]]; then
      if printf '%s\n' "$pw" | htpasswd -vi "$HTPASSWD_FILE" "$ADMIN_USER" >/dev/null 2>&1; then
        sync_state="in_sync"
      else
        sync_state="out_of_sync"
      fi
    fi
  fi

  printf 'admin_user=%s\n' "$ADMIN_USER"
  printf 'htpasswd_file=%s\n' "$HTPASSWD_FILE"
  printf 'htpasswd_entry_exists=%s\n' "$htpasswd_exists"
  printf 'dropin_file=%s\n' "$DROPIN_FILE"
  printf 'dropin_exists=%s\n' "$dropin_exists"
  printf 'sync_state=%s\n' "$sync_state"
}

main(){
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    set) cmd_set "$@" ;;
    reset) cmd_reset "$@" ;;
    status) cmd_status "$@" ;;
    -h|--help|help|"") usage; [[ "$cmd" == "" ]] && exit 1 || exit 0 ;;
    *) die "Unknown subcommand: $cmd (use set|reset|status)" ;;
  esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
