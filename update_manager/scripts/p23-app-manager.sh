#!/usr/bin/env bash
set -euo pipefail

# p23-app-manager.sh
# Hidden/test utility for building, deploying, and switching between P2_app/p2app
# and P3_app/p3app from the Saturn web UI terminal runner.

SCRIPT_VERSION="0.1.0"

ACTION=""
APP=""
VERBOSE=0
DRY_RUN=0
NO_RESTART=0
CLEAN_BUILD=1
SERVICE_MODE_PROFILE="${P23_SERVICE_MODE_PROFILE:-panel}"
PANEL_MODE="${P23_PANEL_MODE:-auto}"

progress(){ echo "Progress: $1%"; }
info(){ echo "$@"; }
warn(){ echo "WARN: $*" >&2; }
die(){ echo "ERR: $*" >&2; exit 1; }

usage(){
  cat <<'EOF'
Usage: p23-app-manager.sh [options]
Actions (choose one):
  --status               Show current build/deploy/service status
  --build p2|p3          Build selected app in repo tree
  --deploy p2|p3         Build, install, switch service to selected app
  --switch p2|p3         Switch service to already-deployed selected app
  --revert               Remove Saturn P2/P3 override and restore unit default ExecStart

Options:
  --mode <profile>       Service startup profile: panel|headless|panel-debug
  --panel <mode>         Front-panel detect mode: auto|g2|g2v2|prefer-g2|prefer-g2v2|off
  --no-restart           Update symlink/override without restarting p2app.service
  --no-clean             Skip 'make clean' before build
  --dry-run              Print commands without changing system
  --verbose              Echo commands before running
  -h, --help             Show help
EOF
}

run_cmd(){
  if (( DRY_RUN )); then
    info "[dry-run] $*"
    return 0
  fi
  if (( VERBOSE )); then
    info "+ $*"
  fi
  "$@"
}

run_root_cmd(){
  if (( EUID == 0 )); then
    run_cmd "$@"
    return
  fi
  if (( DRY_RUN )); then
    info "[dry-run] sudo -n $*"
    return 0
  fi
  if (( VERBOSE )); then
    info "+ sudo -n $*"
  fi
  sudo -n "$@"
}

need_cmd(){
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

resolve_app_name(){
  case "${1:-}" in
    p2|P2) echo "p2" ;;
    p3|P3) echo "p3" ;;
    *) return 1 ;;
  esac
}

resolve_service_mode_profile(){
  case "${1:-}" in
    panel|headless|panel-debug) echo "$1" ;;
    *) return 1 ;;
  esac
}

resolve_panel_mode(){
  case "${1:-}" in
    auto|g2|g2v2|prefer-g2|prefer_g2|prefer-g2v2|prefer_g2v2|off|none) echo "$1" ;;
    *) return 1 ;;
  esac
}

service_args_for_profile(){
  case "${1:-}" in
    panel) echo "-s -p" ;;
    headless) echo "-s" ;;
    panel-debug) echo "-s -p -d" ;;
    *) return 1 ;;
  esac
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --status)
      [[ -z "$ACTION" ]] || die "Only one action may be specified"
      ACTION="status"
      shift
      ;;
    --revert)
      [[ -z "$ACTION" ]] || die "Only one action may be specified"
      ACTION="revert"
      shift
      ;;
    --build|--deploy|--switch)
      [[ -z "$ACTION" ]] || die "Only one action may be specified"
      ACTION="${1#--}"
      shift
      [[ $# -gt 0 ]] || die "$ACTION requires p2 or p3"
      APP="$(resolve_app_name "$1")" || die "Invalid app '$1' (expected p2 or p3)"
      shift
      ;;
    --mode)
      shift
      [[ $# -gt 0 ]] || die "--mode requires a profile"
      SERVICE_MODE_PROFILE="$(resolve_service_mode_profile "$1")" || \
        die "Invalid mode '$1' (expected panel|headless|panel-debug)"
      shift
      ;;
    --panel)
      shift
      [[ $# -gt 0 ]] || die "--panel requires a mode"
      PANEL_MODE="$(resolve_panel_mode "$1")" || \
        die "Invalid panel mode '$1' (expected auto|g2|g2v2|prefer-g2|prefer-g2v2|off)"
      shift
      ;;
    --no-restart) NO_RESTART=1; shift ;;
    --no-clean) CLEAN_BUILD=0; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --verbose) VERBOSE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[[ -n "$ACTION" ]] || die "No action specified. Use --status, --build, --deploy, or --switch."

REPO_ROOT="${SATURN_ACTIVE_REPO_ROOT:-${SATURN_REPO_ROOT:-}}"
[[ -n "$REPO_ROOT" ]] || die "SATURN_ACTIVE_REPO_ROOT/SATURN_REPO_ROOT is not set"
[[ -d "$REPO_ROOT/.git" ]] || die "Repo root is not a git checkout: $REPO_ROOT"

P2_DIR="$REPO_ROOT/sw_projects/P2_app"
P3_DIR="$REPO_ROOT/sw_projects/P3_app"
P2_BIN="$P2_DIR/p2app"
P3_BIN="$P3_DIR/p3app"

P23_SERVICE_NAME="${P23_SERVICE_NAME:-p2app.service}"
P23_PANEL_ENV_NAME="${P23_PANEL_ENV_NAME:-SATURN_FRONT_PANEL_MODE}"
if [[ -n "${P23_SERVICE_ARGS+x}" ]] && [[ -n "${P23_SERVICE_ARGS}" ]]; then
  P23_SERVICE_ARGS="$P23_SERVICE_ARGS"
  SERVICE_MODE_PROFILE="custom"
else
  P23_SERVICE_ARGS="$(service_args_for_profile "$SERVICE_MODE_PROFILE")" || \
    die "Unsupported service mode profile: $SERVICE_MODE_PROFILE"
fi
P23_DEPLOY_ROOT="${P23_DEPLOY_ROOT:-/opt/saturn-go/p23-apps}"
P23_CURRENT_LINK="$P23_DEPLOY_ROOT/current"
P23_P2_DEPLOY_BIN="$P23_DEPLOY_ROOT/p2app"
P23_P3_DEPLOY_BIN="$P23_DEPLOY_ROOT/p3app"
P23_OVERRIDE_DIR="/etc/systemd/system/${P23_SERVICE_NAME}.d"
P23_OVERRIDE_FILE="$P23_OVERRIDE_DIR/10-saturn-p23-switch.conf"

need_cmd make

app_dir_for(){
  case "$1" in
    p2) echo "$P2_DIR" ;;
    p3) echo "$P3_DIR" ;;
    *) return 1 ;;
  esac
}

app_repo_bin_for(){
  case "$1" in
    p2) echo "$P2_BIN" ;;
    p3) echo "$P3_BIN" ;;
    *) return 1 ;;
  esac
}

app_deploy_bin_for(){
  case "$1" in
    p2) echo "$P23_P2_DEPLOY_BIN" ;;
    p3) echo "$P23_P3_DEPLOY_BIN" ;;
    *) return 1 ;;
  esac
}

show_status(){
  progress 5
  info "P2/P3 App Manager ${SCRIPT_VERSION}"
  info "Repo root: $REPO_ROOT"
  info "Service: $P23_SERVICE_NAME"
  info "Deploy root: $P23_DEPLOY_ROOT"
  info "Service args: $P23_SERVICE_ARGS"
  info "Startup profile: $SERVICE_MODE_PROFILE"
  info "Panel mode env: ${P23_PANEL_ENV_NAME}=${PANEL_MODE}"
  progress 20

  if [[ -d "$P2_DIR" ]]; then
    info "P2 dir: $P2_DIR"
  else
    warn "P2 dir missing: $P2_DIR"
  fi
  if [[ -d "$P3_DIR" ]]; then
    info "P3 dir: $P3_DIR"
  else
    warn "P3 dir missing: $P3_DIR"
  fi

  for f in "$P2_BIN" "$P3_BIN"; do
    if [[ -x "$f" ]]; then
      info "Built: $f ($(stat -c '%y' "$f" 2>/dev/null || echo unknown))"
    else
      info "Built: $f (missing)"
    fi
  done
  progress 45

  for f in "$P23_P2_DEPLOY_BIN" "$P23_P3_DEPLOY_BIN"; do
    if [[ -x "$f" ]]; then
      info "Deployed: $f ($(stat -c '%y' "$f" 2>/dev/null || echo unknown))"
    else
      info "Deployed: $f (missing)"
    fi
  done

  if [[ -L "$P23_CURRENT_LINK" ]]; then
    local_target="$(readlink -f "$P23_CURRENT_LINK" 2>/dev/null || true)"
    info "Current symlink: $P23_CURRENT_LINK -> ${local_target:-unknown}"
    case "$local_target" in
      "$P23_P2_DEPLOY_BIN") info "Active selection (symlink): p2" ;;
      "$P23_P3_DEPLOY_BIN") info "Active selection (symlink): p3" ;;
      *) warn "Current symlink target is unexpected" ;;
    esac
  elif [[ -e "$P23_CURRENT_LINK" ]]; then
    warn "Current path exists but is not a symlink: $P23_CURRENT_LINK"
  else
    info "Current symlink: missing ($P23_CURRENT_LINK)"
  fi
  progress 70

  if systemctl list-unit-files "$P23_SERVICE_NAME" >/dev/null 2>&1; then
    info "systemctl is-enabled $P23_SERVICE_NAME: $(systemctl is-enabled "$P23_SERVICE_NAME" 2>/dev/null || echo unknown)"
    info "systemctl is-active  $P23_SERVICE_NAME: $(systemctl is-active "$P23_SERVICE_NAME" 2>/dev/null || echo unknown)"
  else
    warn "Service not found: $P23_SERVICE_NAME"
  fi
  if [[ -f "$P23_OVERRIDE_FILE" ]]; then
    info "Override file: $P23_OVERRIDE_FILE"
    sed 's/^/  /' "$P23_OVERRIDE_FILE"
  else
    info "Override file: missing ($P23_OVERRIDE_FILE)"
  fi
  progress 100
  info "Done"
}

build_app(){
  local app="$1"
  local app_dir app_bin
  app_dir="$(app_dir_for "$app")"
  app_bin="$(app_repo_bin_for "$app")"
  [[ -d "$app_dir" ]] || die "App directory not found: $app_dir"

  progress 10
  info "Building $app in $app_dir"
  if (( CLEAN_BUILD )); then
    run_cmd make -C "$app_dir" clean
  else
    info "Skipping clean (--no-clean)"
  fi
  progress 40
  run_cmd make -C "$app_dir" -j1
  progress 80

  if (( ! DRY_RUN )) && [[ ! -x "$app_bin" ]]; then
    die "Build finished but binary not found: $app_bin"
  fi
  info "Built binary: $app_bin"
  progress 100
  info "Done"
}

write_override_and_switch(){
  local app="$1"
  local deploy_bin
  deploy_bin="$(app_deploy_bin_for "$app")"
  [[ -n "$deploy_bin" ]] || die "Unknown app '$app'"
  if (( ! DRY_RUN )) && [[ ! -x "$deploy_bin" ]]; then
    die "Deployed binary missing: $deploy_bin (run --deploy $app first)"
  fi

  progress 15
  info "Switching $P23_SERVICE_NAME to $app"
  run_root_cmd mkdir -p "$P23_DEPLOY_ROOT" "$P23_OVERRIDE_DIR"
  run_root_cmd ln -sfn "$deploy_bin" "$P23_CURRENT_LINK"

  if (( DRY_RUN )); then
    info "[dry-run] install override file -> $P23_OVERRIDE_FILE"
    info "[dry-run] override content:"
    info "[dry-run]   [Service]"
    info "[dry-run]   # saturn-p23 mode=${SERVICE_MODE_PROFILE} panel=${PANEL_MODE}"
    info "[dry-run]   Environment=${P23_PANEL_ENV_NAME}=${PANEL_MODE}"
    info "[dry-run]   ExecStart="
    info "[dry-run]   ExecStart=${P23_CURRENT_LINK} ${P23_SERVICE_ARGS}"
  else
    local tmp_override
    tmp_override="$(mktemp)"
    cat > "$tmp_override" <<EOF
[Service]
# saturn-p23 mode=${SERVICE_MODE_PROFILE} panel=${PANEL_MODE}
Environment=${P23_PANEL_ENV_NAME}=${PANEL_MODE}
ExecStart=
ExecStart=${P23_CURRENT_LINK} ${P23_SERVICE_ARGS}
EOF
    run_root_cmd install -m 0644 "$tmp_override" "$P23_OVERRIDE_FILE"
    rm -f "$tmp_override"
  fi

  progress 55
  run_root_cmd systemctl daemon-reload
  if (( NO_RESTART )); then
    info "Skipping restart (--no-restart)"
  else
    run_root_cmd systemctl restart "$P23_SERVICE_NAME"
    progress 90
    run_root_cmd systemctl --no-pager --full --lines=0 status "$P23_SERVICE_NAME" >/dev/null
    info "Service restarted: $P23_SERVICE_NAME"
  fi

  progress 100
  info "Done"
}

deploy_app(){
  local app="$1"
  local app_dir repo_bin deploy_bin
  app_dir="$(app_dir_for "$app")"
  repo_bin="$(app_repo_bin_for "$app")"
  deploy_bin="$(app_deploy_bin_for "$app")"

  [[ -d "$app_dir" ]] || die "App directory not found: $app_dir"

  progress 5
  info "Deploy action: build + install + switch ($app)"
  if (( CLEAN_BUILD )); then
    run_cmd make -C "$app_dir" clean
  else
    info "Skipping clean (--no-clean)"
  fi
  progress 25
  run_cmd make -C "$app_dir" -j1
  progress 50

  if (( ! DRY_RUN )) && [[ ! -x "$repo_bin" ]]; then
    die "Built binary not found: $repo_bin"
  fi

  run_root_cmd mkdir -p "$P23_DEPLOY_ROOT" "$P23_OVERRIDE_DIR"
  run_root_cmd install -m 0755 "$repo_bin" "$deploy_bin"
  info "Installed: $deploy_bin"
  progress 70

  write_override_and_switch "$app"
}

revert_to_unit_default(){
  progress 10
  info "Reverting $P23_SERVICE_NAME to unit default ExecStart (remove Saturn override)"

  if [[ -f "$P23_OVERRIDE_FILE" ]]; then
    run_root_cmd rm -f "$P23_OVERRIDE_FILE"
    info "Removed override: $P23_OVERRIDE_FILE"
  else
    info "Override already absent: $P23_OVERRIDE_FILE"
  fi

  # Best effort cleanup of now-empty override dir.
  if [[ -d "$P23_OVERRIDE_DIR" ]]; then
    if (( DRY_RUN )); then
      info "[dry-run] rmdir $P23_OVERRIDE_DIR (if empty)"
    else
      rmdir "$P23_OVERRIDE_DIR" >/dev/null 2>&1 || true
    fi
  fi

  progress 55
  run_root_cmd systemctl daemon-reload
  if (( NO_RESTART )); then
    info "Skipping restart (--no-restart)"
  else
    run_root_cmd systemctl restart "$P23_SERVICE_NAME"
    progress 90
    run_root_cmd systemctl --no-pager --full --lines=0 status "$P23_SERVICE_NAME" >/dev/null
    info "Service restarted: $P23_SERVICE_NAME"
  fi

  progress 100
  info "Done"
}

case "$ACTION" in
  status) show_status ;;
  build) build_app "$APP" ;;
  switch) write_override_and_switch "$APP" ;;
  deploy) deploy_app "$APP" ;;
  revert) revert_to_unit_default ;;
  *) die "Unhandled action: $ACTION" ;;
esac
