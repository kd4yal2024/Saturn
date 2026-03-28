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

if [ "$EUID" -ne 0 ]; then
    echo "ERROR: This script must be run as root. Please run it with sudo."
    exit 1
fi

declare -a rule_files=()
serial_rules_file=""

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
