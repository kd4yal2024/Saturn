#!/usr/bin/env bash
set -euo pipefail

# Copy Saturn desktop-session autostart entries.
# NOTE: shutdown waiting is now handled by systemd
# (saturn-shutdown-waiter.service), not desktop autostart.

AUTOSTART_DIR="/home/pi/.config/autostart"
SRC_DIR="/home/pi/github/Saturn/autostart-files"

mkdir -p "$AUTOSTART_DIR"
rm -f "${AUTOSTART_DIR}/g2-autostart-p2app.desktop" \
      "${AUTOSTART_DIR}/g2-autostart-piHPSDR.desktop" \
      "${AUTOSTART_DIR}/g2-shutdown.desktop"

if [[ "${1:-}" == "display" ]]; then
  echo "Copying piHPSDR autostart entry"
  cp "${SRC_DIR}/g2-autostart-piHPSDR.desktop" "$AUTOSTART_DIR/"
  echo "Shutdown waiter is managed by systemd (not autostart)"
elif [[ "${1:-}" == "nodisplay" ]]; then
  echo "Copying p2app autostart entry"
  cp "${SRC_DIR}/g2-autostart-p2app.desktop" "$AUTOSTART_DIR/"
  echo "Shutdown waiter is managed by systemd (not autostart)"
else
  echo "usage: ./copy-autostart.sh display (if there is a display on your radio)"
  echo "    or ./copy-autostart.sh nodisplay"
  exit 1
fi
