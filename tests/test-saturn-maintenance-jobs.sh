#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BROKER="$REPO_ROOT/update_manager/scripts/saturn-maintenance-lock.py"
TMP_ROOT="$(mktemp -d)"
LOCK_DIR="$TMP_ROOT/locks"
OUTPUT_DIR="$TMP_ROOT/output"
RESULT_DIR="$TMP_ROOT/results"
ORPHAN_GROUP=""

cleanup(){
  if [[ -n "$ORPHAN_GROUP" ]]; then
    kill -KILL -- "-$ORPHAN_GROUP" >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

mkdir -p "$LOCK_DIR" "$OUTPUT_DIR" "$RESULT_DIR"
for resource in release repository disk fpga package network radio read-only; do
  : >"$LOCK_DIR/$resource.lock"
  chmod 0660 "$LOCK_DIR/$resource.lock"
done

set +e
# Worker substitutions intentionally expand in the child shell.
# shellcheck disable=SC2016
setsid "$BROKER" --lock-dir "$LOCK_DIR" run \
  --operation durable-test \
  --resources repository,radio \
  --job-id job-test-1 \
  --output-file "$OUTPUT_DIR/job-test-1.log" \
  --result-file "$RESULT_DIR/job-test-1.json" \
  -- sh -c 'printf "child-pgrp=%s\n" "$(ps -o pgid= -p $$ | tr -d " ")"; printf "durable output\n"; exit 7' \
  >"$TMP_ROOT/live-output.log" 2>"$TMP_ROOT/live-error.log"
status=$?
set -e

[[ "$status" == "7" ]] || { echo "broker did not preserve child exit status" >&2; exit 1; }
grep -Fq 'durable output' "$OUTPUT_DIR/job-test-1.log"
grep -Fq 'durable output' "$TMP_ROOT/live-output.log"

python3 - "$RESULT_DIR/job-test-1.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    result = json.load(stream)
assert result["job_id"] == "job-test-1"
assert result["exit_code"] == 7
assert result["finished_at"]
PY

output_mode="$(stat -c '%a' "$OUTPUT_DIR/job-test-1.log")"
result_mode="$(stat -c '%a' "$RESULT_DIR/job-test-1.json")"
[[ "$output_mode" == "640" ]] || { echo "unexpected output mode: $output_mode" >&2; exit 1; }
[[ "$result_mode" == "640" ]] || { echo "unexpected result mode: $result_mode" >&2; exit 1; }

setsid "$BROKER" --lock-dir "$LOCK_DIR" run \
  --operation bounded-output-test \
  --resources read-only \
  --job-id job-test-bounded \
  --output-file "$OUTPUT_DIR/job-test-bounded.log" \
  --result-file "$RESULT_DIR/job-test-bounded.json" \
  --output-max-bytes 512 \
  --output-max-lines 5 \
  -- sh -c 'seq 1 100 | sed "s/^/noisy-output-/"' \
  >"$TMP_ROOT/bounded-live-output.log"

bounded_bytes="$(stat -c '%s' "$OUTPUT_DIR/job-test-bounded.log")"
bounded_lines="$(wc -l <"$OUTPUT_DIR/job-test-bounded.log")"
(( bounded_bytes <= 512 )) || { echo "bounded output exceeded byte cap: $bounded_bytes" >&2; exit 1; }
(( bounded_lines <= 5 )) || { echo "bounded output exceeded line cap: $bounded_lines" >&2; exit 1; }
grep -Fq '[output truncated: durable log byte/line limit reached' "$OUTPUT_DIR/job-test-bounded.log"
grep -Fq 'noisy-output-100' "$TMP_ROOT/bounded-live-output.log"

set +e
setsid "$BROKER" --lock-dir "$LOCK_DIR" run \
  --operation deadline-test \
  --resources read-only \
  --job-id job-test-timeout \
  --output-file "$OUTPUT_DIR/job-test-timeout.log" \
  --result-file "$RESULT_DIR/job-test-timeout.json" \
  --timeout-seconds 1 \
  -- sh -c 'printf "waiting\n"; sleep 30' \
  >"$TMP_ROOT/timeout-live-output.log" 2>&1
timeout_status=$?
set -e
[[ "$timeout_status" != "0" ]] || { echo "deadline test unexpectedly succeeded" >&2; exit 1; }
python3 - "$RESULT_DIR/job-test-timeout.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    result = json.load(stream)
assert result["timed_out"] is True
assert result["exit_code"] != 0
PY

# A broker killed without cleanup leaves its child process group and inherited
# lock descriptors intact. This is the orphan case reconciled by Saturn Go.
# shellcheck disable=SC2016
setsid "$BROKER" --lock-dir "$LOCK_DIR" run \
  --operation orphan-test \
  --resources repository \
  --job-id job-test-orphan \
  --output-file "$OUTPUT_DIR/job-test-orphan.log" \
  --result-file "$RESULT_DIR/job-test-orphan.json" \
  -- sh -c 'printf "%s\n" "$$" >"$1"; trap "" TERM INT; sleep 30' \
  sh "$TMP_ROOT/orphan-child.pid" >/dev/null 2>&1 &
ORPHAN_GROUP=$!
for _ in {1..50}; do
  [[ -s "$TMP_ROOT/orphan-child.pid" ]] && break
  sleep 0.1
done
[[ -s "$TMP_ROOT/orphan-child.pid" ]] || { echo "orphan test child did not start" >&2; exit 1; }
child_pid="$(<"$TMP_ROOT/orphan-child.pid")"
child_group="$(ps -o pgid= -p "$child_pid" | tr -d ' ')"
[[ "$child_group" == "$ORPHAN_GROUP" ]] || { echo "child is not in broker process group" >&2; exit 1; }
kill -KILL "$ORPHAN_GROUP"
wait "$ORPHAN_GROUP" >/dev/null 2>&1 || true

if "$BROKER" --lock-dir "$LOCK_DIR" probe \
  --operation orphan-conflict --resources repository >/dev/null 2>&1; then
  echo "orphaned maintenance child lost its resource lock" >&2
  exit 1
fi
kill -KILL -- "-$ORPHAN_GROUP" >/dev/null 2>&1 || true
ORPHAN_GROUP=""

echo "Saturn durable maintenance job broker tests passed"
