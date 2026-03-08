#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PROVISION_SCRIPT="${REPO_ROOT}/provision/cloud-init/provision-saturn.sh"
DETECT_SCRIPT="${SCRIPT_DIR}/detect-lcd-profile.sh"

usage() {
  cat <<'EOF'
Usage:
  saturn-lcd-helper.sh detect
  saturn-lcd-helper.sh backups
  saturn-lcd-helper.sh preview --profile <profile>
  saturn-lcd-helper.sh apply --profile <profile>
  saturn-lcd-helper.sh restore-latest
  saturn-lcd-helper.sh restore --backup <path>
  saturn-lcd-helper.sh profiles

Profiles:
  auto
  cm4-7
  cm4-8
  cm5-7
  cm5-7-g2-dual-dsi
  cm5-8
EOF
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

source_provision_helpers() {
  [[ -f "$PROVISION_SCRIPT" ]] || die "Missing provision script: $PROVISION_SCRIPT"
  # Load helper functions without executing main().
  # shellcheck disable=SC1090
  source <(head -n -1 "$PROVISION_SCRIPT")
}

get_boot_config_path() {
  local candidate
  for candidate in /boot/firmware/config.txt /boot/config.txt; do
    [[ -f "$candidate" ]] && {
      printf '%s\n' "$candidate"
      return 0
    }
  done
  return 1
}

current_overlay_lines() {
  local boot_config="$1"
  grep -E '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare' "$boot_config" 2>/dev/null || true
}

latest_backup_path() {
  local boot_config="$1"
  find "$(dirname "$boot_config")" -maxdepth 1 -type f -name "$(basename "$boot_config").bak.lcd-tool.*" | sort | tail -n 1
}

print_backups() {
  local boot_config
  boot_config="$(get_boot_config_path 2>/dev/null || true)"
  [[ -n "$boot_config" ]] || die "Could not locate config.txt"

  find "$(dirname "$boot_config")" -maxdepth 1 -type f -name "$(basename "$boot_config").bak.lcd-tool.*" | sort
}

profile_label() {
  case "$1" in
    auto) printf 'Auto Detect\n' ;;
    cm4-7) printf 'CM4 7-inch\n' ;;
    cm4-8) printf 'CM4 8-inch\n' ;;
    cm5-7) printf 'CM5 7-inch\n' ;;
    cm5-7-g2-dual-dsi) printf 'CM5 7-inch G2 Dual-DSI\n' ;;
    cm5-8) printf 'CM5 8-inch\n' ;;
    *) printf '%s\n' "$1" ;;
  esac
}

profile_description() {
  case "$1" in
    auto) printf 'Use current detection logic and existing config hints.\n' ;;
    cm4-7) printf 'CM4 with Waveshare 7-inch panel using the single-overlay 800x480 path.\n' ;;
    cm4-8) printf 'CM4 with Waveshare 8-inch panel using i2c1.\n' ;;
    cm5-7) printf 'CM5 with Waveshare 7-inch panel using the single-overlay 800x480 path.\n' ;;
    cm5-7-g2-dual-dsi) printf 'CM5 Saturn G2 7-inch field profile using both DSI paths.\n' ;;
    cm5-8) printf 'CM5 with Waveshare 8-inch panel using i2c1.\n' ;;
    *) printf 'Unknown profile.\n' ;;
  esac
}

print_profiles() {
  local profile
  for profile in auto cm4-7 cm4-8 cm5-7 cm5-7-g2-dual-dsi cm5-8; do
    printf '%s|%s|%s\n' "$profile" "$(profile_label "$profile")" "$(profile_description "$profile")"
  done
}

print_detect() {
  local detect_output boot_config current_profile current_overlays
  [[ -x "$DETECT_SCRIPT" ]] || die "Missing detect script: $DETECT_SCRIPT"

  detect_output="$("$DETECT_SCRIPT" 2>/dev/null || true)"
  printf '%s\n' "$detect_output"

  boot_config="$(get_boot_config_path 2>/dev/null || true)"
  if [[ -n "$boot_config" ]]; then
    printf 'boot_config_path=%s\n' "$boot_config"
    current_profile="$(awk -F= '/^profile=/{print $2; exit}' <<<"$detect_output")"
    printf 'current_profile=%s\n' "${current_profile:-unknown}"
    printf 'current_overlay_lines<<EOF\n'
    current_overlays="$(current_overlay_lines "$boot_config")"
    if [[ -n "$current_overlays" ]]; then
      printf '%s\n' "$current_overlays"
    else
      printf '(none)\n'
    fi
    printf 'EOF\n'
  fi
}

preview_profile() {
  local profile="$1"
  source_provision_helpers
  if [[ "$profile" == "auto" ]]; then
    local boot_config resolved
    boot_config="$(get_boot_config_path 2>/dev/null || true)"
    SATURN_LCD_PROFILE=auto
    resolved="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"
    [[ -n "$resolved" ]] || die "Auto mode could not resolve a profile on this system."
    profile="$resolved"
  fi
  printf 'profile=%s\n' "$profile"
  printf 'label=%s\n' "$(profile_label "$profile")"
  printf 'description=%s\n' "$(profile_description "$profile")"
  printf 'preview_block<<EOF\n'
  render_lcd_profile_block "$profile"
  printf 'EOF\n'
}

apply_profile() {
  local profile="$1"
  local boot_config backup_path

  [[ "$(id -u)" -eq 0 ]] || die "apply must be run as root (use pkexec or sudo)."

  source_provision_helpers
  boot_config="$(get_boot_config_path 2>/dev/null || true)"
  [[ -n "$boot_config" ]] || die "Could not locate config.txt"

  if [[ "$profile" == "auto" ]]; then
    SATURN_LCD_PROFILE=auto
  else
    SATURN_LCD_PROFILE="$profile"
  fi

  backup_path="${boot_config}.bak.lcd-tool.$(date +%Y%m%d%H%M%S)"
  cp -a "$boot_config" "$backup_path"
  configure_lcd_profile

  printf 'status=ok\n'
  printf 'applied_profile=%s\n' "$SATURN_LCD_PROFILE"
  printf 'boot_config=%s\n' "$boot_config"
  printf 'backup_path=%s\n' "$backup_path"
}

restore_backup() {
  local backup_path="${1:-}"
  local boot_config latest_backup

  [[ "$(id -u)" -eq 0 ]] || die "restore must be run as root (use pkexec or sudo)."
  boot_config="$(get_boot_config_path 2>/dev/null || true)"
  [[ -n "$boot_config" ]] || die "Could not locate config.txt"

  if [[ -z "$backup_path" ]]; then
    latest_backup="$(latest_backup_path "$boot_config" 2>/dev/null || true)"
    [[ -n "$latest_backup" ]] || die "No LCD tool backups found for $(basename "$boot_config")"
    backup_path="$latest_backup"
  fi

  [[ -f "$backup_path" ]] || die "Backup not found: $backup_path"
  cp -a "$backup_path" "$boot_config"

  printf 'status=ok\n'
  printf 'restored_from=%s\n' "$backup_path"
  printf 'boot_config=%s\n' "$boot_config"
}

main() {
  local cmd="${1:-}"
  local profile=""
  shift || true

  case "$cmd" in
    detect)
      print_detect
      ;;
    backups)
      print_backups
      ;;
    profiles)
      print_profiles
      ;;
    preview|apply)
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --profile)
            profile="${2:-}"
            shift 2
            ;;
          *)
            usage >&2
            exit 2
            ;;
        esac
      done
      [[ -n "$profile" ]] || die "--profile is required"
      case "$cmd" in
        preview) preview_profile "$profile" ;;
        apply) apply_profile "$profile" ;;
      esac
      ;;
    restore-latest)
      restore_backup
      ;;
    restore)
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --backup)
            profile="${2:-}"
            shift 2
            ;;
          *)
            usage >&2
            exit 2
            ;;
        esac
      done
      [[ -n "$profile" ]] || die "--backup is required"
      restore_backup "$profile"
      ;;
    -h|--help|"")
      usage
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
