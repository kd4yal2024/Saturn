#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WAITER="$REPO_ROOT/scripts/shutdown-waiter.sh"
INSTALLER="$REPO_ROOT/scripts/install-shutdown-waiter-service.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/bin" "$TMP_DIR/device-tree" "$TMP_DIR/home" "$TMP_DIR/etc/xdg/autostart"

cat > "$TMP_DIR/input-native" <<'EOF'
I: Bus=0019 Vendor=0001 Product=0001 Version=0100
N: Name="pwr_button"
P: Phys=gpio-keys/input0
S: Sysfs=/devices/platform/pwr_button/input/input0
H: Handlers=kbd event0
B: EV=3
B: KEY=10000000000000 0
EOF

cat > "$TMP_DIR/input-overlay-only" <<'EOF'
I: Bus=0019 Vendor=0001 Product=0001 Version=0100
N: Name="shutdown_button@1a"
P: Phys=gpio-keys/input0
S: Sysfs=/devices/platform/shutdown_button@1a/input/input0
H: Handlers=kbd event0
B: EV=3
B: KEY=10000000000000 0
EOF

cat > "$TMP_DIR/config.txt" <<'EOF'
# dtoverlay=gpio-shutdown,gpio_pin=20
dtoverlay=gpio-shutdown,gpio_pin=26
EOF

cat > "$TMP_DIR/bin/systemd-inhibit" <<'EOF'
#!/usr/bin/env bash
printf 'WHO UID USER PID COMM WHAT WHY MODE\n'
printf 'Power Key Inhibit 1000 pi 42 rpi-gui-nop handle-power-key desktop block\n'
EOF
chmod 0755 "$TMP_DIR/bin/systemd-inhibit"

common_env=(
  SATURN_POWER_DEVICE_TREE_ROOT="$TMP_DIR/device-tree"
  SATURN_POWER_BOOT_CONFIG_PATH="$TMP_DIR/config.txt"
  SATURN_POWER_SYSTEMD_INHIBIT="$TMP_DIR/bin/systemd-inhibit"
)

# A native CM5 gpio-keys input is positively identified.
env "${common_env[@]}" \
  SATURN_POWER_INPUT_DEVICES_PATH="$TMP_DIR/input-native" \
  "$WAITER" --probe-native-power-button

# An overlay-created gpio-keys device alone is not mistaken for a native button.
if env "${common_env[@]}" \
  SATURN_POWER_INPUT_DEVICES_PATH="$TMP_DIR/input-overlay-only" \
  "$WAITER" --probe-native-power-button; then
  printf 'overlay-only input was incorrectly identified as a native power button\n' >&2
  exit 1
fi

diagnostics="$(env "${common_env[@]}" \
  SATURN_POWER_INPUT_DEVICES_PATH="$TMP_DIR/input-native" \
  "$WAITER" --diagnose)"
grep -Fq 'native_power_button=yes' <<<"$diagnostics"
grep -Fq 'gpio_shutdown_overlay=2:dtoverlay=gpio-shutdown,gpio_pin=26' <<<"$diagnostics"
grep -Fq 'power_key_inhibitor=yes' <<<"$diagnostics"

# Native ownership exits before the polling fallback attempts to claim GPIO26.
cat > "$TMP_DIR/bin/i2cget" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
cat > "$TMP_DIR/bin/gpioget" <<EOF
#!/usr/bin/env bash
touch "$TMP_DIR/gpioget-called"
exit 1
EOF
chmod 0755 "$TMP_DIR/bin/i2cget" "$TMP_DIR/bin/gpioget"
native_run="$(env "${common_env[@]}" \
  PATH="$TMP_DIR/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
  SATURN_POWER_INPUT_DEVICES_PATH="$TMP_DIR/input-native" \
  SATURN_SHUTDOWN_WAITER_CONFIG="$TMP_DIR/missing-config" \
  "$WAITER")"
grep -Fq 'native gpio-keys power button detected' <<<"$native_run"
[[ ! -e "$TMP_DIR/gpioget-called" ]]

# The installer writes and removes only its own per-user inhibitor override.
cat > "$TMP_DIR/etc/xdg/autostart/pwrkey.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Exec=systemd-inhibit --what=handle-power-key rpi-gui-nop
EOF
SATURN_USER="$(id -un)"
SATURN_SYSTEM_POWER_KEY_AUTOSTART="$TMP_DIR/etc/xdg/autostart/pwrkey.desktop"
SATURN_SHUTDOWN_WAITER_SOURCE_ONLY=1
export SATURN_USER SATURN_SYSTEM_POWER_KEY_AUTOSTART SATURN_SHUTDOWN_WAITER_SOURCE_ONLY
# shellcheck disable=SC1090
source "$INSTALLER"

write_native_power_key_override "$TMP_DIR/home"
override="$TMP_DIR/home/.config/autostart/pwrkey.desktop"
grep -Fq 'Hidden=true' "$override"
grep -Fq 'X-Saturn-Native-Power-Button=true' "$override"
remove_native_power_key_override "$TMP_DIR/home"
[[ ! -e "$override" ]]

mkdir -p "$(dirname "$override")"
printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Hidden=false' > "$override"
remove_native_power_key_override "$TMP_DIR/home"
[[ -f "$override" ]]
write_native_power_key_override "$TMP_DIR/home"
grep -Fq 'Hidden=false' "$override"
if grep -Fq 'X-Saturn-Native-Power-Button=true' "$override"; then
  printf 'operator-owned power-key override was overwritten\n' >&2
  exit 1
fi

# Native policy disables the waiter and installs the desktop inhibitor override.
rm -f "$override"
cat > "$TMP_DIR/bin/systemctl" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TMP_DIR/systemctl-calls"
EOF
chmod 0755 "$TMP_DIR/bin/systemctl"
export SATURN_POWER_DEVICE_TREE_ROOT="$TMP_DIR/device-tree"
export SATURN_POWER_BOOT_CONFIG_PATH="$TMP_DIR/config.txt"
export SATURN_POWER_SYSTEMD_INHIBIT="$TMP_DIR/bin/systemd-inhibit"
export SATURN_POWER_INPUT_DEVICES_PATH="$TMP_DIR/input-native"
PATH="$TMP_DIR/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export PATH
# Used by apply_power_button_policy(), which is sourced from the installer.
# shellcheck disable=SC2034
DEST_SCRIPT="$WAITER"
saturn_user_home() { printf '%s\n' "$TMP_DIR/home"; }
policy_output="$(apply_power_button_policy)"
grep -Fq 'native pwr_button and gpio-shutdown both claim the configured shutdown GPIO' <<<"$policy_output"
grep -Fq 'Native gpio-keys KEY_POWER input detected; polling waiter disabled' <<<"$policy_output"
grep -Fq 'disable --now saturn-shutdown-waiter.service' "$TMP_DIR/systemctl-calls"
grep -Fq 'X-Saturn-Native-Power-Button=true' "$override"

printf 'shutdown waiter policy tests passed\n'
