#!/usr/bin/env bash
# saturn-tailscale.sh
# Multi-action Tailscale management helper for the Saturn web UI.
#
# Subcommands:
#   install                              Install the Tailscale package via the official installer.
#   up [--auth-key=KEY]                  Bring tailscaled up. Without --auth-key, prints the
#      [--hostname=NAME]                 login URL on stderr (browser auth flow).
#      [--ssh] [--accept-routes]
#      [--accept-dns] [--reset]
#   down                                 tailscale down (drop from tailnet, keep auth).
#   logout                               tailscale logout (forget node key, requires re-auth).
#   serve-on  [--port=N]                 Run saturn-go-tailscale-serve.sh (gates + configure).
#   serve-off                            tailscale serve reset.
#
# This script is invoked via NOPASSWD sudo from rust-server. Every accepted
# flag is validated against a strict allow-list; arbitrary tailscale flags
# are NOT forwarded.

set -euo pipefail

PRIVILEGED_DIR="${SATURN_PRIVILEGED_DIR:-/usr/local/lib/saturn-go/scripts}"
SERVE_HELPER="${SATURN_SERVE_HELPER:-${PRIVILEGED_DIR}/saturn-go-tailscale-serve.sh}"
TAILSCALE_INSTALL_URL="${TAILSCALE_INSTALL_URL:-https://tailscale.com/install.sh}"

# Allow-list patterns. Reject anything that does not match exactly.
HOSTNAME_RE='^[A-Za-z0-9][A-Za-z0-9-]{0,62}$'
AUTHKEY_RE='^tskey-[A-Za-z0-9_-]{8,256}$'
PORT_RE='^[0-9]{1,5}$'

bold() { printf "\e[1m%s\e[0m\n" "$*"; }
info() { printf "[INFO] %s\n" "$*"; }
warn() { printf "[WARN] %s\n" "$*" >&2; }
err()  { printf "[ERR] %s\n" "$*" >&2; }
ok()   { printf "[OK] %s\n" "$*"; }

usage() {
  cat <<'EOF'
Usage:
  saturn-tailscale.sh install
  saturn-tailscale.sh up [--auth-key=tskey-...] [--hostname=NAME]
                         [--ssh] [--accept-routes] [--accept-dns] [--reset]
  saturn-tailscale.sh down
  saturn-tailscale.sh logout
  saturn-tailscale.sh serve-on [--port=N]
  saturn-tailscale.sh serve-off
EOF
}

require_root() {
  if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
    err "Run as root (sudo)."
    exit 1
  fi
}

require_tailscale() {
  if ! command -v tailscale >/dev/null 2>&1; then
    err "tailscale CLI not found. Run 'install' first."
    exit 1
  fi
}

cmd_install() {
  require_root
  if command -v tailscale >/dev/null 2>&1; then
    ok "Tailscale CLI already installed: $(command -v tailscale)"
    tailscale version 2>/dev/null | head -n 1 || true
    return 0
  fi
  if ! command -v curl >/dev/null 2>&1; then
    err "curl is required to fetch the Tailscale installer."
    exit 1
  fi
  info "Fetching Tailscale installer from ${TAILSCALE_INSTALL_URL}"
  curl -fsSL "${TAILSCALE_INSTALL_URL}" | sh
  if ! command -v tailscale >/dev/null 2>&1; then
    err "Installer ran but tailscale CLI is not on PATH."
    exit 1
  fi
  ok "Tailscale installed: $(command -v tailscale)"
  systemctl enable --now tailscaled.service
  ok "tailscaled.service enabled and started."
}

cmd_up() {
  require_root
  require_tailscale

  local auth_key="" hostname="" ssh=0 accept_routes=0 accept_dns=0 reset=0
  for arg in "$@"; do
    case "$arg" in
      --auth-key=*)
        auth_key="${arg#--auth-key=}"
        if [[ ! "$auth_key" =~ $AUTHKEY_RE ]]; then
          err "Invalid --auth-key format. Expected tskey-... with allowed characters."
          exit 2
        fi
        ;;
      --hostname=*)
        hostname="${arg#--hostname=}"
        if [[ ! "$hostname" =~ $HOSTNAME_RE ]]; then
          err "Invalid --hostname. Use letters, digits, and hyphens (max 63 chars)."
          exit 2
        fi
        ;;
      --ssh)            ssh=1 ;;
      --accept-routes)  accept_routes=1 ;;
      --accept-dns)     accept_dns=1 ;;
      --reset)          reset=1 ;;
      *)
        err "Unknown argument to 'up': $arg"
        usage >&2
        exit 2
        ;;
    esac
  done

  # Make sure tailscaled is running before we try to bring it up.
  if ! systemctl is-active --quiet tailscaled.service; then
    info "tailscaled.service is not active; starting it."
    systemctl enable --now tailscaled.service
  fi

  local args=(up)
  [[ -n "$auth_key" ]] && args+=("--auth-key=${auth_key}")
  [[ -n "$hostname" ]] && args+=("--hostname=${hostname}")
  (( ssh ))           && args+=(--ssh)
  (( accept_routes )) && args+=(--accept-routes)
  (( accept_dns ))    && args+=(--accept-dns)
  (( reset ))         && args+=(--reset)
  # Always run non-interactively so the browser-auth URL is printed and the
  # process returns instead of blocking on stdin/tty.
  args+=(--timeout=60s)

  info "Running: tailscale ${args[*]}"
  if (( ${#auth_key} )); then
    # Mask key in output already shown; tailscale does not echo it.
    :
  fi
  tailscale "${args[@]}"
  ok "tailscale up completed."
}

cmd_down() {
  require_root
  require_tailscale
  info "Running: tailscale down"
  tailscale down
  ok "tailscale down completed."
}

cmd_logout() {
  require_root
  require_tailscale
  info "Running: tailscale logout"
  tailscale logout
  ok "tailscale logout completed."
}

cmd_serve_on() {
  require_root
  require_tailscale

  local port=""
  for arg in "$@"; do
    case "$arg" in
      --port=*)
        port="${arg#--port=}"
        if [[ ! "$port" =~ $PORT_RE ]] || (( port < 1 || port > 65535 )); then
          err "Invalid --port. Expected 1..65535."
          exit 2
        fi
        ;;
      *)
        err "Unknown argument to 'serve-on': $arg"
        usage >&2
        exit 2
        ;;
    esac
  done

  if [[ ! -x "$SERVE_HELPER" ]]; then
    err "Serve helper not found or not executable: $SERVE_HELPER"
    exit 1
  fi

  if [[ -n "$port" ]]; then
    SERVE_HTTPS_PORT="$port" "$SERVE_HELPER"
  else
    "$SERVE_HELPER"
  fi
}

cmd_serve_off() {
  require_root
  require_tailscale
  info "Running: tailscale serve reset"
  tailscale serve reset
  ok "tailscale serve reset completed."
}

main() {
  local sub="${1:-}"
  if [[ -z "$sub" ]]; then
    usage >&2
    exit 2
  fi
  shift || true
  case "$sub" in
    install)    cmd_install "$@" ;;
    up)         cmd_up "$@" ;;
    down)       cmd_down "$@" ;;
    logout)     cmd_logout "$@" ;;
    serve-on)   cmd_serve_on "$@" ;;
    serve-off)  cmd_serve_off "$@" ;;
    -h|--help)  usage ;;
    *)
      err "Unknown subcommand: $sub"
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
