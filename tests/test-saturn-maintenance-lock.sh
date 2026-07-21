#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BROKER="$REPO_ROOT/update_manager/scripts/saturn-maintenance-lock.py"
INSTALLER="$REPO_ROOT/update_manager/install_saturn_go_nginx.sh"
TMP_ROOT="$(mktemp -d)"
LOCK_DIR="$TMP_ROOT/locks"
BROKER_PID=""
READ_ONLY_PID=""

cleanup(){
  if [[ -n "$BROKER_PID" ]]; then
    kill "$BROKER_PID" >/dev/null 2>&1 || true
    wait "$BROKER_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$READ_ONLY_PID" ]]; then
    kill "$READ_ONLY_PID" >/dev/null 2>&1 || true
    wait "$READ_ONLY_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

mkdir -p "$LOCK_DIR"
for resource in release repository disk fpga package network radio read-only; do
  : >"$LOCK_DIR/$resource.lock"
  chmod 0660 "$LOCK_DIR/$resource.lock"
done

# Worker parameters intentionally expand in the child shell.
# shellcheck disable=SC2016
"$BROKER" --lock-dir "$LOCK_DIR" run \
  --operation appliance-update \
  --resources release,repository,package,radio \
  -- sh -c 'printf "%s\n" "$$" >"$1"; touch "$2"; trap "exit 0" TERM INT; while :; do sleep 1; done' \
  sh "$TMP_ROOT/worker.pid" "$TMP_ROOT/started" &
BROKER_PID=$!

for _ in {1..50}; do
  [[ -f "$TMP_ROOT/started" ]] && break
  sleep 0.1
done
[[ -f "$TMP_ROOT/started" ]] || { echo "maintenance worker did not start" >&2; exit 1; }

# The broker, not the calling web process, owns the locks while its child runs.
# A later Saturn Go process therefore cannot enter an overlapping operation.
if "$BROKER" --lock-dir "$LOCK_DIR" probe \
  --operation second-server-update --resources repository 2>"$TMP_ROOT/conflict.err"; then
  echo "conflicting repository operation unexpectedly acquired its lock" >&2
  exit 1
fi
grep -Fq 'resource is busy: repository' "$TMP_ROOT/conflict.err"

# Independent resources remain usable, and read-only operations use shared locks.
"$BROKER" --lock-dir "$LOCK_DIR" probe \
  --operation network-change --resources network >/dev/null
"$BROKER" --lock-dir "$LOCK_DIR" run \
  --operation diagnostics --resources read-only \
  -- sh -c 'trap "exit 0" TERM INT; while :; do sleep 1; done' &
READ_ONLY_PID=$!
sleep 0.2
"$BROKER" --lock-dir "$LOCK_DIR" probe \
  --operation diagnostics-2 --resources read-only >/dev/null
kill "$READ_ONLY_PID"
wait "$READ_ONLY_PID" || true
READ_ONLY_PID=""

kill "$BROKER_PID"
wait "$BROKER_PID" || true
BROKER_PID=""
for _ in {1..50}; do
  if "$BROKER" --lock-dir "$LOCK_DIR" probe \
    --operation after-completion --resources repository >/dev/null 2>&1; then
    released=1
    break
  fi
  sleep 0.1
done
[[ "${released:-0}" == "1" ]] || { echo "repository lock was not released" >&2; exit 1; }

grep -Fq 'SATURN_MAINTENANCE_LOCK_TOOL=' "$INSTALLER"
grep -Fq "f \$SATURN_MAINTENANCE_LOCK_DIR/fpga.lock" "$INSTALLER"
grep -Fq "f \$SATURN_MAINTENANCE_LOCK_DIR/disk.lock" "$INSTALLER"

echo "Saturn host-level maintenance lock tests passed"
