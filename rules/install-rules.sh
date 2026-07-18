#!/bin/bash
#
# Install Saturn udev rules into /etc/udev/rules.d.
# This includes the serial-device rule and the XDMA rule/helper used by PCIe access.
#
# Environment:
#   SATURN_FRONT_PANEL_TYPE=G2V1|G2V2|NONE
#   SATURN_SERIAL_RULES_FILE=<absolute-or-relative-path>

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
XDMA_RULES_DIR="$SCRIPT_DIR/../linuxdriver/etc/udev/rules.d"
DEST_DIR="/etc/udev/rules.d"
SERIAL_RULES_OVERRIDE="${SATURN_SERIAL_RULES_FILE:-}"
DEFAULT_SERIAL_RULES_BASENAME="61-g2-serial.rules"
OPERATOR_USER="${SATURN_USER:-${SUDO_USER:-}}"

if [ "$EUID" -ne 0 ]; then
    echo "ERROR: This script must be run as root. Please run it with sudo."
    exit 1
fi

declare -a rule_files=()
serial_rules_file=""

ensure_operator_xdma_access() {
    local operator_user="$1"

    if [ -z "$operator_user" ] || [ "$operator_user" = "root" ]; then
        return 0
    fi
    if ! id -u "$operator_user" >/dev/null 2>&1; then
        echo "ERROR: Saturn operator user does not exist: $operator_user" >&2
        return 1
    fi
    if id -nG "$operator_user" | tr ' ' '\n' | grep -Fxq saturn-radio; then
        return 0
    fi

    usermod -a -G saturn-radio "$operator_user"
    echo "Added $operator_user to saturn-radio for piHPSDR/deskHPSDR XDMA access."
    echo "Log out and back in, or reboot, before using an existing desktop launcher."
}

resolve_serial_rules_file() {
    if [ -n "$SERIAL_RULES_OVERRIDE" ]; then
        if [ -f "$SERIAL_RULES_OVERRIDE" ]; then
            printf '%s\n' "$SERIAL_RULES_OVERRIDE"
            return 0
        fi
        if [ -f "$SCRIPT_DIR/$SERIAL_RULES_OVERRIDE" ]; then
            printf '%s\n' "$SCRIPT_DIR/$SERIAL_RULES_OVERRIDE"
            return 0
        fi
        echo "WARN: SATURN_SERIAL_RULES_FILE not found: $SERIAL_RULES_OVERRIDE; falling back to defaults." >&2
    fi

    if [ -f "$SCRIPT_DIR/$DEFAULT_SERIAL_RULES_BASENAME" ]; then
        printf '%s\n' "$SCRIPT_DIR/$DEFAULT_SERIAL_RULES_BASENAME"
        return 0
    fi

    return 1
}

serial_rules_file="$(resolve_serial_rules_file || true)"
if [ -n "$serial_rules_file" ]; then
    rule_files+=("$serial_rules_file")
fi
if [ -f "$XDMA_RULES_DIR/60-xdma.rules" ]; then
    rule_files+=("$XDMA_RULES_DIR/60-xdma.rules")
fi
if [ -f "$XDMA_RULES_DIR/xdma-udev-command.sh" ]; then
    rule_files+=("$XDMA_RULES_DIR/xdma-udev-command.sh")
fi

if [ "${#rule_files[@]}" -eq 0 ]; then
    echo "ERROR: No Saturn udev rule files were found."
    exit 1
fi

echo "##############################################################"
echo ""
echo "Installing Saturn udev rules:"
printf ' - %s\n' "${rule_files[@]}"
echo ""
echo "##############################################################"

install -d -m 0755 "$DEST_DIR"
if ! getent group saturn-radio >/dev/null 2>&1; then
    groupadd --system saturn-radio
fi
ensure_operator_xdma_access "$OPERATOR_USER"

for src in "${rule_files[@]}"; do
    base=$(basename "$src")
    mode=0644
    if [ "$base" = "xdma-udev-command.sh" ]; then
        mode=0755
    fi
    install -m "$mode" "$src" "$DEST_DIR/$base"
done

udevadm control --reload-rules
udevadm trigger
udevadm trigger --subsystem-match=xdma || true

echo "✓ Udev rules reloaded successfully."
