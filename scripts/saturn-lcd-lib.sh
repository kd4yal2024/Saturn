#!/usr/bin/env bash
# saturn-lcd-lib.sh — shared LCD and front-panel detection helpers.
#
# Source this file; do not execute it directly.
#
# Callers:
#   provision/cloud-init/provision-saturn.sh
#   scripts/detect-lcd-profile.sh
#   scripts/saturn-lcd-helper.sh
#
# Environment variables honoured:
#   SATURN_LCD_PROFILE           none|cm4-7|...|auto
#   SATURN_LCD_SIZE_INCH         7|8  (explicit override)
#   SATURN_LCD_AUTO_DEFAULT_SIZE_INCH  7|8  (fallback when ambiguous)
#   SATURN_LCD_I2C_DETECT_ADDR   I2C address to probe (default 0x45)
#   SATURN_LCD_DETECT_ONLY       1 → resolve but do not write config.txt

[[ -n "${_SATURN_LCD_LIB_LOADED:-}" ]] && return 0
_SATURN_LCD_LIB_LOADED=1

# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

# _saturn_lcd_log MSG
# Delegates to the caller's log() if defined (provisioning captures it to a
# file); otherwise writes to stderr.
_saturn_lcd_log() {
  if declare -f log >/dev/null 2>&1; then
    log "$@"
  else
    printf '%s\n' "$*" >&2
  fi
}

# _saturn_bool_true VAL
# Returns 0 for truthy values (1/true/TRUE/yes/YES/on/ON), 1 otherwise.
_saturn_bool_true() {
  case "${1:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

# ---------------------------------------------------------------------------
# Boot config location
# ---------------------------------------------------------------------------

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

# ---------------------------------------------------------------------------
# Hardware detection
# ---------------------------------------------------------------------------

detect_compute_module_generation() {
  detect_module_family
}

read_device_tree_model() {
  local model
  model="$(tr -d '\0' < /proc/device-tree/model 2>/dev/null || true)"
  [[ -n "$model" ]] || return 1
  printf '%s\n' "$model"
}

detect_platform_vendor() {
  local model lower
  model="$(read_device_tree_model 2>/dev/null || true)"
  lower="${model,,}"
  case "$lower" in
    *radxa*) printf 'radxa\n' ;;
    *"raspberry pi"*|*"compute module"*) printf 'raspberrypi\n' ;;
    *) return 1 ;;
  esac
}

detect_module_family() {
  local model lower
  model="$(read_device_tree_model 2>/dev/null || true)"
  lower="${model,,}"
  case "$model" in
    *"Compute Module 5"*) printf 'cm5\n' ;;
    *"Compute Module 4"*) printf 'cm4\n' ;;
    *)
      case "$lower" in
        *cm5*) printf 'cm5\n' ;;
        *cm4*) printf 'cm4\n' ;;
        *) return 1 ;;
      esac
      ;;
  esac
}

detect_module_storage_variant() {
  local model lower
  model="$(read_device_tree_model 2>/dev/null || true)"
  lower="${model,,}"
  case "$lower" in
    *" lite"*) printf 'lite\n' ;;
    *emmc*)    printf 'emmc\n' ;;
    *) return 1 ;;
  esac
}

# ---------------------------------------------------------------------------
# LCD size/profile detection from boot config
# ---------------------------------------------------------------------------

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
  if grep -Eq '^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,.*$' "$boot_config"; then
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

# lcd_profile_size PROFILE → 7|8|none
lcd_profile_size() {
  case "$1" in
    cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi)
      printf '7\n' ;;
    cm4-8|cm5-8)
      printf '8\n' ;;
    none)
      printf 'none\n' ;;
    *)
      return 1 ;;
  esac
}

# ---------------------------------------------------------------------------
# I2C probe
# ---------------------------------------------------------------------------

# i2c_address_detected BUS [ADDR]
# Prints 1 if the address responds on the bus, 0 otherwise.
# Uses read-probe mode (-r) to avoid disturbing sensitive devices.
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
  out="$(i2cdetect -r -y "$bus" "$addr_dec" "$addr_dec" 2>/dev/null || true)"
  if grep -Eq "(^|[[:space:]])(UU|${addr_hex})([[:space:]]|$)" <<<"$out"; then
    printf '1\n'
  else
    printf '0\n'
  fi
}

detect_lcd_size_from_i2c_probe() {
  local detect_addr="${SATURN_LCD_I2C_DETECT_ADDR:-0x45}"
  local bus0_has=0 bus1_has=0 bus10_has=0

  [[ "$(i2c_address_detected 0  "$detect_addr")" == "1" ]] && bus0_has=1
  [[ "$(i2c_address_detected 1  "$detect_addr")" == "1" ]] && bus1_has=1
  [[ "$(i2c_address_detected 10 "$detect_addr")" == "1" ]] && bus10_has=1

  # Bus 1 only → 8-inch (touchscreen on i2c1)
  if [[ "$bus1_has" -eq 1 && "$bus0_has" -eq 0 && "$bus10_has" -eq 0 ]]; then
    printf '8\n'
    return
  fi
  # Bus 10 present (CM5 DSI1 I2C) → 7-inch
  if [[ "$bus10_has" -eq 1 && "$bus1_has" -eq 0 ]]; then
    printf '7\n'
    return
  fi
  # Bus 0 only → 7-inch
  if [[ "$bus0_has" -eq 1 && "$bus1_has" -eq 0 && "$bus10_has" -eq 0 ]]; then
    printf '7\n'
    return
  fi
}

# detect_lcd_size_auto BOOT_CONFIG
# Prints SIZE|SOURCE where SOURCE is env|config|i2c-probe|default.
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
      _saturn_lcd_log "WARN: Invalid SATURN_LCD_SIZE_INCH='$size'; expected 7 or 8."
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
      return 0
      ;;
  esac

  return 0
}

# ---------------------------------------------------------------------------
# Profile resolution
# ---------------------------------------------------------------------------

# resolve_lcd_profile BOOT_CONFIG
# Prints: PROFILE|SIZE|SOURCE
#   PROFILE  cm4-7 | cm4-8 | ... | none | unknown
#   SIZE     7 | 8 | none | unknown
#   SOURCE   explicit | config | i2c-probe | env | default | none | unknown
# Returns 0 on success (including the none case), 1 if resolution fails.
resolve_lcd_profile() {
  local boot_config="$1"
  local requested cm size size_source auto_detect_result profile model platform_vendor

  requested="${SATURN_LCD_PROFILE,,}"

  case "$requested" in
    none|"")
      printf 'none|none|none\n'
      return 0
      ;;
    cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8)
      printf '%s|%s|explicit\n' "$requested" "$(lcd_profile_size "$requested")"
      return 0
      ;;
    auto)
      profile="$(detect_lcd_profile_from_config "$boot_config" 2>/dev/null || true)"
      if [[ -n "$profile" ]]; then
        _saturn_lcd_log "Auto-selected LCD profile from existing config: '$profile'"
        printf '%s|%s|config\n' "$profile" "$(lcd_profile_size "$profile" 2>/dev/null || printf 'unknown')"
        return 0
      fi

      model="$(read_device_tree_model 2>/dev/null || true)"
      platform_vendor="$(detect_platform_vendor 2>/dev/null || true)"
      cm="$(detect_compute_module_generation 2>/dev/null || true)"
      auto_detect_result="$(detect_lcd_size_auto "$boot_config" 2>/dev/null || true)"
      if [[ -n "$auto_detect_result" ]]; then
        IFS='|' read -r size size_source <<<"$auto_detect_result"
      else
        size=""
        size_source=""
      fi

      if [[ "$platform_vendor" != "raspberrypi" ]]; then
        _saturn_lcd_log "WARN: SATURN_LCD_PROFILE=auto currently supports Raspberry Pi CM4/CM5 overlays only (model='${model:-unknown}', vendor='${platform_vendor:-unknown}', module='${cm:-unknown}'). Set SATURN_LCD_PROFILE explicitly."
        return 1
      fi

      if [[ -z "$cm" || -z "$size" ]]; then
        _saturn_lcd_log "WARN: SATURN_LCD_PROFILE=auto could not resolve a unique profile (model='${model:-unknown}', vendor='${platform_vendor:-unknown}', cm='${cm:-unknown}', size='${size:-unknown}'). Set SATURN_LCD_PROFILE explicitly."
        return 1
      fi

      _saturn_lcd_log "Auto-selected LCD profile inputs: model='${model:-unknown}', vendor='${platform_vendor:-unknown}', cm='$cm', size='${size}' (source=${size_source:-unknown})"

      # Front-panel tiebreaker:
      #   - CM4 + G2V1/G2V2 -> single-DSI G2 profile
      #   - CM5 + G2V1      -> dual-DSI G2 field profile
      #   - CM5 + G2V2      -> single-DSI G2 profile
      # Only active when SATURN_FRONT_PANEL_TYPE is pre-populated by the caller
      # (helper reads the state file; provisioning sets it after detection).
      local fp_type="${SATURN_FRONT_PANEL_TYPE:-}"
      if [[ "$size" == "7" ]]; then
        case "$cm:$fp_type" in
          cm4:G2V1|cm4:G2V2)
            profile="${cm}-7-g2-single-dsi"
            _saturn_lcd_log "Front-panel tiebreaker: cm='$cm', type='$fp_type' → using profile '$profile'"
            printf '%s|%s|%s\n' "$profile" "$size" "$size_source"
            return 0
            ;;
          cm5:G2V1)
            profile="${cm}-7-g2-dual-dsi"
            _saturn_lcd_log "Front-panel tiebreaker: cm='$cm', type='$fp_type' → using profile '$profile'"
            printf '%s|%s|%s\n' "$profile" "$size" "$size_source"
            return 0
            ;;
          cm5:G2V2)
            profile="${cm}-7-g2-single-dsi"
            _saturn_lcd_log "Front-panel tiebreaker: cm='$cm', type='$fp_type' → using profile '$profile'"
            printf '%s|%s|%s\n' "$profile" "$size" "$size_source"
            return 0
            ;;
        esac
      fi

      printf '%s-%s|%s|%s\n' "$cm" "$size" "$size" "$size_source"
      return 0
      ;;
    *)
      _saturn_lcd_log "WARN: Unknown SATURN_LCD_PROFILE='$SATURN_LCD_PROFILE'; expected none|cm4-7|cm4-7-custom-jd|cm4-7-g2-single-dsi|cm4-8|cm5-7|cm5-7-g2-single-dsi|cm5-7-g2-dual-dsi|cm5-8|auto"
      return 1
      ;;
  esac
}

# ---------------------------------------------------------------------------
# Profile → overlay mapping
# ---------------------------------------------------------------------------

# recommended_overlays_for_profile PROFILE
# Prints: UART_OVERLAY|PANEL_OVERLAY  (for display/diagnostic use)
recommended_overlays_for_profile() {
  local profile="$1"
  local uart panel

  case "$profile" in
    cm4-7|cm4-7-custom-jd)
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

# render_lcd_profile_block PROFILE
# Renders the full managed config.txt block for the given profile.
render_lcd_profile_block() {
  local profile="$1"
  local uart_line panel_line

  case "$profile" in
    cm4-7|cm4-7-custom-jd)
      uart_line='dtoverlay=uart3'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-800x480'
      ;;
    cm4-7-g2-single-dsi)
      uart_line='dtoverlay=uart3'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0'
      ;;
    cm4-8)
      uart_line='dtoverlay=uart3'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1'
      ;;
    cm5-7)
      uart_line='dtoverlay=uart2-pi5'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-800x480'
      ;;
    cm5-7-g2-single-dsi)
      uart_line='dtoverlay=uart2-pi5'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0'
      ;;
    cm5-7-g2-dual-dsi)
      uart_line='dtoverlay=uart2-pi5'
      panel_line=$'dtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c0,dsi1\ndtoverlay=vc4-kms-dsi-waveshare-panel,7_0_inchC,i2c1,dsi0'
      ;;
    cm5-8)
      uart_line='dtoverlay=uart2-pi5'
      panel_line='dtoverlay=vc4-kms-dsi-waveshare-panel,8_0_inch,i2c1'
      ;;
    *)
      return 1
      ;;
  esac

  cat <<EOF
# Saturn managed LCD profile: $profile
dtparam=i2c_arm=on
dtparam=audio=on
auto_initramfs=1
dtoverlay=vc4-kms-v3d
max_framebuffers=2
disable_fw_kms_setup=1
arm_64bit=1
disable_overscan=1
arm_boost=1

[cm4]
otg_mode=1

[cm5]
dtoverlay=dwc2,dr_mode=host

[all]
dtparam=uart0=on
$uart_line
$panel_line
usb_max_current_enable=1
EOF
}

# ---------------------------------------------------------------------------
# Config write
# ---------------------------------------------------------------------------

# configure_lcd_profile
# Resolves SATURN_LCD_PROFILE and writes the managed block to config.txt.
# No-op when SATURN_LCD_DETECT_ONLY=1 or profile is none/unresolvable.
configure_lcd_profile() {
  local boot_config profile_raw profile block

  if ! boot_config="$(get_boot_config_file 2>/dev/null)"; then
    _saturn_lcd_log "WARN: Could not locate /boot/firmware/config.txt or /boot/config.txt for LCD profile setup."
    return 0
  fi

  profile_raw="$(resolve_lcd_profile "$boot_config" 2>/dev/null || true)"
  [[ -n "$profile_raw" ]] || return 0
  IFS='|' read -r profile _ _ <<<"$profile_raw"
  [[ -n "$profile" && "$profile" != "none" && "$profile" != "unknown" ]] || return 0

  if _saturn_bool_true "${SATURN_LCD_DETECT_ONLY:-0}"; then
    _saturn_lcd_log "SATURN_LCD_DETECT_ONLY=1 set; auto-detection resolved profile '$profile' and no config.txt changes were made."
    return 0
  fi

  if ! block="$(render_lcd_profile_block "$profile")"; then
    _saturn_lcd_log "WARN: Failed to render LCD block for profile '$profile'; skipping."
    return 0
  fi

  # Remove legacy/foreign panel overlays before applying managed block.
  sed -i -E '/^[[:space:]]*dtoverlay=vc4-kms-dsi-waveshare(-800x480|-panel,.*)[[:space:]]*$/d' "$boot_config"

  sed -i '/^# BEGIN SATURN LCD PROFILE$/,/^# END SATURN LCD PROFILE$/d' "$boot_config"
  {
    printf '\n# BEGIN SATURN LCD PROFILE\n'
    printf '# Managed by Saturn provisioning (non-destructive append)\n'
    printf '%s\n' "$block"
    printf '# END SATURN LCD PROFILE\n'
  } >>"$boot_config"

  _saturn_lcd_log "Applied SATURN_LCD_PROFILE='$profile' to $boot_config"
  _saturn_lcd_log "HDMI settings preserved (existing HDMI lines were not removed)."
}
