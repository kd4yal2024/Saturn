#!/usr/bin/env bash
set -Eeuo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
STRESS_SCRIPT="$REPO_ROOT/update_manager/scripts/saturn-xdma-phase4-stress.sh"

fail(){
  printf 'XDMA Phase 4 stress contract failed: %s\n' "$*" >&2
  exit 1
}

bash -n "$STRESS_SCRIPT"

# These literal checks protect the safety properties of a workload that runs
# as root against a mounted appliance.
grep -Fq -- '--readonly' "$STRESS_SCRIPT" \
  || fail "storage workload is not explicitly read-only"
# shellcheck disable=SC2016
grep -Fq -- '[[ -b "$BLOCK_DEVICE" ]]' "$STRESS_SCRIPT" \
  || fail "storage workload does not require a block device"
# shellcheck disable=SC2016
grep -Fq -- '[[ "$(findmnt -n -o FSTYPE -T /dev/shm)" == "tmpfs" ]]' "$STRESS_SCRIPT" \
  || fail "memory write workload is not restricted to tmpfs"
grep -Fq -- 'mktemp -d /dev/shm/' "$STRESS_SCRIPT" \
  || fail "stress logs are not restricted to tmpfs"
grep -Fq -- 'SATURN_BRIDGE_XDMA_DUC_PATTERN=changing' "$STRESS_SCRIPT" \
  || fail "probe does not use the changing-IQ pattern"
# shellcheck disable=SC2016
grep -Fq -- 'SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY="$RT_PRIORITY"' "$STRESS_SCRIPT" \
  || fail "probe does not request bounded real-time scheduling"
grep -Fq -- 'trap cleanup EXIT' "$STRESS_SCRIPT" \
  || fail "script does not install unconditional cleanup"
grep -Fq -- 'systemctl start p2app.service' "$STRESS_SCRIPT" \
  || fail "cleanup does not restore P2"
grep -Fq -- 'cpu|memory|network|storage|combined' "$STRESS_SCRIPT" \
  || fail "script does not expose staged stress profiles"
grep -Fq -- 'Failure tail:' "$STRESS_SCRIPT" \
  || fail "script does not preserve failure diagnostics"

combined_dry_run="$(
  SATURN_XDMA_STRESS_BLOCK_DEVICE=/dev/test-block-device \
    bash "$STRESS_SCRIPT" --profile combined --duration-seconds 60 --dry-run 2>&1
)"
grep -Fq -- '--readonly' <<<"$combined_dry_run" \
  || fail "dry run does not show read-only storage pressure"
grep -Fq 'SATURN_BRIDGE_XDMA_DUC_RT_PRIORITY=20' <<<"$combined_dry_run" \
  || fail "dry run does not show real-time XDMA probe"

for profile in cpu memory network storage; do
  if [[ "$profile" == "storage" ]]; then
    dry_run="$(
      SATURN_XDMA_STRESS_BLOCK_DEVICE=/dev/test-block-device \
        bash "$STRESS_SCRIPT" --profile "$profile" --duration-seconds 60 --dry-run 2>&1
    )"
  else
    dry_run="$(bash "$STRESS_SCRIPT" --profile "$profile" --duration-seconds 60 --dry-run 2>&1)"
  fi
  grep -Fq "stress profile: $profile" <<<"$dry_run" \
    || fail "$profile dry run does not identify its profile"
  grep -Fq -- '--xdma-duc-probe' <<<"$dry_run" \
    || fail "$profile dry run omits the XDMA probe"
done

cpu_dry_run="$(bash "$STRESS_SCRIPT" --profile cpu --duration-seconds 60 --dry-run 2>&1)"
if grep -Fq -- '--readonly' <<<"$cpu_dry_run"; then
  fail "CPU profile unexpectedly includes storage pressure"
fi

printf 'XDMA Phase 4 stress contract passed\n'
