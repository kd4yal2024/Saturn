#!/usr/bin/env bash
# No tracked symbolic link may point outside the repository or use an absolute path: a link made on one machine (for example a node_modules
# directory borrowed from a scratch checkout) breaks every clean checkout and publishes a path that means nothing anywhere else.
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$REPO_ROOT"
bad=0
while IFS= read -r -d '' path; do
  target="$(readlink -- "$path")"
  case "$target" in
    /*)
      printf 'tracked symbolic link with an absolute target: %s -> %s\n' "$path" "$target" >&2
      bad=1
      continue
      ;;
  esac
  resolved="$(realpath -m -- "$(dirname -- "$path")/$target")"
  case "$resolved" in
    "$REPO_ROOT"|"$REPO_ROOT"/*) ;;
    *)
      printf 'tracked symbolic link leaves the repository: %s -> %s\n' "$path" "$target" >&2
      bad=1
      ;;
  esac
done < <(git ls-files -z -s | python3 -c '
import sys
data = sys.stdin.buffer.read().split(b"\0")
for entry in data:
    if entry.startswith(b"120000 "):
        sys.stdout.buffer.write(entry.split(b"\t", 1)[1] + b"\0")
')
if (( bad )); then
  exit 1
fi
printf 'No tracked symbolic link points outside the repository\n'
