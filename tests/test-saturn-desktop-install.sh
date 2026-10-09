#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
fixture="$TMP_DIR/fixture"
home_dir="$TMP_DIR/home"
mkdir -p "$fixture/desktop/icons" "$home_dir"
cp "$REPO_ROOT/desktop/icons/saturn-update-manager.svg" "$fixture/desktop/icons/"
cp "$REPO_ROOT/desktop/icons/saturn-lcd-setup.svg" "$fixture/desktop/icons/"
cp "$REPO_ROOT/desktop/SaturnUpdateManager.desktop" "$fixture/desktop/"

run_installer() {
  HOME="$home_dir" SATURN_ROOT="$fixture" SATURN_SKIP_P2APP_BUILD=1 \
    bash "$REPO_ROOT/scripts/update-desktop-apps.sh" >/dev/null
}
run_installer

for name in BiasCheck AudioTest AXIReaderWriter FlashWriter SaturnLCDSetup SaturnUpdateManager; do
  [[ -s "$home_dir/Desktop/$name.desktop" ]]
  [[ -s "$home_dir/.local/share/applications/$name.desktop" ]]
  grep -Fxq '[Desktop Entry]' "$home_dir/Desktop/$name.desktop"
done
icon="$home_dir/.local/share/icons/hicolor/scalable/apps/saturn-update-manager.svg"
[[ -s "$icon" ]]
[[ -s "$home_dir/.local/share/icons/hicolor/scalable/apps/saturn-lcd-setup.svg" ]]

icon_hash="$(sha256sum "$icon" | cut -d' ' -f1)"
: >"$fixture/desktop/icons/saturn-update-manager.svg"
if run_installer 2>/dev/null; then
  printf 'empty icon source was accepted\n' >&2
  exit 1
fi
[[ "$(sha256sum "$icon" | cut -d' ' -f1)" == "$icon_hash" ]]

cp "$REPO_ROOT/desktop/icons/saturn-update-manager.svg" "$fixture/desktop/icons/"
launcher="$home_dir/Desktop/SaturnUpdateManager.desktop"
launcher_hash="$(sha256sum "$launcher" | cut -d' ' -f1)"
: >"$fixture/desktop/SaturnUpdateManager.desktop"
if run_installer 2>/dev/null; then
  printf 'empty launcher source was accepted\n' >&2
  exit 1
fi
[[ "$(sha256sum "$launcher" | cut -d' ' -f1)" == "$launcher_hash" ]]

printf 'Saturn desktop install tests passed\n'
