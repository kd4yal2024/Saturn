#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
lab_dir=$(cd "$script_dir/.." && pwd)

if ! command -v sby >/dev/null 2>&1; then
    echo 'ERROR: sby is not on PATH. Source the OSS CAD Suite environment.' >&2
    exit 1
fi

mkdir -p "$lab_dir/results/formal"
cd "$lab_dir/formal"
for task in prove cover; do
    sby -f -d "$lab_dir/results/formal/watchdog-$task" watchdog.sby "$task"
done
echo 'SATURN_LAB_FORMAL_OK'
