# Status bookkeeping for an order run. Sourced by order.sh and by test_order_status.sh; starts nothing.
#
# The rule: a failed window never stops the run, so that every record is preserved, but it always
# fails the run. The window's real exit status is recorded (no pipe, so nothing swallows it), and
# the final status combines it with the checker's:
#   0  every window ran and the checker accepted the evidence, TCP_NODELAY verified
#   1  a window failed, or the checker found invalid evidence or could not run
#   3  everything is valid but TCP_NODELAY is UNVERIFIED (the checker's own status 3)
FAILED_WINDOWS=()

run_window() {  # run_window INDEX ARM COMMAND [ARGS...]: runs the command, logs it, records a failure
  local idx=$1 arm=$2; shift 2
  local log="${ORDER_DIR:?ORDER_DIR must be set}/window_${idx}_${arm}.log"
  "$@" > "$log" 2>&1
  local status=$?
  tail -n 1 "$log"
  if [ "$status" -ne 0 ]; then
    FAILED_WINDOWS+=("${idx}:${arm}:${status}")
    echo "WINDOW ${idx} (${arm}) FAILED with status ${status}; its record and log are kept"
  fi
  return 0
}

finish_order() {  # finish_order CHECKER_COMMAND [ARGS...]: runs the checker and returns the final status
  local final=0
  if [ "${#FAILED_WINDOWS[@]}" -ne 0 ]; then
    echo "FAILED WINDOWS: ${FAILED_WINDOWS[*]}"
    final=1
  fi
  "$@"
  local checked=$?
  case "$checked" in
    0) ;;
    3) echo "checker: every window valid, TCP_NODELAY UNVERIFIED"; [ "$final" -eq 0 ] && final=3 ;;
    *) echo "checker: invalid evidence or checker error (status ${checked})"; final=1 ;;
  esac
  return "$final"
}
