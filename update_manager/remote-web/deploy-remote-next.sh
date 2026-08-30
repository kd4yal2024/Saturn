#!/usr/bin/env bash
set -euo pipefail
SRC_DIR="/home/pi/github/Saturn/update_manager"
DEST_DIR="/var/lib/saturn-web"

sudo install -o nobody -g nogroup -m 644 "$SRC_DIR/remote-web/dist/saturn-remote-next.js" "$DEST_DIR/saturn-remote-next.js"
sudo install -o nobody -g nogroup -m 644 "$SRC_DIR/remote-web/dist/saturn-remote-next.js.sha256" "$DEST_DIR/saturn-remote-next.js.sha256"
sudo install -o nobody -g nogroup -m 644 "$SRC_DIR/templates/saturn-remote-next.html" "$DEST_DIR/saturn-remote-next.html"

echo "--- verify ---"
diff -q "$DEST_DIR/saturn-remote-next.js" "$SRC_DIR/remote-web/dist/saturn-remote-next.js" && echo "JS: MATCHES"
diff -q "$DEST_DIR/saturn-remote-next.html" "$SRC_DIR/templates/saturn-remote-next.html" && echo "HTML: MATCHES"
grep -c "audioInputDeviceId" "$DEST_DIR/saturn-remote-next.js"
grep -c "audio-devices-1" "$DEST_DIR/saturn-remote-next.html"
