#!/usr/bin/env bash
set -euo pipefail

# p23-app-manager.sh
# Hidden/test utility for the converged hardened p2app implementation.
# The historical P2/P3 split has been retired; this script now manages a
# single supported app path while keeping the same web UI/override endpoints.

SCRIPT_VERSION="0.3.0"

ACTION=""
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
  --build [p2|p3]        Build the converged app in sw_projects/P2_app
  --deploy [p2|p3]       Build, install, and optionally restart the converged app
  --restart [p2|p3]      Reapply override metadata/current symlink and restart service
  --switch [p2|p3]       Deprecated alias for --restart
  --revert               Remove Saturn override and restore unit default ExecStart

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

consume_legacy_app_arg(){
  if [[ $# -gt 0 ]]; then
    case "${1:-}" in
      p2|P2)
        info "Using converged p2app implementation"
        return 0
        ;;
      p3|P3)
        warn "Legacy P3 selection requested; using the converged p2app implementation instead."
        return 0
        ;;
    esac
  fi
  return 1
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
    --build|--deploy|--restart|--switch)
      [[ -z "$ACTION" ]] || die "Only one action may be specified"
      case "$1" in
        --switch) ACTION="restart" ;;
        *) ACTION="${1#--}" ;;
      esac
      shift
      if consume_legacy_app_arg "${1:-}"; then
        shift
      fi
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

[[ -n "$ACTION" ]] || die "No action specified. Use --status, --build, --deploy, --restart, or --revert."

REPO_ROOT="${SATURN_ACTIVE_REPO_ROOT:-${SATURN_REPO_ROOT:-}}"
[[ -n "$REPO_ROOT" ]] || die "SATURN_ACTIVE_REPO_ROOT/SATURN_REPO_ROOT is not set"
[[ -d "$REPO_ROOT/.git" ]] || die "Repo root is not a git checkout: $REPO_ROOT"

P2_DIR="$REPO_ROOT/sw_projects/P2_app"
P2_BIN="$P2_DIR/p2app"

P23_SERVICE_NAME="${P23_SERVICE_NAME:-p2app.service}"
P23_XDMA_READY_SERVICE="${P23_XDMA_READY_SERVICE:-saturn-xdma-ready.service}"
P23_XDMA_DOCTOR="${P23_XDMA_DOCTOR:-/usr/local/bin/saturn-xdma-doctor.sh}"
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
P23_DEPLOY_BIN="$P23_DEPLOY_ROOT/p2app"
P23_OVERRIDE_DIR="/etc/systemd/system/${P23_SERVICE_NAME}.d"
P23_OVERRIDE_FILE="$P23_OVERRIDE_DIR/10-saturn-p23-switch.conf"

need_cmd make

show_xdma_doctor_stage(){
  if [[ ! -x "$P23_XDMA_DOCTOR" ]]; then
    warn "XDMA doctor not found: $P23_XDMA_DOCTOR"
    return 0
  fi
  local stage
  stage="$("$P23_XDMA_DOCTOR" --stage-only --skip-service-check 2>/dev/null || true)"
  info "XDMA doctor stage: ${stage:-unknown}"
}

emit_xdma_doctor_report(){
  if [[ ! -x "$P23_XDMA_DOCTOR" ]]; then
    warn "XDMA doctor not found: $P23_XDMA_DOCTOR"
    return 0
  fi
  info "XDMA doctor report:"
  "$P23_XDMA_DOCTOR" --skip-service-check 2>&1 | sed 's/^/  /'
}

restart_service_and_report(){
  if (( NO_RESTART )); then
    info "Skipping restart (--no-restart)"
    return 0
  fi

  if ! run_root_cmd systemctl restart "$P23_SERVICE_NAME"; then
    warn "systemctl restart failed for $P23_SERVICE_NAME"
    emit_xdma_doctor_report
    die "Failed to restart $P23_SERVICE_NAME"
  fi

  progress 90
  if ! run_root_cmd systemctl --no-pager --full --lines=0 status "$P23_SERVICE_NAME" >/dev/null; then
    warn "Restart verification failed for $P23_SERVICE_NAME"
    emit_xdma_doctor_report
    die "$P23_SERVICE_NAME did not restart cleanly"
  fi

  info "Service restarted: $P23_SERVICE_NAME"
  show_xdma_doctor_stage
}

render_override_to(){
  local target_file="$1"
  cat > "$target_file" <<EOF
[Service]
# saturn-p23 mode=${SERVICE_MODE_PROFILE} panel=${PANEL_MODE}
Environment=${P23_PANEL_ENV_NAME}=${PANEL_MODE}
ExecStart=
ExecStart=${P23_CURRENT_LINK} ${P23_SERVICE_ARGS}
EOF
}

write_override(){
  run_root_cmd mkdir -p "$P23_DEPLOY_ROOT" "$P23_OVERRIDE_DIR"
  run_root_cmd ln -sfn "$P23_DEPLOY_BIN" "$P23_CURRENT_LINK"

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
    render_override_to "$tmp_override"
    run_root_cmd install -m 0644 "$tmp_override" "$P23_OVERRIDE_FILE"
    rm -f "$tmp_override"
  fi

  run_root_cmd systemctl daemon-reload
}

show_status(){
  progress 5
  info "p2app Service Manager ${SCRIPT_VERSION}"
  info "Repo root: $REPO_ROOT"
  info "Service: $P23_SERVICE_NAME"
  info "Deploy root: $P23_DEPLOY_ROOT"
  info "Service args: $P23_SERVICE_ARGS"
  info "Startup profile: $SERVICE_MODE_PROFILE"
  info "Panel mode env: ${P23_PANEL_ENV_NAME}=${PANEL_MODE}"
  progress 20

  if [[ -d "$P2_DIR" ]]; then
    info "Source dir: $P2_DIR"
  else
    warn "Source dir missing: $P2_DIR"
  fi

  if [[ -x "$P2_BIN" ]]; then
    info "Built: $P2_BIN ($(stat -c '%y' "$P2_BIN" 2>/dev/null || echo unknown))"
  else
    info "Built: $P2_BIN (missing)"
  fi
  progress 45

  if [[ -x "$P23_DEPLOY_BIN" ]]; then
    info "Deployed: $P23_DEPLOY_BIN ($(stat -c '%y' "$P23_DEPLOY_BIN" 2>/dev/null || echo unknown))"
  else
    info "Deployed: $P23_DEPLOY_BIN (missing)"
  fi

  if [[ -L "$P23_CURRENT_LINK" ]]; then
    local local_target
    local_target="$(readlink -f "$P23_CURRENT_LINK" 2>/dev/null || true)"
    info "Current symlink: $P23_CURRENT_LINK -> ${local_target:-unknown}"
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
  if systemctl list-unit-files "$P23_XDMA_READY_SERVICE" >/dev/null 2>&1; then
    local ready_active ready_result
    ready_active="$(systemctl show -p ActiveState --value "$P23_XDMA_READY_SERVICE" 2>/dev/null || true)"
    ready_result="$(systemctl show -p Result --value "$P23_XDMA_READY_SERVICE" 2>/dev/null || true)"
    info "XDMA readiness gate: active_state=${ready_active:-unknown} result=${ready_result:-unknown}"
  else
    warn "XDMA readiness service not found: $P23_XDMA_READY_SERVICE"
  fi
  show_xdma_doctor_stage
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
  [[ -d "$P2_DIR" ]] || die "App directory not found: $P2_DIR"

  progress 10
  info "Building converged app in $P2_DIR"
  if (( CLEAN_BUILD )); then
    run_cmd make -C "$P2_DIR" clean
  else
    info "Skipping clean (--no-clean)"
  fi
  progress 40
  run_cmd make -C "$P2_DIR" -j1
  progress 80

  if (( ! DRY_RUN )) && [[ ! -x "$P2_BIN" ]]; then
    die "Build finished but binary not found: $P2_BIN"
  fi
  info "Built binary: $P2_BIN"
  progress 100
  info "Done"
}

deploy_app(){
  [[ -d "$P2_DIR" ]] || die "App directory not found: $P2_DIR"

  progress 5
  info "Deploy action: build + install + refresh override"
  if (( CLEAN_BUILD )); then
    run_cmd make -C "$P2_DIR" clean
  else
    info "Skipping clean (--no-clean)"
  fi
  progress 25
  run_cmd make -C "$P2_DIR" -j1
  progress 50

  if (( ! DRY_RUN )) && [[ ! -x "$P2_BIN" ]]; then
    die "Built binary not found: $P2_BIN"
  fi

  run_root_cmd mkdir -p "$P23_DEPLOY_ROOT" "$P23_OVERRIDE_DIR"
  run_root_cmd install -m 0755 "$P2_BIN" "$P23_DEPLOY_BIN"
  info "Installed: $P23_DEPLOY_BIN"
  progress 70

  write_override
  restart_service_and_report

  progress 100
  info "Done"
}

restart_with_current_override(){
  progress 15
  info "Refreshing override/current symlink for converged p2app"
  write_override
  progress 55
  restart_service_and_report
  progress 100
  info "Done"
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

  if [[ -d "$P23_OVERRIDE_DIR" ]]; then
    if (( DRY_RUN )); then
      info "[dry-run] rmdir $P23_OVERRIDE_DIR (if empty)"
    else
      rmdir "$P23_OVERRIDE_DIR" >/dev/null 2>&1 || true
    fi
  fi

  progress 55
  run_root_cmd systemctl daemon-reload
  restart_service_and_report

  progress 100
  info "Done"
}

case "$ACTION" in
  status) show_status ;;
  build) build_app ;;
  deploy) deploy_app ;;
  restart) restart_with_current_override ;;
  revert) revert_to_unit_default ;;
  *) die "Unhandled action: $ACTION" ;;
esac
