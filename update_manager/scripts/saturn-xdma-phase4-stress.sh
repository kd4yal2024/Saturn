#!/usr/bin/env bash
set -Eeuo pipefail

# Run the RF-inhibited Phase 4 DUC soak while applying bounded host pressure.
# Persistent storage is read-only; the write workload lives in /dev/shm.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="${SATURN_XDMA_STRESS_REPO_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd -P)}"
BRIDGE_BINARY="${SATURN_XDMA_STRESS_BRIDGE_BINARY:-$REPO_ROOT/update_manager/saturn-bridge/target/release/saturn-bridge}"
DURATION_SECONDS="${SATURN_XDMA_STRESS_DURATION_SECONDS:-1800}"
CPU_WORKERS="${SATURN_XDMA_STRESS_CPU_WORKERS:-2}"
MEMORY_MIB="${SATURN_XDMA_STRESS_MEMORY_MIB:-192}"
HTTP_CONCURRENCY="${SATURN_XDMA_STRESS_HTTP_CONCURRENCY:-8}"
RT_PRIORITY="${SATURN_XDMA_STRESS_RT_PRIORITY:-20}"
BLOCK_DEVICE="${SATURN_XDMA_STRESS_BLOCK_DEVICE:-}"
DRY_RUN=0
P2_STOPPED=0
TEMP_DIR=""
MEMORY_FILE=""
declare -a STRESS_PIDS=()
declare -a STRESS_NAMES=()

log(){ printf '[saturn-xdma-phase4-stress] %s\n' "$*"; }
die(){ printf '[saturn-xdma-phase4-stress] ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd(){ command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

usage(){
  cat <<'EOF'
Usage: sudo saturn-xdma-phase4-stress.sh [--duration-seconds SECONDS] [--dry-run]

Runs the direct-XDMA changing-IQ probe under bounded CPU, memory, loopback HTTP,
and read-only block-device pressure. RF remains inhibited by the probe. P2 is
restored on every exit.
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
  local restore_rc=0
  set +e
  for pid in "${STRESS_PIDS[@]}"; do
    kill -TERM "$pid" >/dev/null 2>&1 || true
  done
  for pid in "${STRESS_PIDS[@]}"; do
    wait "$pid" >/dev/null 2>&1 || true
  done
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

if [[ -z "$BLOCK_DEVICE" ]]; then
  BLOCK_DEVICE="$(findmnt -n -o SOURCE -T "$REPO_ROOT" 2>/dev/null || true)"
  BLOCK_DEVICE="${BLOCK_DEVICE%%\[*}"
fi
[[ "$BLOCK_DEVICE" == /dev/* ]] \
  || die "could not resolve a block device for read-only storage pressure: $BLOCK_DEVICE"

if (( DRY_RUN )); then
  log "Would use repository: $REPO_ROOT"
  log "Would read only from: $BLOCK_DEVICE"
  log "Would allocate at most ${MEMORY_MIB} MiB in /dev/shm"
  print_command systemctl stop p2app.service
  print_command fio --name=saturn-xdma-storage --filename="$BLOCK_DEVICE" --readonly \
    --rw=randread --direct=1 --ioengine=libaio --bs=128k --iodepth=8 \
    --time_based=1 --runtime="$((DURATION_SECONDS + 30))"
  print_command fio --name=saturn-xdma-memory --filename=/dev/shm/saturn-xdma-stress \
    --size="${MEMORY_MIB}m" --rw=randrw --direct=0 --ioengine=sync --bs=64k \
    --time_based=1 --runtime="$((DURATION_SECONDS + 30))"
  print_command ab -t "$((DURATION_SECONDS + 30))" -c "$HTTP_CONCURRENCY" -k \
    http://127.0.0.1:8080/readyz
  print_command env SATURN_BRIDGE_XDMA_DUC_DURATION_MS="$((DURATION_SECONDS * 1000))" \
    SATURN_BRIDGE_XDMA_DUC_PATTERN=changing \
    SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY="$RT_PRIORITY" \
    "$BRIDGE_BINARY" --xdma-duc-probe
  print_command systemctl start p2app.service
  exit 0
fi

(( EUID == 0 )) || die "run this stress soak with sudo"
[[ -b "$BLOCK_DEVICE" ]] || die "storage pressure target is not a block device: $BLOCK_DEVICE"
need_cmd ab
need_cmd fio
need_cmd findmnt
need_cmd sha256sum
need_cmd systemctl
[[ -x "$BRIDGE_BINARY" ]] || die "bridge binary is not executable: $BRIDGE_BINARY"
[[ "$(findmnt -n -o FSTYPE -T /dev/shm)" == "tmpfs" ]] \
  || die "/dev/shm is not tmpfs; refusing the memory write workload"

TEMP_DIR="$(mktemp -d /dev/shm/saturn-xdma-phase4-stress.XXXXXX)"
MEMORY_FILE="/dev/shm/saturn-xdma-stress.$$"
STRESS_SECONDS="$((DURATION_SECONDS + 30))"

systemctl stop p2app.service
P2_STOPPED=1
[[ "$(systemctl is-active p2app.service || true)" == "inactive" ]] \
  || die "p2app.service did not stop"

log "Starting ${DURATION_SECONDS}s RF-inhibited soak"
log "Stress: CPU workers=$CPU_WORKERS memory=${MEMORY_MIB}MiB HTTP concurrency=$HTTP_CONCURRENCY"
log "Storage pressure is read-only on $BLOCK_DEVICE"

for ((worker = 1; worker <= CPU_WORKERS; worker++)); do
  sha256sum /dev/zero >"$TEMP_DIR/cpu-$worker.log" 2>&1 &
  STRESS_PIDS+=("$!")
  STRESS_NAMES+=("cpu-$worker")
done

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
  >"$TEMP_DIR/storage-fio.log" 2>&1 &
STRESS_PIDS+=("$!")
STRESS_NAMES+=("storage-read")

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
  >"$TEMP_DIR/memory-fio.log" 2>&1 &
STRESS_PIDS+=("$!")
STRESS_NAMES+=("memory")

ab -t "$STRESS_SECONDS" \
  -c "$HTTP_CONCURRENCY" \
  -k \
  http://127.0.0.1:8080/readyz \
  >"$TEMP_DIR/apachebench.log" 2>&1 &
STRESS_PIDS+=("$!")
STRESS_NAMES+=("loopback-http")

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
