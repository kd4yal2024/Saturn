#!/usr/bin/env bash
set -Eeuo pipefail

# Run the RF-inhibited Phase 4 DUC soak under one bounded pressure profile.
# Persistent storage is read-only; the memory write workload lives in /dev/shm.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="${SATURN_XDMA_STRESS_REPO_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd -P)}"
BRIDGE_BINARY="${SATURN_XDMA_STRESS_BRIDGE_BINARY:-$REPO_ROOT/update_manager/saturn-bridge/target/release/saturn-bridge}"
DURATION_SECONDS="${SATURN_XDMA_STRESS_DURATION_SECONDS:-1800}"
CPU_WORKERS="${SATURN_XDMA_STRESS_CPU_WORKERS:-2}"
MEMORY_MIB="${SATURN_XDMA_STRESS_MEMORY_MIB:-192}"
HTTP_CONCURRENCY="${SATURN_XDMA_STRESS_HTTP_CONCURRENCY:-8}"
RT_PRIORITY="${SATURN_XDMA_STRESS_RT_PRIORITY:-20}"
BLOCK_DEVICE="${SATURN_XDMA_STRESS_BLOCK_DEVICE:-}"
PROFILE="${SATURN_XDMA_STRESS_PROFILE:-combined}"
DRY_RUN=0
P2_STOPPED=0
TEMP_DIR=""
MEMORY_FILE=""
declare -a STRESS_PIDS=()
declare -a STRESS_NAMES=()
declare -a STRESS_LOGS=()

log(){ printf '[saturn-xdma-phase4-stress] %s\n' "$*"; }
die(){ printf '[saturn-xdma-phase4-stress] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage: sudo saturn-xdma-phase4-stress.sh \
  [--profile cpu|memory|network|storage|combined] \
  [--duration-seconds SECONDS] [--dry-run]

Runs the direct-XDMA changing-IQ probe under one bounded host-pressure profile.
The combined profile applies every workload. RF remains inhibited by the probe,
and P2 is restored on every exit.
EOF
}

positive_integer(){
  [[ "$2" =~ ^[1-9][0-9]*$ ]] || die "$1 must be a positive integer, got: $2"
}

print_command(){
  printf '[dry-run]'
  printf ' %q' "$@"
  printf '\n'
}

cleanup(){
  local rc="$?"
  local pid
  local log_file
  local restore_rc=0
  set +e
  for pid in "${STRESS_PIDS[@]}"; do
    kill -TERM "$pid" >/dev/null 2>&1 || true
  done
  for pid in "${STRESS_PIDS[@]}"; do
    wait "$pid" >/dev/null 2>&1 || true
  done
  if (( rc != 0 )) && [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
    for log_file in "${STRESS_LOGS[@]}"; do
      if [[ -s "$log_file" ]]; then
        log "Failure tail: $(basename -- "$log_file")"
        tail -n 20 -- "$log_file" >&2
      fi
    done
  fi
  [[ -z "$MEMORY_FILE" ]] || rm -f -- "$MEMORY_FILE"
  [[ -z "$TEMP_DIR" ]] || rm -rf -- "$TEMP_DIR"
  if (( P2_STOPPED )); then
    if systemctl start p2app.service; then
      log "Restored p2app.service"
    else
      printf '[saturn-xdma-phase4-stress] ERROR: could not restore p2app.service\n' >&2
      restore_rc=1
    fi
  fi
  if (( rc == 0 && restore_rc != 0 )); then
    rc="$restore_rc"
  fi
  return "$rc"
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --duration-seconds)
      [[ $# -ge 2 ]] || die "--duration-seconds requires a value"
      DURATION_SECONDS="$2"
      shift 2
      ;;
    --profile)
      [[ $# -ge 2 ]] || die "--profile requires a value"
      PROFILE="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

case "$PROFILE" in
  cpu|memory|network|storage|combined) ;;
  *) die "profile must be cpu, memory, network, storage, or combined, got: $PROFILE" ;;
esac

uses_profile(){
  [[ "$PROFILE" == "$1" || "$PROFILE" == "combined" ]]
}

positive_integer "duration" "$DURATION_SECONDS"
positive_integer "CPU worker count" "$CPU_WORKERS"
positive_integer "memory MiB" "$MEMORY_MIB"
positive_integer "HTTP concurrency" "$HTTP_CONCURRENCY"
positive_integer "real-time priority" "$RT_PRIORITY"
(( DURATION_SECONDS >= 60 && DURATION_SECONDS <= 86400 )) \
  || die "duration must be between 60 and 86400 seconds"
(( CPU_WORKERS <= 3 )) || die "CPU worker count must not exceed 3"
(( MEMORY_MIB <= 384 )) || die "memory workload must not exceed 384 MiB"
(( HTTP_CONCURRENCY <= 32 )) || die "HTTP concurrency must not exceed 32"
(( RT_PRIORITY <= 80 )) || die "real-time priority must not exceed 80"

if uses_profile storage; then
  if [[ -z "$BLOCK_DEVICE" ]]; then
    BLOCK_DEVICE="$(findmnt -n -o SOURCE -T "$REPO_ROOT" 2>/dev/null || true)"
    BLOCK_DEVICE="${BLOCK_DEVICE%%\[*}"
  fi
  [[ "$BLOCK_DEVICE" == /dev/* ]] \
    || die "could not resolve a block device for read-only storage pressure: $BLOCK_DEVICE"
fi

if (( DRY_RUN )); then
  log "Would use repository: $REPO_ROOT"
  log "Would use stress profile: $PROFILE"
  print_command systemctl stop p2app.service
  if uses_profile cpu; then
    print_command sha256sum /dev/zero
  fi
  if uses_profile memory; then
    log "Would allocate at most ${MEMORY_MIB} MiB in /dev/shm"
    print_command fio --name=saturn-xdma-memory --filename=/dev/shm/saturn-xdma-stress \
      --size="${MEMORY_MIB}m" --rw=randrw --direct=0 --ioengine=sync --bs=64k \
      --time_based=1 --runtime="$((DURATION_SECONDS + 30))"
  fi
  if uses_profile network; then
    print_command ab -t "$((DURATION_SECONDS + 30))" -c "$HTTP_CONCURRENCY" -k \
      http://127.0.0.1:8080/readyz
  fi
  if uses_profile storage; then
    log "Would read only from: $BLOCK_DEVICE"
    print_command fio --name=saturn-xdma-storage --filename="$BLOCK_DEVICE" --readonly \
      --rw=randread --direct=1 --ioengine=libaio --bs=128k --iodepth=8 \
      --time_based=1 --runtime="$((DURATION_SECONDS + 30))"
  fi
  print_command env SATURN_BRIDGE_XDMA_DUC_DURATION_MS="$((DURATION_SECONDS * 1000))" \
    SATURN_BRIDGE_XDMA_DUC_PATTERN=changing \
    SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY="$RT_PRIORITY" \
    "$BRIDGE_BINARY" --xdma-duc-probe
  print_command systemctl start p2app.service
  exit 0
fi

(( EUID == 0 )) || die "run this stress soak with sudo"
need_cmd findmnt
need_cmd systemctl
if uses_profile cpu; then
  need_cmd sha256sum
fi
if uses_profile memory || uses_profile storage; then
  need_cmd fio
fi
if uses_profile network; then
  need_cmd ab
fi
if uses_profile storage; then
  [[ -b "$BLOCK_DEVICE" ]] || die "storage pressure target is not a block device: $BLOCK_DEVICE"
fi
[[ -x "$BRIDGE_BINARY" ]] || die "bridge binary is not executable: $BRIDGE_BINARY"
[[ "$(findmnt -n -o FSTYPE -T /dev/shm)" == "tmpfs" ]] \
  || die "/dev/shm is not tmpfs; refusing temporary stress files"

TEMP_DIR="$(mktemp -d /dev/shm/saturn-xdma-phase4-stress.XXXXXX)"
MEMORY_FILE="/dev/shm/saturn-xdma-stress.$$"
STRESS_SECONDS="$((DURATION_SECONDS + 30))"

systemctl stop p2app.service
P2_STOPPED=1
[[ "$(systemctl is-active p2app.service || true)" == "inactive" ]] \
  || die "p2app.service did not stop"

log "Starting ${DURATION_SECONDS}s RF-inhibited soak with profile=$PROFILE"
if [[ -r /sys/module/xdma/parameters/completion_wq_highpri ]]; then
  log "XDMA completion_wq_highpri=$(</sys/module/xdma/parameters/completion_wq_highpri)"
fi
if [[ -r /sys/module/xdma/parameters/completion_kthread_priority ]]; then
  log "XDMA completion_kthread_priority=$(</sys/module/xdma/parameters/completion_kthread_priority)"
fi
if [[ -r /sys/module/xdma/parameters/transfer_latency_warn_us ]]; then
  log "XDMA transfer_latency_warn_us=$(</sys/module/xdma/parameters/transfer_latency_warn_us)"
fi

if uses_profile cpu; then
  log "CPU pressure: workers=$CPU_WORKERS"
  for ((worker = 1; worker <= CPU_WORKERS; worker++)); do
    log_file="$TEMP_DIR/cpu-$worker.log"
    sha256sum /dev/zero >"$log_file" 2>&1 &
    STRESS_PIDS+=("$!")
    STRESS_NAMES+=("cpu-$worker")
    STRESS_LOGS+=("$log_file")
  done
fi

if uses_profile storage; then
  log "Storage pressure: read-only on $BLOCK_DEVICE"
  log_file="$TEMP_DIR/storage-fio.log"
  fio --name=saturn-xdma-storage \
    --filename="$BLOCK_DEVICE" \
    --readonly \
    --rw=randread \
    --direct=1 \
    --ioengine=libaio \
    --bs=128k \
    --iodepth=8 \
    --time_based=1 \
    --runtime="$STRESS_SECONDS" \
    --group_reporting=1 \
    >"$log_file" 2>&1 &
  STRESS_PIDS+=("$!")
  STRESS_NAMES+=("storage-read")
  STRESS_LOGS+=("$log_file")
fi

if uses_profile memory; then
  log "Memory pressure: ${MEMORY_MIB}MiB in /dev/shm"
  log_file="$TEMP_DIR/memory-fio.log"
  fio --name=saturn-xdma-memory \
    --filename="$MEMORY_FILE" \
    --size="${MEMORY_MIB}m" \
    --rw=randrw \
    --rwmixread=50 \
    --direct=0 \
    --ioengine=sync \
    --bs=64k \
    --time_based=1 \
    --runtime="$STRESS_SECONDS" \
    --fallocate=none \
    --group_reporting=1 \
    >"$log_file" 2>&1 &
  STRESS_PIDS+=("$!")
  STRESS_NAMES+=("memory")
  STRESS_LOGS+=("$log_file")
fi

if uses_profile network; then
  log "Loopback HTTP pressure: concurrency=$HTTP_CONCURRENCY"
  log_file="$TEMP_DIR/apachebench.log"
  ab -t "$STRESS_SECONDS" \
    -c "$HTTP_CONCURRENCY" \
    -k \
    http://127.0.0.1:8080/readyz \
    >"$log_file" 2>&1 &
  STRESS_PIDS+=("$!")
  STRESS_NAMES+=("loopback-http")
  STRESS_LOGS+=("$log_file")
fi

sleep 2
for index in "${!STRESS_PIDS[@]}"; do
  kill -0 "${STRESS_PIDS[$index]}" 2>/dev/null \
    || die "stressor ${STRESS_NAMES[$index]} exited during startup"
done

set +e
env \
  SATURN_BRIDGE_XDMA_DUC_DURATION_MS="$((DURATION_SECONDS * 1000))" \
  SATURN_BRIDGE_XDMA_DUC_PATTERN=changing \
  SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY="$RT_PRIORITY" \
  "$BRIDGE_BINARY" --xdma-duc-probe
PROBE_RC="$?"
set -e

LOAD_RC=0
for index in "${!STRESS_PIDS[@]}"; do
  if ! kill -0 "${STRESS_PIDS[$index]}" 2>/dev/null; then
    log "Stressor exited before probe completion: ${STRESS_NAMES[$index]}"
    LOAD_RC=1
  fi
done

if (( PROBE_RC != 0 )); then
  die "XDMA probe failed under stress with exit code $PROBE_RC"
fi
if (( LOAD_RC != 0 )); then
  die "one or more stressors ended before the probe"
fi

log "Phase 4 stress soak passed"
