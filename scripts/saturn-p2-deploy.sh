#!/usr/bin/env bash
# Trusted production deployment broker for a P2 binary built by update-p2app.sh.

set -Eeuo pipefail

[[ $# -eq 0 ]] || {
  printf '[saturn-p2-deploy] ERROR: this broker accepts no arguments\n' >&2
  exit 2
}

CONFIG_FILE="${SATURN_P2_DEPLOY_CONFIG:-/etc/default/saturn-p2-deploy}"
[[ -r "$CONFIG_FILE" ]] || {
  printf '[saturn-p2-deploy] ERROR: configuration is missing: %s\n' "$CONFIG_FILE" >&2
  exit 1
}
[[ "$(stat -c '%U' "$CONFIG_FILE")" == root ]] || {
  printf '[saturn-p2-deploy] ERROR: configuration must be root-owned: %s\n' "$CONFIG_FILE" >&2
  exit 1
}
config_mode="$(stat -c '%a' "$CONFIG_FILE")"
(( (8#$config_mode & 8#022) == 0 )) || {
  printf '[saturn-p2-deploy] ERROR: configuration must not be group/other writable: %s\n' "$CONFIG_FILE" >&2
  exit 1
}
# shellcheck disable=SC1090
source "$CONFIG_FILE"

P2APP_SOURCE_BIN="${P2APP_SOURCE_BIN:?P2APP_SOURCE_BIN is required}"
P2APP_RUNTIME_BIN="${P2APP_RUNTIME_BIN:-/opt/saturn-radio/bin/p2app}"
P2APP_SERVICE="${P2APP_SERVICE:-p2app.service}"
P2APP_BUILD_USER="${P2APP_BUILD_USER:?P2APP_BUILD_USER is required}"
P2APP_START_TIMEOUT_SECONDS="${P2APP_START_TIMEOUT_SECONDS:-30}"
P2APP_STABLE_SECONDS="${P2APP_STABLE_SECONDS:-2}"

info(){ printf '[saturn-p2-deploy] %s\n' "$*"; }
die(){ printf '[saturn-p2-deploy] ERROR: %s\n' "$*" >&2; exit 1; }

[[ ${EUID:-$(id -u)} -eq 0 ]] || die "run as root"
[[ "$P2APP_START_TIMEOUT_SECONDS" =~ ^[0-9]+$ ]] || die "invalid start timeout"
(( P2APP_START_TIMEOUT_SECONDS >= 1 && P2APP_START_TIMEOUT_SECONDS <= 300 )) || die "start timeout must be 1..300 seconds"
[[ "$P2APP_STABLE_SECONDS" =~ ^[0-9]+$ ]] || die "invalid stability interval"
(( P2APP_STABLE_SECONDS >= 1 && P2APP_STABLE_SECONDS <= 10 )) || die "stability interval must be 1..10 seconds"
(( P2APP_STABLE_SECONDS <= P2APP_START_TIMEOUT_SECONDS )) || die "stability interval exceeds start timeout"

actual_source="$(readlink -f -- "$P2APP_SOURCE_BIN" 2>/dev/null || true)"
[[ -n "$actual_source" ]] || die "P2 source path cannot be resolved"
[[ -f "$actual_source" && ! -L "$P2APP_SOURCE_BIN" && -x "$actual_source" ]] || die "P2 source is not a regular executable"
[[ "$(stat -c '%U' "$actual_source")" == "$P2APP_BUILD_USER" ]] || die "P2 source is not owned by $P2APP_BUILD_USER"
[[ "$(od -An -N4 -tx1 "$actual_source" | tr -d '[:space:]')" == 7f454c46 ]] || die "P2 source is not an ELF binary"

runtime_dir="$(dirname "$P2APP_RUNTIME_BIN")"
backup_bin="${P2APP_RUNTIME_BIN}.previous"
had_runtime=0
service_was_active=0
deploy_complete=0

[[ -f "$P2APP_RUNTIME_BIN" ]] && had_runtime=1
if systemctl is-active --quiet "$P2APP_SERVICE"; then
  service_was_active=1
fi

rollback(){
  local rc="${1:-$?}"
  set +e
  if (( deploy_complete )); then
    return 0
  fi
  info "Deployment did not complete; restoring the previous runtime"
  systemctl stop "$P2APP_SERVICE" >/dev/null 2>&1 || true
  if (( had_runtime )) && [[ -f "$backup_bin" ]]; then
    install -m 0755 -o root -g root "$backup_bin" "$P2APP_RUNTIME_BIN"
  elif (( ! had_runtime )); then
    rm -f "$P2APP_RUNTIME_BIN"
  fi
  if (( service_was_active )); then
    systemctl start "$P2APP_SERVICE" >/dev/null 2>&1 || true
  fi
  exit "$rc"
}
trap 'rollback $?' ERR
trap 'rollback 130' INT
trap 'rollback 143' TERM

install -d -m 0755 -o root -g root "$runtime_dir"
if (( had_runtime )); then
  install -m 0755 -o root -g root "$P2APP_RUNTIME_BIN" "$backup_bin"
fi

info "Stopping $P2APP_SERVICE for safe RF deployment"
systemctl stop "$P2APP_SERVICE"
install -m 0755 -o root -g root "$actual_source" "$P2APP_RUNTIME_BIN"
systemctl start "$P2APP_SERVICE"

elapsed=0
stable=0
while (( elapsed < P2APP_START_TIMEOUT_SECONDS )); do
  # Count complete one-second active intervals. Checking immediately after
  # start would otherwise treat a two-sample observation as two stable seconds.
  sleep 1
  elapsed=$((elapsed + 1))
  if systemctl is-active --quiet "$P2APP_SERVICE"; then
    stable=$((stable + 1))
    if (( stable >= P2APP_STABLE_SECONDS )); then
      deploy_complete=1
      trap - ERR INT TERM
      info "Deployment verified: $(sha256sum "$P2APP_RUNTIME_BIN" | awk '{print $1}')"
      exit 0
    fi
  else
    stable=0
  fi
done

printf '[saturn-p2-deploy] ERROR: %s did not become active within %ss\n' \
  "$P2APP_SERVICE" "$P2APP_START_TIMEOUT_SECONDS" >&2
rollback 1
