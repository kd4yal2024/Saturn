#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=saturn-lcd-lib.sh
source "${SCRIPT_DIR}/saturn-lcd-lib.sh"

DETECT_SCRIPT="${SCRIPT_DIR}/detect-lcd-profile.sh"
FRONT_PANEL_DETECT_SCRIPT="${SCRIPT_DIR}/detect-front-panel.sh"

# State file written by provisioning; prefer it over live re-probing.
SATURN_FRONT_PANEL_STATE_FILE="${SATURN_FRONT_PANEL_STATE_FILE:-/var/lib/saturn-provision/front-panel-type}"

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
  cm4-7-custom-jd
  cm4-7-g2-single-dsi
  cm4-8
  cm5-7
  cm5-7-g2-single-dsi
  cm5-7-g2-dual-dsi
  cm5-8
EOF
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

get_boot_config_path() {
  get_boot_config_file
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

# read_front_panel_type
# Prints TYPE|SOURCE where SOURCE is state-file|live-probe|none.
# Prefers the stable state file written by provisioning; falls back to live
# probing only when the file is absent.
read_front_panel_type() {
  local result=""

  if [[ -f "$SATURN_FRONT_PANEL_STATE_FILE" ]]; then
    result="$(tr -d '[:space:]' < "$SATURN_FRONT_PANEL_STATE_FILE" 2>/dev/null || true)"
    case "$result" in
      G2V1|G2V2|RemoteHead|NONE) printf '%s|state-file\n' "$result"; return ;;
    esac
  fi

  if [[ -x "$FRONT_PANEL_DETECT_SCRIPT" ]]; then
    result="$("$FRONT_PANEL_DETECT_SCRIPT" 2>/dev/null | tr -d '\r\n' || true)"
    case "$result" in
      G2V1|G2V2|RemoteHead|NONE) printf '%s|live-probe\n' "$result"; return ;;
    esac
  fi

  printf 'unknown|none\n'
}

# _setup_front_panel_type
# Reads the front-panel type and exports SATURN_FRONT_PANEL_TYPE so that
# resolve_lcd_profile (and detect-lcd-profile.sh subprocesses) can use it
# as a tiebreaker.  Also returns TYPE and SOURCE to the caller.
_setup_front_panel_type() {
  local fp_raw fp_type fp_source
  fp_raw="$(read_front_panel_type)"
  IFS='|' read -r fp_type fp_source <<<"$fp_raw"
  export SATURN_FRONT_PANEL_TYPE="$fp_type"
  printf '%s|%s\n' "$fp_type" "$fp_source"
}

profile_label() {
  case "$1" in
    auto)                printf 'Auto Detect\n' ;;
    cm4-7)               printf 'CM4 7-inch\n' ;;
    cm4-7-custom-jd)     printf 'CM4 7-inch Custom JD\n' ;;
    cm4-7-g2-single-dsi) printf 'CM4 7-inch G2 Single-DSI\n' ;;
    cm4-8)               printf 'CM4 8-inch\n' ;;
    cm5-7)               printf 'CM5 7-inch\n' ;;
    cm5-7-g2-single-dsi) printf 'CM5 7-inch G2 Single-DSI\n' ;;
    cm5-7-g2-dual-dsi)   printf 'CM5 7-inch G2 Dual-DSI\n' ;;
    cm5-8)               printf 'CM5 8-inch\n' ;;
    *)                   printf '%s\n' "$1" ;;
  esac
}

profile_description() {
  case "$1" in
    auto)                printf 'Use current detection logic and existing config hints.\n' ;;
    cm4-7)               printf 'CM4 with Waveshare 7-inch panel using the single-overlay 800x480 path.\n' ;;
    cm4-7-custom-jd)     printf 'CM4 custom JD 7-inch panel profile using the current known-good 800x480 overlay path.\n' ;;
    cm4-7-g2-single-dsi) printf 'CM4 with Laurence-style 7-inch single-DSI Waveshare panel (7_0_inchC on i2c0).\n' ;;
    cm4-8)               printf 'CM4 with Waveshare 8-inch panel using i2c1.\n' ;;
    cm5-7)               printf 'CM5 with Waveshare 7-inch panel using the single-overlay 800x480 path.\n' ;;
    cm5-7-g2-single-dsi) printf 'CM5 with Laurence-style 7-inch single-DSI Waveshare panel (7_0_inchC on i2c0).\n' ;;
    cm5-7-g2-dual-dsi)   printf 'CM5 Saturn G2 7-inch field profile using both DSI paths.\n' ;;
    cm5-8)               printf 'CM5 with Waveshare 8-inch panel using i2c1.\n' ;;
    *)                   printf 'Unknown profile.\n' ;;
  esac
}

print_profiles() {
  local profile
  for profile in auto cm4-7 cm4-7-custom-jd cm4-7-g2-single-dsi cm4-8 cm5-7 cm5-7-g2-single-dsi cm5-7-g2-dual-dsi cm5-8; do
    printf '%s|%s|%s\n' "$profile" "$(profile_label "$profile")" "$(profile_description "$profile")"
  done
}

print_detect() {
  local detect_output boot_config current_profile current_overlays fp_raw fp_type fp_source
  [[ -x "$DETECT_SCRIPT" ]] || die "Missing detect script: $DETECT_SCRIPT"

  # Read front-panel type first so it is exported into the detect subprocess,
  # allowing the tiebreaker in resolve_lcd_profile to fire.
  fp_raw="$(_setup_front_panel_type)"
  IFS='|' read -r fp_type fp_source <<<"$fp_raw"

  detect_output="$("$DETECT_SCRIPT" 2>/dev/null || true)"
  printf '%s\n' "$detect_output"

  printf 'front_panel_type=%s\n'   "$fp_type"
  printf 'front_panel_source=%s\n' "$fp_source"

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
  local boot_config resolved_raw

  _setup_front_panel_type >/dev/null

  if [[ "$profile" == "auto" ]]; then
    boot_config="$(get_boot_config_path 2>/dev/null || true)"
    SATURN_LCD_PROFILE=auto
    resolved_raw="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"
    [[ -n "$resolved_raw" ]] || die "Auto mode could not resolve a profile on this system."
    IFS='|' read -r profile _ _ <<<"$resolved_raw"
  fi

  printf 'profile=%s\n'      "$profile"
  printf 'label=%s\n'        "$(profile_label "$profile")"
  printf 'description=%s\n'  "$(profile_description "$profile")"
  printf 'preview_block<<EOF\n'
  render_lcd_profile_block "$profile"
  printf 'EOF\n'
}

apply_profile() {
  local requested_profile="$1"
  local boot_config backup_path resolved_profile resolved_raw

  [[ "$(id -u)" -eq 0 ]] || die "apply must be run as root (use pkexec or sudo)."

  boot_config="$(get_boot_config_path 2>/dev/null || true)"
  [[ -n "$boot_config" ]] || die "Could not locate config.txt"

  _setup_front_panel_type >/dev/null

  # Resolve early so we can report the actual profile that will be written.
  SATURN_LCD_PROFILE="$requested_profile"
  resolved_raw="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"
  [[ -n "$resolved_raw" ]] || die "Could not resolve a profile for '$requested_profile' on this system."
  IFS='|' read -r resolved_profile _ _ <<<"$resolved_raw"
  [[ -n "$resolved_profile" && "$resolved_profile" != "none" && "$resolved_profile" != "unknown" ]] \
    || die "Profile resolved to '$resolved_profile'; nothing to apply."

  # Pin to the concrete profile so configure_lcd_profile writes exactly what
  # we resolved above — no second auto-detection pass.
  SATURN_LCD_PROFILE="$resolved_profile"

  backup_path="${boot_config}.bak.lcd-tool.$(date +%Y%m%d%H%M%S)"
  cp -a "$boot_config" "$backup_path"
  configure_lcd_profile

  printf 'status=ok\n'
  printf 'applied_profile=%s\n' "$resolved_profile"
  printf 'boot_config=%s\n'     "$boot_config"
  printf 'backup_path=%s\n'     "$backup_path"
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
  printf 'boot_config=%s\n'   "$boot_config"
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
        apply)   apply_profile   "$profile" ;;
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
