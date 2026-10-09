#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
# shellcheck source=../scripts/saturn-lcd-lib.sh
source "$REPO_ROOT/scripts/saturn-lcd-lib.sh"

export SATURN_BOOT_CONFIG_BACKUP_DIR="$TMP_DIR/backups"
config="$TMP_DIR/config.txt"
printf '%s\n' '# Trixie defaults' 'arm_64bit=1' '[all]' 'enable_uart=1' >"$config"
chmod 0644 "$config"
original_hash="$(sha256sum "$config" | cut -d' ' -f1)"

append_setting() { printf 'gpio=15=op,dh\n' >>"$1"; }
saturn_boot_config_update "$config" append_setting
grep -Fxq '# Trixie defaults' "$config"
grep -Fxq 'arm_64bit=1' "$config"
grep -Fxq 'gpio=15=op,dh' "$config"
[[ "$(stat -c %a "$config")" == 644 ]]
backup="$(find "$SATURN_BOOT_CONFIG_BACKUP_DIR" -type f -name '*.bak' -print -quit)"
[[ -n "$backup" && "$(sha256sum "$backup" | cut -d' ' -f1)" == "$original_hash" ]]

preserve_content() { :; }
saturn_boot_config_update "$config" preserve_content
[[ "$(find "$SATURN_BOOT_CONFIG_BACKUP_DIR" -type f -name '*.bak' | wc -l)" -eq 1 ]]

before="$(sha256sum "$config" | cut -d' ' -f1)"
empty_content() { : >"$1"; }
if saturn_boot_config_update "$config" empty_content; then
  printf 'empty boot replacement was accepted\n' >&2
  exit 1
fi
[[ "$(sha256sum "$config" | cut -d' ' -f1)" == "$before" ]]

sync() { return 1; }
if saturn_boot_config_update "$config" append_setting; then
  printf 'failed flush was accepted\n' >&2
  exit 1
fi
unset -f sync
[[ "$(sha256sum "$config" | cut -d' ' -f1)" == "$before" ]]

: >"$TMP_DIR/empty.txt"
if saturn_boot_config_update "$TMP_DIR/empty.txt" append_setting; then
  printf 'existing empty boot config was accepted\n' >&2
  exit 1
fi
[[ ! -s "$TMP_DIR/empty.txt" ]]

# The shared LCD writer preserves the stock lines while replacing its own block.
get_boot_config_file() { printf '%s\n' "$config"; }
SATURN_LCD_PROFILE=cm4-7
configure_lcd_profile
grep -Fxq '# Trixie defaults' "$config"
grep -Fxq 'arm_64bit=1' "$config"
grep -Fxq '# Saturn managed LCD profile: cm4-7' "$config"
configure_lcd_profile
[[ "$(grep -Fc '# BEGIN SATURN LCD PROFILE' "$config")" -eq 1 ]]
[[ "$(stat -c %a "$config")" == 644 ]]

printf 'Saturn boot-config update tests passed\n'
