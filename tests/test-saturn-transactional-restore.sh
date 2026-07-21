#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tool="$repo_root/update_manager/scripts/saturn-restore-transaction.py"
installer="$repo_root/update_manager/install_saturn_go_nginx.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

state="$work/state"
scripts="$work/scripts"
pihpsdr="$work/pihpsdr"
deskhpsdr="$work/deskhpsdr"
archive="$work/saturn-settings-v1"
current="$work/current-repo"
source_repo="$work/source-repo"
mkdir -p \
  "$state" "$scripts" "$pihpsdr" "$deskhpsdr" \
  "$archive/saturn-state" "$archive/custom-scripts" \
  "$current/.git" "$current/update_manager" \
  "$source_repo/.git" "$source_repo/update_manager"

printf 'old source\n' >"$current/source-version.txt"
printf 'new source\n' >"$source_repo/source-version.txt"
printf '#!/usr/bin/env bash\nexit 0\n' >"$source_repo/run-test.sh"
chmod 0755 "$source_repo/run-test.sh"
printf '%s\n' "$current" >"$state/repo_root.txt"
printf '{"value":"old"}\n' >"$state/remote_settings.json"
printf '[{"filename":"operator.sh","version":"operator"}]\n' >"$state/custom_scripts.json"
printf '#!/usr/bin/env bash\nprintf "old script\\n"\n' >"$scripts/operator.sh"
chmod 0755 "$scripts/operator.sh"

printf '{"format":"saturn-persistent-state","schema_version":1,"state_schema_version":1}\n' \
  >"$archive/saturn-state/state-schema.json"
printf '{"value":"new"}\n' >"$archive/saturn-state/remote_settings.json"
printf '[{"filename":"operator.sh","version":"operator"}]\n' \
  >"$archive/saturn-state/custom_scripts.json"
printf '%s\n' "$source_repo" >"$archive/saturn-state/repo_root.txt"
printf '#!/usr/bin/env bash\nprintf "new script\\n"\n' >"$archive/custom-scripts/operator.sh"
chmod 0755 "$archive/custom-scripts/operator.sh"

python3 - "$archive" <<'PY'
import hashlib
import json
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
files = []
for path in sorted(root.rglob("*")):
    if not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    content = path.read_bytes()
    files.append({
        "inventory_id": "test",
        "archive_path": relative,
        "size": len(content),
        "mode": path.stat().st_mode & 0o777,
        "sha256": hashlib.sha256(content).hexdigest(),
    })
manifest = {
    "format": "saturn-settings-backup",
    "schema_version": 1,
    "files": files,
}
(root / "manifest.json").write_text(json.dumps(manifest, sort_keys=True) + "\n")
PY

settings_args=(
  settings
  --state-root "$state"
  --archive-root "$archive"
  --scripts-root "$scripts"
  --pihpsdr-root "$pihpsdr"
  --deskhpsdr-root "$deskhpsdr"
)

dry_run="$($tool "${settings_args[@]}" --dry-run)"
jq -e '.status == "ok" and .dry_run == true and .include_host_policy == false' \
  <<<"$dry_run" >/dev/null
grep -q '"old"' "$state/remote_settings.json"
grep -q 'old script' "$scripts/operator.sh"
grep -Fxq "$current" "$state/repo_root.txt"

if SATURN_RESTORE_TEST_AVAILABLE_BYTES=0 "$tool" "${settings_args[@]}" >/dev/null 2>&1; then
  echo "expected settings restore ENOSPC preflight to fail" >&2
  exit 1
fi
grep -q '"old"' "$state/remote_settings.json"
grep -q 'old script' "$scripts/operator.sh"

set +e
SATURN_RESTORE_FAILPOINT=settings_after_1 "$tool" "${settings_args[@]}" >/dev/null 2>&1
fail_status=$?
set -e
if [[ "$fail_status" -ne 97 ]]; then
  echo "expected settings failpoint exit 97, got $fail_status" >&2
  exit 1
fi
recovery="$($tool recover --state-root "$state")"
jq -e '.status == "ok" and (.recovered | length) == 1' <<<"$recovery" >/dev/null
grep -q '"old"' "$state/remote_settings.json"
grep -q 'old script' "$scripts/operator.sh"
grep -Fxq "$current" "$state/repo_root.txt"

settings_result="$($tool "${settings_args[@]}")"
jq -e '.status == "ok" and .dry_run == false and .files == 4' \
  <<<"$settings_result" >/dev/null
grep -q '"new"' "$state/remote_settings.json"
grep -q 'new script' "$scripts/operator.sh"
grep -Fxq "$current" "$state/repo_root.txt"
test "$(stat -c '%a' "$state/remote_settings.json")" = "640"
test "$(stat -c '%a' "$scripts/operator.sh")" = "755"

source_args=(
  source
  --state-root "$state"
  --source-root "$source_repo"
  --current-repo-root "$current"
  --repo-root-file "$state/repo_root.txt"
)
source_dry_run="$(SATURN_RESTORE_TEST_AVAILABLE_BYTES=1073741824 \
  "$tool" "${source_args[@]}" --dry-run)"
jq -e '.status == "ok" and .dry_run == true and .bytes > 0' \
  <<<"$source_dry_run" >/dev/null

set +e
SATURN_RESTORE_TEST_AVAILABLE_BYTES=1073741824 \
  SATURN_RESTORE_FAILPOINT=source_after_copy \
  "$tool" "${source_args[@]}" >/dev/null 2>&1
fail_status=$?
set -e
if [[ "$fail_status" -ne 97 ]]; then
  echo "expected source copy failpoint exit 97, got $fail_status" >&2
  exit 1
fi
recovery="$($tool recover --state-root "$state")"
jq -e '.status == "ok" and (.recovered | length) == 1' <<<"$recovery" >/dev/null
grep -Fxq "$current" "$state/repo_root.txt"
if find "$state/repository-restores" -maxdepth 1 -name '*.staging' | grep -q .; then
  echo "startup recovery left a partial source staging directory" >&2
  exit 1
fi

set +e
SATURN_RESTORE_TEST_AVAILABLE_BYTES=1073741824 \
  SATURN_RESTORE_FAILPOINT=source_after_pointer \
  "$tool" "${source_args[@]}" >/dev/null 2>&1
fail_status=$?
set -e
if [[ "$fail_status" -ne 97 ]]; then
  echo "expected source failpoint exit 97, got $fail_status" >&2
  exit 1
fi
grep -q 'new source' "$(cat "$state/repo_root.txt")/source-version.txt"
recovery="$($tool recover --state-root "$state")"
jq -e '.status == "ok" and (.recovered | length) == 1' <<<"$recovery" >/dev/null
grep -Fxq "$current" "$state/repo_root.txt"
grep -q 'old source' "$current/source-version.txt"

source_result="$(SATURN_RESTORE_TEST_AVAILABLE_BYTES=1073741824 \
  "$tool" "${source_args[@]}")"
new_root="$(jq -r '.new_repo_root' <<<"$source_result")"
jq -e '.status == "ok" and .dry_run == false' <<<"$source_result" >/dev/null
grep -Fxq "$new_root" "$state/repo_root.txt"
grep -q 'new source' "$new_root/source-version.txt"
test "$(stat -c '%a' "$new_root/run-test.sh")" = "755"
grep -q 'old source' "$current/source-version.txt"

grep -Fq 'SATURN_REPOSITORY_RESTORE_DIR=' "$installer"
grep -Fq -- "-path \"\$SATURN_REPOSITORY_RESTORE_DIR\"" "$installer"

status_json="$($tool status --state-root "$state")"
jq -e '[.transactions[] | select(.status == "committed")] | length == 2' \
  <<<"$status_json" >/dev/null

echo "Saturn transactional restore tests passed"
