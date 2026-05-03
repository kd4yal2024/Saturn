#!/usr/bin/env bash
# saturn-go-tailscale-serve.sh
# Configure Tailscale Serve in front of the Saturn Remote TLS listener.
#
# Hard safety gates (refuse, do not warn):
#   - tailscale CLI must be installed
#   - tailscaled.service must be running (warns if not enabled at boot)
#   - tailscale must be authenticated (BackendState=Running)
#   - tailnet hostname must not be a stock 'raspberrypi[-N]' name
#   - SATURN_REMOTE_BASIC_AUTH must be set + well-formed in saturn-go.service
#   - https://127.0.0.1:8443/remote-next must return 401 Unauthorized
#
# All gates must pass before `tailscale serve` is configured. The auth gate
# defends against a misconfigured tailnet exposing /remote-next unauthenticated.
#
# Documented contract: update_manager/docs/OPERATIONS_RUNBOOK.md
#                      "Secure Remote Access with Tailscale"

set -euo pipefail

SATURN_TLS_HOST="${SATURN_TLS_HOST:-127.0.0.1}"
SATURN_TLS_PORT="${SATURN_TLS_PORT:-8443}"
SATURN_TLS_PATH="${SATURN_TLS_PATH:-/remote-next}"
SATURN_SERVICE="${SATURN_SERVICE:-saturn-go.service}"
SERVE_HTTPS_PORT="${SERVE_HTTPS_PORT:-443}"
GENERIC_HOSTNAME_RE='^raspberrypi(-[0-9]+)?$'

bold() { printf "\e[1m%s\e[0m\n" "$*"; }
ok()   { printf "[OK] %s\n" "$*"; }
info() { printf "[INFO] %s\n" "$*"; }
warn() { printf "[WARN] %s\n" "$*" >&2; }
err()  { printf "[ERR] %s\n" "$*" >&2; }

usage() {
  cat <<EOF
Usage: sudo bash $(basename "$0")

Configures Tailscale Serve to expose the Saturn Remote TLS listener
(\${SATURN_TLS_HOST}:\${SATURN_TLS_PORT}, currently ${SATURN_TLS_HOST}:${SATURN_TLS_PORT})
on the tailnet over HTTPS port ${SERVE_HTTPS_PORT}.

Refuses to configure Serve unless every Saturn safety gate passes. See
docs/OPERATIONS_RUNBOOK.md "Secure Remote Access with Tailscale".

No flags are accepted in this version. To override defaults, set:
  SATURN_TLS_HOST   (default 127.0.0.1)
  SATURN_TLS_PORT   (default 8443)
  SATURN_TLS_PATH   (default /remote-next)
  SATURN_SERVICE    (default saturn-go.service)
  SERVE_HTTPS_PORT  (default 443)
EOF
}

case "${1:-}" in
  -h|--help) usage; exit 0 ;;
  "") ;;
  *) err "unknown argument: $1"; usage >&2; exit 2 ;;
esac

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  err "Run as root (sudo)."
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  err "python3 is required to parse 'tailscale status --json'"
  info "Install: sudo apt-get install -y python3"
  exit 1
fi

TMP_HEADERS="$(mktemp)"
TMP_BODY="$(mktemp)"
trap 'rm -f "$TMP_HEADERS" "$TMP_BODY"' EXIT

ts_json_field() {
  TS_PATH="$1" python3 -c '
import json, sys, os
try:
    data = json.load(sys.stdin)
except json.JSONDecodeError as e:
    sys.stderr.write("json parse error: {}\n".format(e))
    sys.exit(2)
path = os.environ.get("TS_PATH", "")
parts = path.split(".") if path else []
cur = data
for key in parts:
    if isinstance(cur, dict) and key in cur:
        cur = cur[key]
    else:
        sys.exit(0)
if cur is None:
    sys.exit(0)
print(cur)
'
}

check_tailscale_installed() {
  if ! command -v tailscale >/dev/null 2>&1; then
    err "tailscale CLI not found"
    cat >&2 <<EOF
Install Tailscale before running this script:
  curl -fsSL https://tailscale.com/install.sh | sh
or on Debian/Ubuntu:
  sudo apt-get install -y tailscale

Then authenticate (sudo tailscale up) and re-run this script.
EOF
    exit 1
  fi
  ok "tailscale CLI present: $(command -v tailscale)"
}

check_tailscaled_state() {
  if ! systemctl is-active --quiet tailscaled.service; then
    err "tailscaled.service is not running"
    info "Start it with: sudo systemctl start tailscaled.service"
    info "(or persist across reboots: sudo systemctl enable --now tailscaled.service)"
    exit 1
  fi
  if ! systemctl is-enabled --quiet tailscaled.service; then
    warn "tailscaled.service is not enabled at boot"
    warn "Serve config is persisted in tailscaled state, but the daemon must"
    warn "auto-start for the mapping to come back after reboot."
    warn "Enable with: sudo systemctl enable tailscaled.service"
  else
    ok "tailscaled.service is enabled and running"
  fi
}

TS_STATUS_JSON=""
fetch_tailscale_status() {
  local raw rc=0
  raw="$(tailscale status --json 2>&1)" || rc=$?
  if [[ $rc -ne 0 ]]; then
    if printf '%s' "$raw" | grep -qiE 'logged out|not logged in|needs login|NeedsLogin'; then
      info "tailscale is not logged in. Authenticate first:"
      info "  sudo tailscale up"
      info "Then re-run this script."
      exit 0
    fi
    err "'tailscale status --json' failed (exit $rc):"
    printf '%s\n' "$raw" | sed 's/^/  /' >&2
    exit 1
  fi
  TS_STATUS_JSON="$raw"
}

check_tailscale_auth_state() {
  local backend_state
  backend_state="$(printf '%s' "$TS_STATUS_JSON" | ts_json_field BackendState)"
  case "$backend_state" in
    Running)
      ok "tailscale BackendState=Running"
      ;;
    NeedsLogin|NoState|Stopped|"")
      info "tailscale is not authenticated (BackendState=${backend_state:-unknown})."
      info "Authenticate first:"
      info "  sudo tailscale up"
      info "Then re-run this script."
      exit 0
      ;;
    *)
      err "tailscale is not in BackendState=Running (got: $backend_state)"
      info "Saturn Remote Tailscale Serve only configures from a confirmed Running state."
      info "States like Starting, NeedsMachineAuth, or other transient/non-Running states"
      info "are not safe to proceed from. Wait for or correct the state and re-run."
      info "  tailscale status"
      exit 1
      ;;
  esac
}

check_hostname() {
  local hn
  hn="$(printf '%s' "$TS_STATUS_JSON" | ts_json_field Self.HostName)"
  if [[ -z "$hn" ]]; then
    err "could not read Self.HostName from 'tailscale status --json'"
    exit 1
  fi
  if [[ "$hn" =~ $GENERIC_HOSTNAME_RE ]]; then
    err "tailnet hostname is generic: $hn"
    info "A stock Raspberry Pi hostname collides with every other Pi on your tailnet."
    info "Set a meaningful, tailnet-unique hostname and re-run:"
    info "  sudo tailscale set --hostname=saturn-g2"
    info "  (or saturn-shack, kd4yal-saturn, etc.)"
    exit 1
  fi
  ok "tailnet hostname: $hn"
}

check_saturn_auth_static() {
  local env_blob
  if ! env_blob="$(systemctl show "$SATURN_SERVICE" -p Environment --value 2>/dev/null)"; then
    err "could not read environment from $SATURN_SERVICE"
    info "Verify with: sudo systemctl status $SATURN_SERVICE"
    exit 1
  fi
  local match value user pass
  match="$(printf '%s' "$env_blob" | tr ' ' '\n' | grep '^SATURN_REMOTE_BASIC_AUTH=' || true)"
  if [[ -z "$match" ]]; then
    err "SATURN_REMOTE_BASIC_AUTH is not set in $SATURN_SERVICE environment"
    info "Saturn Remote falls back to UNAUTHENTICATED when this is missing."
    info "Set it via a drop-in override and restart:"
    info "  sudo systemctl edit $SATURN_SERVICE"
    info "  Add under [Service]:"
    info "    Environment=SATURN_REMOTE_BASIC_AUTH=admin:CHOOSE-A-STRONG-PASSWORD"
    info "  sudo systemctl restart $SATURN_SERVICE"
    info "Refusing to configure Tailscale Serve while Saturn Remote auth is unconfigured."
    exit 1
  fi
  value="${match#SATURN_REMOTE_BASIC_AUTH=}"
  if [[ "$value" != *:* ]]; then
    err "SATURN_REMOTE_BASIC_AUTH is set but missing ':' separator"
    info "Expected format: SATURN_REMOTE_BASIC_AUTH=username:password"
    exit 1
  fi
  user="${value%%:*}"
  pass="${value#*:}"
  if [[ -z "$user" || -z "$pass" ]]; then
    err "SATURN_REMOTE_BASIC_AUTH has empty username or password"
    info "Expected format: SATURN_REMOTE_BASIC_AUTH=username:password (both non-empty)"
    exit 1
  fi
  ok "SATURN_REMOTE_BASIC_AUTH is set in $SATURN_SERVICE (format looks valid)"
}

check_listener_reachable() {
  if ! ss -ltn 2>/dev/null | awk -v p=":${SATURN_TLS_PORT}\$" '$4 ~ p { found=1 } END { exit !found }'; then
    err "saturn-go TLS listener not bound on port ${SATURN_TLS_PORT}"
    info "Check service: sudo systemctl status $SATURN_SERVICE"
    info "Check logs:    sudo journalctl -u $SATURN_SERVICE -n 50"
    exit 1
  fi
  ok "saturn-go TLS listener present on port ${SATURN_TLS_PORT}"
}

check_saturn_auth_runtime() {
  local url="https://${SATURN_TLS_HOST}:${SATURN_TLS_PORT}${SATURN_TLS_PATH}"
  local rc=0
  curl -k -sS -o "$TMP_BODY" -D "$TMP_HEADERS" --max-time 5 "$url" >/dev/null 2>&1 || rc=$?

  local code=""
  if [[ -s "$TMP_HEADERS" ]]; then
    code="$(awk 'NR==1 { print $2 }' "$TMP_HEADERS" 2>/dev/null || true)"
  fi
  [[ -z "$code" ]] && code=000

  case "$code" in
    401)
      ok "$url returns 401 (basic-auth gate is active)"
      ;;
    200)
      err "$url returned 200 WITHOUT credentials"
      err "Saturn Remote is exposing /remote-next unauthenticated."
      info "Static check passed but the running listener is not enforcing auth."
      info "Likely causes:"
      info "  - Service started before SATURN_REMOTE_BASIC_AUTH was added (restart it)"
      info "  - Env var loaded from a different unit-file path than expected"
      info "  - saturn-go binary is older than the auth gate"
      info "Verify what the running service has loaded:"
      info "  sudo systemctl show $SATURN_SERVICE -p Environment --value | tr ' ' '\\n' | grep BASIC_AUTH"
      info "Recent service logs:"
      info "  sudo journalctl -u $SATURN_SERVICE -n 20 | grep -i auth"
      info "Response headers (first 5 lines):"
      head -n 5 "$TMP_HEADERS" 2>/dev/null | sed 's/^/  /' >&2 || true
      info "Refusing to configure Tailscale Serve."
      exit 1
      ;;
    301|302|303|307|308)
      err "$url returned $code (redirect — expected 401)"
      info "The TLS listener should never redirect /remote-next; check rust-server config."
      info "Response headers (first 5 lines):"
      head -n 5 "$TMP_HEADERS" 2>/dev/null | sed 's/^/  /' >&2 || true
      exit 1
      ;;
    404)
      err "$url returned 404 — /remote-next is not registered on this build"
      info "The deployed saturn-go binary may pre-date /remote-next routing."
      info "Re-run the installer to refresh the binary and web assets:"
      info "  sudo bash $(dirname "$(readlink -f "$0")")/../install_saturn_go_nginx.sh"
      exit 1
      ;;
    5*)
      err "$url returned $code (server error)"
      info "Check $SATURN_SERVICE logs:"
      info "  sudo journalctl -u $SATURN_SERVICE -n 50"
      exit 1
      ;;
    000)
      err "could not connect to $url (curl exit=$rc)"
      info "saturn-go.service may be starting up or unhealthy:"
      info "  sudo systemctl status $SATURN_SERVICE"
      info "  sudo journalctl -u $SATURN_SERVICE -n 50"
      exit 1
      ;;
    *)
      err "$url returned unexpected status: $code"
      info "Expected 401 — refusing to configure Serve."
      info "Response headers (first 5 lines):"
      head -n 5 "$TMP_HEADERS" 2>/dev/null | sed 's/^/  /' >&2 || true
      exit 1
      ;;
  esac
}

configure_serve() {
  local target="https+insecure://${SATURN_TLS_HOST}:${SATURN_TLS_PORT}"
  info "Configuring: tailscale serve --bg --https=${SERVE_HTTPS_PORT} ${target}"
  local rc=0
  tailscale serve --bg --https="${SERVE_HTTPS_PORT}" "$target" || rc=$?
  if [[ $rc -ne 0 ]]; then
    err "'tailscale serve' configuration failed (exit $rc)"
    info "Current serve status:"
    tailscale serve status 2>&1 | sed 's/^/  /' >&2 || true
    exit 1
  fi
  ok "tailscale serve configured"
}

print_serve_status() {
  bold "[serve status]"
  tailscale serve status 2>&1 | sed 's/^/  /' || true
}

print_final_url() {
  local dns_name
  dns_name="$(printf '%s' "$TS_STATUS_JSON" | ts_json_field Self.DNSName)"
  dns_name="${dns_name%.}"
  echo
  if [[ -z "$dns_name" ]]; then
    warn "Could not derive tailnet DNS name from status JSON."
    warn "Run 'tailscale status' manually to find the URL for this node."
    return
  fi
  bold "Saturn Remote URL (over Tailscale):"
  printf "  https://%s%s\n" "$dns_name" "$SATURN_TLS_PATH"
  echo
  info "Authenticate with the SATURN_REMOTE_BASIC_AUTH credentials configured in $SATURN_SERVICE."
  info "RF TX is opt-in via SATURN_REMOTE_TX_RF_ENABLED in saturn-bridge.service environment."
  info "Tailscale Serve does NOT change either of those controls."
}

main() {
  bold "[saturn-go-tailscale-serve] Verifying safety gates before exposing Saturn Remote on the tailnet"
  check_tailscale_installed
  check_tailscaled_state
  fetch_tailscale_status
  check_tailscale_auth_state
  check_hostname
  check_saturn_auth_static
  check_listener_reachable
  check_saturn_auth_runtime
  configure_serve
  print_serve_status
  print_final_url
  ok "Done."
}

main "$@"
