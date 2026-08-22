#!/usr/bin/env bash
set -euo pipefail

# shutdown-waiter.sh
# Wait for a shutdown signal on GPIO26, but avoid false triggers on
# hardware variants where this line is not a dedicated shutdown input.
#
# CM5 device trees can expose the front-panel switch as a native gpio-keys
# KEY_POWER device named "pwr_button". In that case systemd-logind owns the
# shutdown path and this fallback must not compete for the same GPIO line.
#
# Optional config file:
#   /etc/default/saturn-shutdown-waiter
# Supported keys:
#   SATURN_SHUTDOWN_WAITER_ENABLED=auto|true|false
#   SATURN_SHUTDOWN_WAITER_GPIO_CHIP=gpiochip0
#   SATURN_SHUTDOWN_WAITER_GPIO_LINE=26
#   SATURN_SHUTDOWN_WAITER_ARM_DELAY_SEC=20
#   SATURN_SHUTDOWN_WAITER_POLL_SEC=1
#   SATURN_SHUTDOWN_WAITER_LOW_CONFIRM_COUNT=3
#   SATURN_SHUTDOWN_WAITER_REQUIRE_HIGH_BEFORE_ARM=1
#   SATURN_SHUTDOWN_WAITER_I2C_BUS=1
#   SATURN_SHUTDOWN_WAITER_I2C_ADDR=0x20

CONFIG_FILE="${SATURN_SHUTDOWN_WAITER_CONFIG:-/etc/default/saturn-shutdown-waiter}"
if [[ -r "$CONFIG_FILE" ]]; then
  # shellcheck disable=SC1090
  . "$CONFIG_FILE"
fi

ENABLED="${SATURN_SHUTDOWN_WAITER_ENABLED:-auto}"
GPIO_CHIP="${SATURN_SHUTDOWN_WAITER_GPIO_CHIP:-gpiochip0}"
GPIO_LINE="${SATURN_SHUTDOWN_WAITER_GPIO_LINE:-26}"
ARM_DELAY_SEC="${SATURN_SHUTDOWN_WAITER_ARM_DELAY_SEC:-20}"
POLL_SEC="${SATURN_SHUTDOWN_WAITER_POLL_SEC:-1}"
LOW_CONFIRM_COUNT="${SATURN_SHUTDOWN_WAITER_LOW_CONFIRM_COUNT:-3}"
REQUIRE_HIGH_BEFORE_ARM="${SATURN_SHUTDOWN_WAITER_REQUIRE_HIGH_BEFORE_ARM:-1}"
I2C_BUS="${SATURN_SHUTDOWN_WAITER_I2C_BUS:-1}"
I2C_ADDR="${SATURN_SHUTDOWN_WAITER_I2C_ADDR:-0x20}"
INPUT_DEVICES_PATH="${SATURN_POWER_INPUT_DEVICES_PATH:-/proc/bus/input/devices}"
DEVICE_TREE_ROOT="${SATURN_POWER_DEVICE_TREE_ROOT:-/sys/firmware/devicetree/base}"
BOOT_CONFIG_PATH="${SATURN_POWER_BOOT_CONFIG_PATH:-/boot/firmware/config.txt}"
SYSTEMD_INHIBIT_BIN="${SATURN_POWER_SYSTEMD_INHIBIT:-systemd-inhibit}"

log() {
  printf '[saturn-shutdown-waiter] %s\n' "$1"
}

native_power_button_present() {
  if [[ -r "$INPUT_DEVICES_PATH" ]]; then
    awk '
      BEGIN { RS=""; found=0 }
      /N: Name="pwr_button"/ && (/P: Phys=gpio-keys\/input/ || /S: Sysfs=.*\/platform\/pwr_button\/input\//) { found=1 }
      END { exit(found ? 0 : 1) }
    ' "$INPUT_DEVICES_PATH"
    return $?
  fi

  [[ -d "$DEVICE_TREE_ROOT/pwr_button" ]]
}

active_gpio_shutdown_overlay() {
  [[ -r "$BOOT_CONFIG_PATH" ]] || return 1
  awk -v gpio_line="$GPIO_LINE" '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*dtoverlay=gpio-shutdown([,[:space:]]|$)/ {
      if ($0 ~ "gpio_pin=" gpio_line "([,[:space:]]|$)") {
        print NR ":" $0
        found=1
      }
    }
    END { exit(found ? 0 : 1) }
  ' "$BOOT_CONFIG_PATH"
}

power_key_inhibitor_present() {
  command -v "$SYSTEMD_INHIBIT_BIN" >/dev/null 2>&1 || return 1
  "$SYSTEMD_INHIBIT_BIN" --list --no-pager 2>/dev/null \
    | grep -Eq 'handle-power-key|handle-power-key:'
}

diagnose_power_button_policy() {
  local overlay=""
  if native_power_button_present; then
    printf 'native_power_button=yes\n'
  else
    printf 'native_power_button=no\n'
  fi

  overlay="$(active_gpio_shutdown_overlay || true)"
  if [[ -n "$overlay" ]]; then
    printf 'gpio_shutdown_overlay=%s\n' "$overlay"
  else
    printf 'gpio_shutdown_overlay=none\n'
  fi

  if power_key_inhibitor_present; then
    printf 'power_key_inhibitor=yes\n'
  else
    printf 'power_key_inhibitor=no\n'
  fi
}

normalize_pin_value() {
  local value="$1"
  value="${value//$'\n'/}"
  value="${value//\"/}"
  value="${value//\'/}"
  value="${value##*=}"
  value="${value##* }"
  case "${value,,}" in
    1|active)
      printf '1\n'
      return 0
      ;;
    0|inactive)
      printf '0\n'
      return 0
      ;;
  esac
  return 1
}

read_pin() {
  local raw

  if raw="$(gpioget --bias=pull-up --numeric -c "$GPIO_CHIP" "$GPIO_LINE" 2>/dev/null)"; then
    normalize_pin_value "$raw" && return 0
  fi

  if raw="$(gpioget --bias=pull-up --numeric "$GPIO_CHIP" "$GPIO_LINE" 2>/dev/null)"; then
    normalize_pin_value "$raw" && return 0
  fi

  if raw="$(gpioget --bias=pull-up -c "$GPIO_CHIP" "$GPIO_LINE" 2>/dev/null)"; then
    normalize_pin_value "$raw" && return 0
  fi

  if raw="$(gpioget --bias=pull-up "$GPIO_CHIP" "$GPIO_LINE" 2>/dev/null)"; then
    normalize_pin_value "$raw" && return 0
  fi

  return 1
}

detect_g2v1_i2c() {
  command -v i2cget >/dev/null 2>&1 || return 1
  i2cget -y "$I2C_BUS" "$I2C_ADDR" >/dev/null 2>&1
}

request_shutdown() {
  if systemctl poweroff; then
    return 0
  fi
  if command -v sudo >/dev/null 2>&1 && sudo -n systemctl poweroff; then
    return 0
  fi
  log "ERROR: failed to request shutdown (systemctl poweroff)"
  return 1
}

case "${1:-}" in
  --probe-native-power-button)
    native_power_button_present
    exit $?
    ;;
  --diagnose)
    diagnose_power_button_policy
    exit 0
    ;;
  "") ;;
  *)
    printf 'Usage: %s [--probe-native-power-button|--diagnose]\n' "$0" >&2
    exit 2
    ;;
esac

case "${ENABLED,,}" in
  0|false|no|off|disabled)
    log "disabled by config (SATURN_SHUTDOWN_WAITER_ENABLED=$ENABLED)"
    exit 0
    ;;
  1|true|yes|on|enabled)
    log "forced enabled by config"
    ;;
  auto)
    if detect_g2v1_i2c; then
      log "G2V1 I2C device detected at bus $I2C_BUS addr $I2C_ADDR; shutdown waiter not needed"
      exit 0
    fi
    log "No G2V1 I2C device detected; continuing in auto mode"
    ;;
  *)
    log "ERROR: invalid SATURN_SHUTDOWN_WAITER_ENABLED value: $ENABLED"
    exit 1
    ;;
esac

if native_power_button_present; then
  log "native gpio-keys power button detected; polling fallback is not required"
  if overlay="$(active_gpio_shutdown_overlay || true)" && [[ -n "$overlay" ]]; then
    log "WARNING: active gpio-shutdown overlay also targets GPIO${GPIO_LINE}: $overlay"
  fi
  if power_key_inhibitor_present; then
    log "WARNING: a handle-power-key inhibitor is active; the native KEY_POWER event may be ignored"
  fi
  exit 0
fi

if ! command -v gpioget >/dev/null 2>&1; then
  log "ERROR: gpioget not found"
  exit 1
fi

log "Arming delay ${ARM_DELAY_SEC}s on ${GPIO_CHIP} line ${GPIO_LINE}"
sleep "$ARM_DELAY_SEC"

if [[ "$REQUIRE_HIGH_BEFORE_ARM" == "1" ]]; then
  pin="$(read_pin || true)"
  if [[ "$pin" != "1" ]]; then
    log "GPIO line is not high after arm delay (value=${pin:-unreadable}); refusing to arm to avoid false shutdown"
    exit 0
  fi
fi

log "waiting for shutdown trigger..."
low_count=0
while true; do
  pin="$(read_pin || true)"
  if [[ "$pin" == "0" ]]; then
    low_count=$((low_count + 1))
    if (( low_count >= LOW_CONFIRM_COUNT )); then
      log "shutdown request detected (low count=$low_count)"
      request_shutdown
      exit 0
    fi
  else
    low_count=0
  fi
  sleep "$POLL_SEC"
done
