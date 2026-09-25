#!/usr/bin/env bash
# Deploy the remote-next page and bundle to the Saturn web root.
#
# Run as root:  sudo /path/to/repo/update_manager/remote-web/deploy-remote-next.sh
#
# This is the surgical path (two files, no service rebuild). Prefer
# `sudo ./install.sh` for a full install; both paths now refuse to deploy an
# asset set that is missing the High-Res 3D waterfall.
#
# Why the guard exists: on 2026-09-25 this script and `install.sh` deployed from
# a checkout parked on a stale side branch, which silently replaced the live
# page with one that had no High-Res 3D waterfall and no display settings.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SATURN_SOURCE_DIR:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
WEB_ROOT="${SATURN_WEB_ROOT:-/var/lib/saturn-web}"

SRC_HTML="$REPO_ROOT/update_manager/templates/saturn-remote-next.html"
SRC_JS="$REPO_ROOT/update_manager/remote-web/dist/saturn-remote-next.js"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "[ERR] run as root: sudo $0" >&2
  exit 1
fi
if [[ ! -s "$SRC_HTML" ]]; then
  echo "[ERR] missing $SRC_HTML" >&2
  exit 1
fi
if [[ ! -s "$SRC_JS" ]]; then
  echo "[ERR] missing $SRC_JS" >&2
  echo "[ERR] build it first: cd $REPO_ROOT/update_manager/remote-web && npm ci && npm run build" >&2
  exit 1
fi

# Refuse to downgrade the live UI.
if ! grep -q "terrain-canvas" "$SRC_HTML"; then
  echo "[ERR] $SRC_HTML has no High-Res 3D markup; refusing to deploy" >&2
  echo "[ERR] The checkout is probably stale: git -C $REPO_ROOT fetch origin && git -C $REPO_ROOT checkout main && git -C $REPO_ROOT pull --ff-only" >&2
  exit 1
fi
if ! grep -q "TerrainRenderer" "$SRC_JS"; then
  echo "[ERR] $SRC_JS has no terrain renderer; refusing to deploy" >&2
  echo "[ERR] Rebuild remote-web, then re-run this script." >&2
  exit 1
fi

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$WEB_ROOT"
for dest in "$WEB_ROOT/saturn-remote-next.html" "$WEB_ROOT/saturn-remote-next.js"; do
  if [[ -e "$dest" ]]; then
    cp -a "$dest" "$dest.$STAMP.bak"
    echo "backed up $(basename "$dest") -> $(basename "$dest").$STAMP.bak"
  fi
done

install -o root -g root -m 0644 "$SRC_HTML" "$WEB_ROOT/saturn-remote-next.html"
install -o root -g root -m 0644 "$SRC_JS" "$WEB_ROOT/saturn-remote-next.js"

# The web root checksum file is verified with `cd "$WEB_ROOT" && sha256sum -c`.
printf '%s  saturn-remote-next.js\n' "$(sha256sum "$SRC_JS" | awk '{print $1}')" \
  >"$WEB_ROOT/saturn-remote-next.js.sha256"
chown root:root "$WEB_ROOT/saturn-remote-next.js.sha256"
chmod 0644 "$WEB_ROOT/saturn-remote-next.js.sha256"

if command -v git >/dev/null 2>&1 && git -C "$REPO_ROOT" rev-parse --show-toplevel >/dev/null 2>&1; then
  {
    printf 'branch=%s\n' "$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD)"
    printf 'commit=%s\n' "$(git -C "$REPO_ROOT" rev-parse HEAD)"
    printf 'installed_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$WEB_ROOT/.saturn-web-revision"
  chown root:root "$WEB_ROOT/.saturn-web-revision"
  chmod 0644 "$WEB_ROOT/.saturn-web-revision"
fi

echo "--- verify ---"
printf 'template terrain-canvas: %s\n' "$(grep -c 'terrain-canvas' "$WEB_ROOT/saturn-remote-next.html")"
printf 'template view-3d:        %s\n' "$(grep -c 'view-3d' "$WEB_ROOT/saturn-remote-next.html")"
printf 'bundle TerrainRenderer:  %s\n' "$(grep -c 'TerrainRenderer' "$WEB_ROOT/saturn-remote-next.js")"
( cd "$WEB_ROOT" && sha256sum -c saturn-remote-next.js.sha256 )
echo
echo "OK - hard-reload the page (Ctrl-Shift-R):"
echo "  http://192.168.0.139/saturn/remote-next"
echo "  https://192.168.0.139:8443/remote-next"
