#!/bin/bash
# Tests for order_lib.sh and for window.mjs's failure handling. Needs no browser, Bridge or proxy.
#   bash tools/browser-comparison/test_order_status.sh
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIX="$HERE/fixtures/rehearsal-2026-10-09"
failures=0; checks=0
ok()   { checks=$((checks + 1)); }
fail() { checks=$((checks + 1)); failures=$((failures + 1)); echo "FAIL: $*"; }
expect() { # expect DESCRIPTION EXPECTED ACTUAL
  if [ "$2" = "$3" ]; then ok; else fail "$1: expected '$2', got '$3'"; fi
}
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

# A stand-in for window.mjs: succeeds, or fails with the status named in $STUB_FAIL_<idx>, always leaving a record.
cat > "$tmp/stub_window.sh" <<'STUB'
#!/bin/bash
out="$1"; idx="$2"
var="STUB_FAIL_$idx"
echo "{\"stub\": true}" > "$out.json"
if [ -n "${!var:-}" ]; then echo "window $idx crashed: simulated"; exit "${!var}"; fi
echo "window $idx done: 30 samples"
STUB
chmod +x "$tmp/stub_window.sh"

run_three() { # run_three CHECKER_STATUS -> runs three stub windows (failures from STUB_FAIL_*) and finish_order with a stub checker
  ( # a subshell keeps FAILED_WINDOWS and the library state per case
    ORDER_DIR="$tmp/case"; rm -rf "$ORDER_DIR"; mkdir -p "$ORDER_DIR/windows"
    # shellcheck source=order_lib.sh
    source "$HERE/order_lib.sh"
    for i in 1 2 3; do run_window "$i" "X$i" "$tmp/stub_window.sh" "$ORDER_DIR/windows/w$i" "$i"; done
    echo "ran: $(ls "$ORDER_DIR/windows" | tr '\n' ' ')" >> "$tmp/case.log"
    finish_order bash -c "touch '$tmp/checker_ran'; exit $1"
  ) > "$tmp/case.out" 2>&1
  echo $?
}

unset STUB_FAIL_1 STUB_FAIL_2 STUB_FAIL_3
rm -f "$tmp/checker_ran"
expect "all windows ok, checker 0 -> 0" 0 "$(run_three 0)"
expect "all windows ok, checker 3 (TCP_NODELAY unverified) -> 3" 3 "$(run_three 3)"
expect "all windows ok, checker 1 (invalid evidence) -> 1" 1 "$(run_three 1)"
expect "all windows ok, checker 2 (usage error) -> 1" 1 "$(run_three 2)"

export STUB_FAIL_2=7
rm -f "$tmp/checker_ran"
expect "window 2 crashes, checker 0 -> 1 (a failed window always fails the run)" 1 "$(run_three 0)"
grep -q "FAILED WINDOWS: 2:X2:7" "$tmp/case.out" && ok || fail "the failed window and its real status 7 are reported"
expect "the failed window's record is kept" 1 "$(ls "$tmp/case/windows/w2.json" 2>/dev/null | wc -l)"
expect "the later windows still ran" 1 "$(ls "$tmp/case/windows/w3.json" 2>/dev/null | wc -l)"
expect "the failed window's log is kept" 1 "$(grep -c 'crashed: simulated' "$tmp/case/window_2_X2.log")"
[ -f "$tmp/checker_ran" ] && ok || fail "the checker still runs when a window failed, so its report covers every kept record"
expect "window 2 crashes, checker 3 -> 1 (invalid beats unverified)" 1 "$(run_three 3)"
expect "window 2 crashes, checker 1 -> 1" 1 "$(run_three 1)"
unset STUB_FAIL_2

export STUB_FAIL_1=1 STUB_FAIL_3=1
expect "two windows crash -> 1" 1 "$(run_three 0)"
grep -q "FAILED WINDOWS: 1:X1:1 3:X3:1" "$tmp/case.out" && ok || fail "both failed windows are reported"
unset STUB_FAIL_1 STUB_FAIL_3

# The old pattern swallowed a window's status: `cmd | tail -1` returns tail's status. Prove the library cannot be fooled the same way.
( ORDER_DIR="$tmp/pipe"; mkdir -p "$ORDER_DIR"; source "$HERE/order_lib.sh"
  run_window 1 P bash -c 'echo some output; exit 9'
  [ "${FAILED_WINDOWS[*]}" = "1:P:9" ] ) && ok || fail "a window exiting 9 is recorded as 9"

# Integration with the REAL checker on the real fixtures (two windows, planned order C,A-off): accepted, status 0.
fixture_run() { # fixture_run EXPECTED_STATUS DESCRIPTION [window files to leave out...]
  local want=$1 desc=$2; shift 2
  local d="$tmp/real"; rm -rf "$d"; mkdir -p "$d/windows"
  cp "$FIX/windows/w1_C.json" "$FIX/windows/w2_A-off.json" "$d/windows/"
  for omit in "$@"; do rm -f "$d/windows/$omit"; done
  local status
  ( ORDER_DIR="$d"; source "$HERE/order_lib.sh"
    for i in 1 2; do
      case $i in 1) arm=C;; 2) arm=A-off;; esac
      [ -f "$d/windows/w${i}_$arm.json" ] || FAILED_WINDOWS+=("${i}:${arm}:1")  # a window that crashed and left no record
    done
    finish_order python3 "$HERE/check_order.py" "$d/windows" --bridge-logs "$FIX/bridge-logs" --order C,A-off --report-dir "$d/report" ) > "$d/out" 2>&1
  status=$?
  expect "$desc" "$want" "$status"
}
fixture_run 0 "real checker on two valid fixture windows -> 0"
fixture_run 1 "a window that crashed without a record -> 1" w2_A-off.json

# window.mjs: when it cannot even reach Chrome it still writes a record carrying the failure, and exits nonzero.
if command -v node > /dev/null 2>&1; then
  node "$HERE/window.mjs" 59998 user:pw http://127.0.0.1:1/x "$tmp/none" 1 "$tmp/w1_C" c '{"index":1,"arm":"C"}' > "$tmp/node.out" 2>&1
  status=$?
  [ "$status" -ne 0 ] && ok || fail "window.mjs exits nonzero when it fails (got $status)"
  [ -f "$tmp/w1_C.json" ] && ok || fail "window.mjs writes a record even when it fails"
  python3 -c "
import json,sys
r=json.load(open('$tmp/w1_C.json'))
sys.exit(0 if r.get('failure',{}).get('message') and r['meta']['arm']=='C' and r['samples']==[] else 1)" && ok || fail "the record carries the failure and the window's identity"
  # ...and the checker rejects that record.
  mkdir -p "$tmp/failedwin"; cp "$tmp/w1_C.json" "$tmp/failedwin/"
  python3 "$HERE/check_order.py" "$tmp/failedwin" --mode single --report-dir "$tmp/failedwin/report" > /dev/null 2>&1
  expect "the checker rejects a record that carries a failure" 1 "$?"
else
  echo "node not found: skipping the window.mjs failure test"
fi

echo "$checks checks, $failures failed"
[ "$failures" -eq 0 ]
