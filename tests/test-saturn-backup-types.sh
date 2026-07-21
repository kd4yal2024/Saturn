#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
MAIN_RS="$REPO_ROOT/update_manager/rust-server/src/main.rs"
BACKUP_RS="$REPO_ROOT/update_manager/rust-server/src/backup.rs"
RESTORE_RS="$REPO_ROOT/update_manager/rust-server/src/restore.rs"
BACKUP_HTML="$REPO_ROOT/update_manager/templates/backup.html"
FORMATS_DOC="$REPO_ROOT/update_manager/docs/BACKUP_FORMATS.md"
INVENTORY_DOC="$REPO_ROOT/update_manager/docs/STATE_INVENTORY.md"

grep -Fq '.route("/backup_settings", get(backup_settings))' "$MAIN_RS"
grep -Fq '.route("/backup_source", get(backup_source))' "$MAIN_RS"
grep -Fq '.route("/backup_releases", get(backup_releases))' "$MAIN_RS"
grep -Fq '.route("/backup_release", get(backup_release))' "$MAIN_RS"
grep -Fq '.route("/backup_full", get(backup_source))' "$MAIN_RS"
grep -Fq '.route("/restore_settings", post(restore_settings))' "$MAIN_RS"
grep -Fq '.route("/restore_source", post(restore_source))' "$MAIN_RS"
grep -Fq '.route("/restore_full", post(restore_source))' "$MAIN_RS"
if grep -Fq 'async fn backup_full' "$MAIN_RS"; then
  printf 'legacy backup_full implementation still exists instead of the source alias\n' >&2
  exit 1
fi

grep -Fq 'const SETTINGS_BACKUP_FORMAT: &str = "saturn-settings-backup";' "$BACKUP_RS"
grep -Fq 'const SETTINGS_BACKUP_SCHEMA_VERSION: u32 = 1;' "$BACKUP_RS"
grep -Fq 'const MAX_SETTINGS_FILE_BYTES: u64 = 16 * 1024 * 1024;' "$BACKUP_RS"
grep -Fq 'const MAX_SETTINGS_TOTAL_BYTES: u64 = 128 * 1024 * 1024;' "$BACKUP_RS"
grep -Fq 'entry.version.as_deref() == Some("custom-default")' "$BACKUP_RS"
grep -Fq 'settings source must be a regular file' "$BACKUP_RS"
grep -Fq 'installed release manifest is missing' "$BACKUP_RS"
grep -Fq 'const RELEASE_MANIFEST_NAME: &str = "release-manifest.json";' "$BACKUP_RS"
if grep -Fq 'share/release/manifest.json' "$BACKUP_RS"; then
  printf 'release backup expects the manifest at the wrong installed path\n' >&2
  exit 1
fi

for required_id in \
  settings-backup-download \
  settings-restore-file \
  settings-restore-validate \
  settings-restore-upload \
  backup-download \
  release-backup-select \
  release-backup-download; do
  grep -Fq "id=\"$required_id\"" "$BACKUP_HTML"
done
grep -Fq "window.location.href = './backup_settings';" "$BACKUP_HTML"
grep -Fq "window.location.href = './backup_source';" "$BACKUP_HTML"
grep -Fq 'Transactional Source Restore' "$BACKUP_HTML"
grep -Fq "xhr.open('POST', \`./restore_settings?" "$BACKUP_HTML"
grep -Fq "'./restore_source?dry_run=1'" "$BACKUP_HTML"
grep -Fq 'pub async fn recover_restore_transactions' "$RESTORE_RS"
if grep -Fq 'Download Full Backup' "$BACKUP_HTML"; then
  printf 'repository archive is still presented as a full backup\n' >&2
  exit 1
fi

for heading in \
  '## 1. Portable settings backup' \
  '## 2. Source repository backup' \
  '## 3. Installed immutable release backup' \
  '## 4. Whole-disk disaster recovery'; do
  grep -Fq "$heading" "$FORMATS_DOC"
done
grep -Fq "Import uses \`POST /restore_settings\`." "$FORMATS_DOC"
grep -Fq 'compatibility alias for the source' "$INVENTORY_DOC"

printf 'Saturn separated backup-type contract tests passed\n'
