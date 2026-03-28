#!/usr/bin/env bash
set -euo pipefail

# Standalone LCD profile detector for Saturn images.
# Detection order: env override → boot config parse → I2C probe → default fallback.

# shellcheck source=saturn-lcd-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/saturn-lcd-lib.sh"

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
  SATURN_FRONT_PANEL_TYPE=G2V1|G2V2|NONE  (tiebreaker: CM4+G2V1/G2V2 -> g2-single-dsi; CM5+G2V1 -> g2-dual-dsi; CM5+G2V2 -> g2-single-dsi)

Output (default):
  size=<7|8|unknown>
  size_source=<env|config|i2c-probe|default|unknown>
  model=<device-tree model|unknown>
  platform_vendor=<raspberrypi|radxa|unknown>
  module_family=<cm4|cm5|unknown>
  storage_variant=<lite|emmc|unknown>
  cm=<cm4|cm5|unknown>
  profile=<cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8|none|unknown>
  recommended_uart_overlay=<dtoverlay=...|none|unknown>
  recommended_panel_overlay=<dtoverlay=...|none|unknown>
USAGE
}

main() {
  local json=0
  local emit_config=0
  local boot_config=""
  local profile_raw profile size source cm model platform_vendor module_family storage_variant
  local overlay_line uart_overlay panel_overlay

  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
  fi
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json)        json=1 ;;
      --emit-config) emit_config=1 ;;
      -h|--help)     usage; exit 0 ;;
      *) usage >&2; exit 2 ;;
    esac
    shift
  done

  boot_config="$(get_boot_config_file 2>/dev/null || true)"

  profile_raw="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"
  IFS='|' read -r profile size source <<<"${profile_raw:-}"

  model="$(read_device_tree_model 2>/dev/null || true)"
  platform_vendor="$(detect_platform_vendor 2>/dev/null || true)"
  module_family="$(detect_module_family 2>/dev/null || true)"
  storage_variant="$(detect_module_storage_variant 2>/dev/null || true)"
  cm="${module_family:-unknown}"
  model="${model:-unknown}"
  platform_vendor="${platform_vendor:-unknown}"
  module_family="${module_family:-unknown}"
  storage_variant="${storage_variant:-unknown}"
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
    printf '{"size":"%s","size_source":"%s","model":"%s","platform_vendor":"%s","module_family":"%s","storage_variant":"%s","cm":"%s","profile":"%s","recommended_uart_overlay":"%s","recommended_panel_overlay":"%s"}\n' \
      "$size" "$source" "$model" "$platform_vendor" "$module_family" "$storage_variant" "$cm" "$profile" "$uart_overlay" "$panel_overlay"
  else
    printf 'size=%s\n'                    "$size"
    printf 'size_source=%s\n'             "$source"
    printf 'model=%s\n'                   "$model"
    printf 'platform_vendor=%s\n'         "$platform_vendor"
    printf 'module_family=%s\n'           "$module_family"
    printf 'storage_variant=%s\n'         "$storage_variant"
    printf 'cm=%s\n'                      "$cm"
    printf 'profile=%s\n'                 "$profile"
    printf 'recommended_uart_overlay=%s\n'  "$uart_overlay"
    printf 'recommended_panel_overlay=%s\n' "$panel_overlay"
    if [[ -n "$boot_config" ]]; then
      printf 'boot_config=%s\n' "$boot_config"
    fi
  fi
}

main "$@"
