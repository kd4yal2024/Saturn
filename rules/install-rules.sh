#!/bin/bash
#
# Install Saturn udev rules into /etc/udev/rules.d.
# This includes the serial-device rule and the XDMA rule/helper used by PCIe access.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
XDMA_RULES_DIR="$SCRIPT_DIR/../linuxdriver/etc/udev/rules.d"
DEST_DIR="/etc/udev/rules.d"

if [ "$EUID" -ne 0 ]; then
    echo "ERROR: This script must be run as root. Please run it with sudo."
    exit 1
fi

declare -a rule_files=()

if [ -f "$SCRIPT_DIR/61-g2-serial.rules" ]; then
    rule_files+=("$SCRIPT_DIR/61-g2-serial.rules")
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
