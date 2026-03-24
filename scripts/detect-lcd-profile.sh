#!/usr/bin/env bash
set -euo pipefail

# Standalone LCD profile detector for Saturn images.
# Mirrors provisioning logic:
# env override -> boot config parse -> I2C probe -> default fallback.

SATURN_LCD_PROFILE="${SATURN_LCD_PROFILE:-auto}"
SATURN_LCD_SIZE_INCH="${SATURN_LCD_SIZE_INCH:-}"
SATURN_LCD_AUTO_DEFAULT_SIZE_INCH="${SATURN_LCD_AUTO_DEFAULT_SIZE_INCH:-}"
SATURN_LCD_I2C_DETECT_ADDR="${SATURN_LCD_I2C_DETECT_ADDR:-0x45}"

usage() {
  cat <<USAGE
Usage: $(basename "$0") [--json] [--emit-config]

Detect Saturn LCD profile using provisioning-compatible logic.

Environment:
  SATURN_LCD_PROFILE=none|cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8|auto
  SATURN_LCD_SIZE_INCH=7|8
  SATURN_LCD_AUTO_DEFAULT_SIZE_INCH=7|8
  SATURN_LCD_I2C_DETECT_ADDR=0x45

Output (default):
  size=<7|8|unknown>
  size_source=<env|config|i2c-probe|default|unknown>
  cm=<cm4|cm5|unknown>
  profile=<cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8|none|unknown>
  recommended_uart_overlay=<dtoverlay=...|none|unknown>
  recommended_panel_overlay=<dtoverlay=...|none|unknown>
USAGE
}

log_warn() {
  printf 'WARN: %s\n' "$*" >&2
}

get_boot_config_file() {
  local candidate
  for candidate in /boot/firmware/config.txt /boot/config.txt; do
    if [[ -f "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

detect_compute_module_generation() {
  local model
  model="$(tr -d '\0' < /proc/device-tree/model 2>/dev/null || true)"
  case "$model" in
    *"Compute Module 5"*) printf 'cm5\n' ;;
    *"Compute Module 4"*) printf 'cm4\n' ;;
    *) return 1 ;;
  esac
}

detect_lcd_size_from_config() {
  local boot_config="$1"

  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-800x480([[:space:]]*,.*)?$' "$boot_config"; then
    printf '7\n'
    return
  fi
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,.*' "$boot_config"; then
    printf '7\n'
    return
  fi
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,.*' "$boot_config"; then
    printf '8\n'
    return
  fi
}

detect_lcd_profile_from_config() {
  local boot_config="$1"
  local managed_profile=""

  managed_profile="$(awk '
    /^# BEGIN SATURN LCD PROFILE$/ { in_block=1; next }
    /^# END SATURN LCD PROFILE$/ { in_block=0 }
    in_block && /^# Saturn managed LCD profile: / {
      sub(/^# Saturn managed LCD profile: /, "")
      print
      exit
    }
  ' "$boot_config")"

  case "$managed_profile" in
    cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8)
      printf '%s\n' "$managed_profile"
      return
      ;;
  esac

  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1([[:space:]]*#.*)?$' "$boot_config" \
    && grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0([[:space:]]*#.*)?$' "$boot_config"; then
    printf 'cm5-7-g2-dual-dsi\n'
    return
  fi
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0([[:space:]]*#.*)?$' "$boot_config" \
    && grep -Eq '^[[:space:]]*dtoverlay=uart3([[:space:]]*#.*)?$' "$boot_config"; then
    printf 'cm4-7-g2-single-dsi\n'
    return
  fi
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0([[:space:]]*#.*)?$' "$boot_config" \
    && grep -Eq '^[[:space:]]*dtoverlay=uart2-pi5([[:space:]]*#.*)?$' "$boot_config"; then
    printf 'cm5-7-g2-single-dsi\n'
    return
  fi
}

lcd_profile_size() {
  case "$1" in
    cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi)
      printf '7\n'
      ;;
    cm4-8|cm5-8)
      printf '8\n'
      ;;
    none)
      printf 'none\n'
      ;;
    *)
      return 1
      ;;
  esac
}

i2c_address_detected() {
  local bus="$1"
  local addr="${2:-0x45}"
  local addr_dec addr_hex out

  if ! command -v i2cdetect >/dev/null 2>&1; then
    printf '0\n'
    return
  fi
  if [[ ! -e "/dev/i2c-${bus}" ]]; then
    printf '0\n'
    return
  fi

  if ! addr_dec=$((addr)); then
    printf '0\n'
    return
  fi
  if (( addr_dec < 0x08 || addr_dec > 0x77 )); then
    printf '0\n'
    return
  fi

  addr_hex="$(printf '%02x' "$addr_dec")"
  out="$(i2cdetect -y "$bus" "$addr_dec" "$addr_dec" 2>/dev/null || true)"
  if grep -Eq "(^|[[:space:]])(UU|${addr_hex})([[:space:]]|$)" <<<"$out"; then
    printf '1\n'
  else
    printf '0\n'
  fi
}

detect_lcd_size_from_i2c_probe() {
  local detect_addr="${SATURN_LCD_I2C_DETECT_ADDR:-0x45}"
  local bus0_has=0 bus1_has=0 bus10_has=0

  if [[ "$(i2c_address_detected 0 "$detect_addr")" == "1" ]]; then
    bus0_has=1
  fi
  if [[ "$(i2c_address_detected 1 "$detect_addr")" == "1" ]]; then
    bus1_has=1
  fi
  if [[ "$(i2c_address_detected 10 "$detect_addr")" == "1" ]]; then
    bus10_has=1
  fi

  if [[ "$bus1_has" -eq 1 && "$bus0_has" -eq 0 && "$bus10_has" -eq 0 ]]; then
    printf '8\n'
    return
  fi
  if [[ "$bus10_has" -eq 1 && "$bus1_has" -eq 0 ]]; then
    printf '7\n'
    return
  fi
  if [[ "$bus0_has" -eq 1 && "$bus1_has" -eq 0 && "$bus10_has" -eq 0 ]]; then
    printf '7\n'
    return
  fi
}

detect_lcd_size_auto() {
  local boot_config="$1"
  local size=""

  size="${SATURN_LCD_SIZE_INCH:-}"
  case "$size" in
    7|8)
      printf '%s|env\n' "$size"
      return 0
      ;;
    "") ;;
    *)
      log_warn "Invalid SATURN_LCD_SIZE_INCH='$size'; expected 7 or 8."
      ;;
  esac

  if [[ -n "$boot_config" ]]; then
    size="$(detect_lcd_size_from_config "$boot_config" 2>/dev/null || true)"
    case "$size" in
      7|8)
        printf '%s|config\n' "$size"
        return 0
        ;;
    esac
  fi

  size="$(detect_lcd_size_from_i2c_probe 2>/dev/null || true)"
  case "$size" in
    7|8)
      printf '%s|i2c-probe\n' "$size"
      return 0
      ;;
  esac

  size="${SATURN_LCD_AUTO_DEFAULT_SIZE_INCH:-}"
  case "$size" in
    7|8)
      printf '%s|default\n' "$size"
      return
      ;;
  esac

  return
}

resolve_lcd_profile() {
  local boot_config="$1"
  local requested cm size size_source auto_detect_result profile
  requested="${SATURN_LCD_PROFILE,,}"

  case "$requested" in
    none)
      printf 'none|none|none\n'
      return 0
      ;;
    cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8)
      size="$(lcd_profile_size "$requested")"
      printf '%s|%s|explicit\n' "$requested" "$size"
      return 0
      ;;
    auto|"")
      profile="$(detect_lcd_profile_from_config "$boot_config" 2>/dev/null || true)"
      if [[ -n "$profile" ]]; then
        size="$(lcd_profile_size "$profile" 2>/dev/null || true)"
        printf '%s|%s|config\n' "$profile" "${size:-unknown}"
        return 0
      fi
      cm="$(detect_compute_module_generation 2>/dev/null || true)"
      auto_detect_result="$(detect_lcd_size_auto "$boot_config" 2>/dev/null || true)"
      if [[ -n "$auto_detect_result" ]]; then
        IFS='|' read -r size size_source <<<"$auto_detect_result"
      else
        size=""
        size_source=""
      fi
      if [[ -z "$cm" || -z "$size" ]]; then
        printf 'unknown|unknown|unknown\n'
        return 1
      fi
      printf '%s-%s|%s|%s\n' "$cm" "$size" "$size" "$size_source"
      return 0
      ;;
    *)
      log_warn "Unknown SATURN_LCD_PROFILE='$SATURN_LCD_PROFILE'; expected none|cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8|auto"
      printf 'unknown|unknown|unknown\n'
      return 1
      ;;
  esac
}

recommended_overlays_for_profile() {
  local profile="$1"
  local uart panel

  case "$profile" in
    cm4-7)
      uart='dtoverlay=uart3'
      panel='dtoverlay=vc4-kms-dsi-waveshare-800x480'
      ;;
    cm4-7-custom-jd)
      uart='dtoverlay=uart3'
      panel='dtoverlay=vc4-kms-dsi-waveshare-800x480'
      ;;
    cm4-7-g2-single-dsi)
      uart='dtoverlay=uart3'
      panel='dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0'
      ;;
    cm4-8)
      uart='dtoverlay=uart3'
      panel='dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1'
      ;;
    cm5-7)
      uart='dtoverlay=uart2-pi5'
      panel='dtoverlay=vc4-kms-dsi-waveshare-800x480'
      ;;
    cm5-7-g2-single-dsi)
      uart='dtoverlay=uart2-pi5'
      panel='dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0'
      ;;
    cm5-7-g2-dual-dsi)
      uart='dtoverlay=uart2-pi5'
      panel='dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1; dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0'
      ;;
    cm5-8)
      uart='dtoverlay=uart2-pi5'
      panel='dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1'
      ;;
    none)
      uart='none'
      panel='none'
      ;;
    *)
      uart='unknown'
      panel='unknown'
      ;;
  esac

  printf '%s|%s\n' "$uart" "$panel"
}

main() {
  local json=0
  local emit_config=0
  local boot_config=""
  local profile size source cm
  local overlay_line uart_overlay panel_overlay

  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
  fi
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json)
        json=1
        ;;
      --emit-config)
        emit_config=1
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        usage >&2
        exit 2
        ;;
    esac
    shift
  done

  boot_config="$(get_boot_config_file 2>/dev/null || true)"

  if profile_line="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"; then
    IFS='|' read -r profile size source <<<"$profile_line"
  else
    IFS='|' read -r profile size source <<<"$profile_line"
  fi

  cm="$(detect_compute_module_generation 2>/dev/null || true)"
  cm="${cm:-unknown}"
  size="${size:-unknown}"
  source="${source:-unknown}"
  profile="${profile:-unknown}"
  overlay_line="$(recommended_overlays_for_profile "$profile")"
  IFS='|' read -r uart_overlay panel_overlay <<<"$overlay_line"

  if [[ "$emit_config" -eq 1 ]]; then
    printf '%s\n' "$uart_overlay"
    printf '%s\n' "$panel_overlay"
    exit 0
  fi

  if [[ "$json" -eq 1 ]]; then
    printf '{"size":"%s","size_source":"%s","cm":"%s","profile":"%s","recommended_uart_overlay":"%s","recommended_panel_overlay":"%s"}\n' \
      "$size" "$source" "$cm" "$profile" "$uart_overlay" "$panel_overlay"
  else
    printf 'size=%s\n' "$size"
    printf 'size_source=%s\n' "$source"
    printf 'cm=%s\n' "$cm"
    printf 'profile=%s\n' "$profile"
    printf 'recommended_uart_overlay=%s\n' "$uart_overlay"
    printf 'recommended_panel_overlay=%s\n' "$panel_overlay"
    if [[ -n "$boot_config" ]]; then
      printf 'boot_config=%s\n' "$boot_config"
    fi
  fi
}

main "$@"
