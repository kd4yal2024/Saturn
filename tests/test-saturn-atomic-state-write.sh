#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
writer="$repo_root/update_manager/scripts/saturn-state-write.py"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

state_file="$tmp_root/policy.json"
last_good="$state_file.last-good"
printf '{"generation":"old"}\n' >"$state_file"
chmod 0640 "$state_file"
original_uid="$(stat -c '%u' "$state_file")"
original_gid="$(stat -c '%g' "$state_file")"

printf '{"generation":"new"}\n' \
  | "$writer" --path "$state_file" --mode 0640 --last-good
jq -e '.generation == "new"' "$state_file" >/dev/null
jq -e '.generation == "new"' "$last_good" >/dev/null
[[ "$(stat -c '%a' "$state_file")" == "640" ]]
[[ "$(stat -c '%u' "$state_file")" == "$original_uid" ]]
[[ "$(stat -c '%g' "$state_file")" == "$original_gid" ]]

printf '{"generation":"old"}\n' >"$state_file"
chmod 0640 "$state_file"
if printf '{"generation":"new"}\n' \
  | "$writer" --path "$state_file" --mode 0640 --last-good --fault before-rename \
    >/dev/null 2>&1; then
  printf 'pre-rename fault injection unexpectedly succeeded\n' >&2
  exit 1
fi
jq -e '.generation == "old"' "$state_file" >/dev/null
jq -e '.generation == "new"' "$last_good" >/dev/null

printf '{"generation":"old"}\n' >"$state_file"
chmod 0640 "$state_file"
if printf '{"generation":"new"}\n' \
  | "$writer" --path "$state_file" --mode 0640 --fault after-rename \
    >/dev/null 2>&1; then
  printf 'post-rename fault injection unexpectedly succeeded\n' >&2
  exit 1
fi
jq -e '.generation == "new"' "$state_file" >/dev/null

cp "$state_file" "$tmp_root/before-invalid.json"
if printf '{not-json\n' \
  | "$writer" --path "$state_file" --mode 0640 --last-good >/dev/null 2>&1; then
  printf 'invalid JSON was accepted\n' >&2
  exit 1
fi
cmp "$tmp_root/before-invalid.json" "$state_file"

if find "$tmp_root" -maxdepth 1 -name '*.tmp' -print -quit | grep -q .; then
  printf 'temporary state file leaked after fault injection\n' >&2
  exit 1
fi

printf 'atomic state writer tests passed\n'
