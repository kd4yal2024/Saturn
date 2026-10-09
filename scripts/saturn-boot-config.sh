#!/usr/bin/env bash
# Shared, crash-resistant updates for Raspberry Pi boot files. Source this file.

saturn_boot_config_update() {
  local target="$1" edit_function="$2" backup_dir backup_file temp_file expected_hash actual_hash
  shift 2

  if [[ ! -f "$target" || -L "$target" || ! -s "$target" ]]; then
    printf '[saturn-boot-config] Refusing to edit missing, linked, or empty boot file: %s\n' "$target" >&2
    return 1
  fi

  temp_file="$(mktemp "${target}.saturn.XXXXXX")" || return 1
  if ! cp -- "$target" "$temp_file" \
      || ! chmod --reference="$target" "$temp_file" \
      || ! "$edit_function" "$temp_file" "$@"; then
    rm -f -- "$temp_file"
    printf '[saturn-boot-config] Edit failed; original preserved: %s\n' "$target" >&2
    return 1
  fi
  if [[ ! -s "$temp_file" ]]; then
    rm -f -- "$temp_file"
    printf '[saturn-boot-config] Refusing empty replacement for %s\n' "$target" >&2
    return 1
  fi
  if cmp -s -- "$target" "$temp_file"; then
    rm -f -- "$temp_file"
    return 0
  fi

  backup_dir="${SATURN_BOOT_CONFIG_BACKUP_DIR:-/var/backups/saturn-boot-config}"
  if ! install -d -m 0700 -- "$backup_dir"; then
    rm -f -- "$temp_file"
    return 1
  fi
  backup_file="$(mktemp "${backup_dir}/$(basename "$target").XXXXXX.bak")" || {
    rm -f -- "$temp_file"
    return 1
  }
  if ! cp -- "$target" "$backup_file" || ! cmp -s -- "$target" "$backup_file" \
      || ! sync -f "$backup_file" || ! sync -f "$temp_file"; then
    rm -f -- "$temp_file"
    printf '[saturn-boot-config] Backup or flush failed; original preserved: %s\n' "$target" >&2
    return 1
  fi

  expected_hash="$(sha256sum "$temp_file")" || {
    rm -f -- "$temp_file"
    return 1
  }
  expected_hash="${expected_hash%% *}"
  if ! mv -f -- "$temp_file" "$target" || ! sync -f "$target"; then
    printf '[saturn-boot-config] Replacement/flush failed for %s; backup: %s\n' "$target" "$backup_file" >&2
    return 1
  fi
  actual_hash="$(sha256sum "$target")" || {
    printf '[saturn-boot-config] Could not verify %s; backup: %s\n' "$target" "$backup_file" >&2
    return 1
  }
  actual_hash="${actual_hash%% *}"
  if [[ "$actual_hash" != "$expected_hash" ]]; then
    printf '[saturn-boot-config] Verification failed for %s; backup: %s\n' "$target" "$backup_file" >&2
    return 1
  fi
}
