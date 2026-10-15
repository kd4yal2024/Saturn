"""Read-only bounded wait for fresh telemetry from one already-started Legacy PID.

Never start/stop a service, release a maintenance guard, write a config or key RF.
Capture --not-before-ms immediately BEFORE the separately authorized start.
Absent/torn/old telemetry is retryable; process replacement or failed state is not.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def wait_for_legacy(read_record, current_process, expected_process, not_before_ms,
                    timeout_seconds=30.0, max_age_ms=3000, monotonic=time.monotonic,
                    wall_ms=lambda: time.time_ns() // 1_000_000, sleep=time.sleep):
    if not 0 < timeout_seconds <= 60 or not 0 < max_age_ms <= 10000 or not_before_ms <= 0:
        raise ValueError('invalid finite wait or freshness boundary')
    end = monotonic() + timeout_seconds
    last_reason = 'telemetry absent'
    attempts = 0
    while monotonic() < end:
        attempts += 1
        process = current_process()
        if process != expected_process:
            raise RuntimeError('Legacy process identity changed during telemetry wait')
        try:
            record = read_record()
            if not isinstance(record, dict):
                raise ValueError('telemetry is not an object')
            metrics = record.get('metrics')
            if not isinstance(metrics, dict):
                raise ValueError('metrics missing')
            stamp = record.get('updated_at_ms')
            if metrics.get('pid') != expected_process['pid']:
                last_reason = 'telemetry from a different PID'
            elif record.get('source') != 'saturn-bridge' or record.get('backend') != 'xdma':
                last_reason = 'wrong telemetry source/backend'
            elif record.get('status') in ('failed', 'error', 'blocked') or record.get('error'):
                raise RuntimeError('Legacy reports failed return')
            elif isinstance(stamp, bool) or not isinstance(stamp, int) or stamp < not_before_ms:
                last_reason = 'telemetry predates authorized start'
            elif not 0 <= wall_ms() - stamp <= max_age_ms:
                last_reason = 'telemetry stale or future dated'
            elif metrics.get('tx_keyed') is not False or metrics.get('tx_stream') is not False:
                raise RuntimeError('Legacy TX state not explicitly off')
            elif record.get('status') == 'ready' and record.get('tx_owner', 'legacy') == 'legacy':
                # Recheck after file read so an old PID/file cannot complete a
                # replaced process's return verification.
                if current_process() != expected_process:
                    raise RuntimeError('Legacy process changed while reading telemetry')
                if monotonic() >= end:
                    raise TimeoutError('Legacy verification exceeded its deadline')
                return {'freshLegacyTelemetry': True, 'pid': expected_process['pid'],
                        'updatedAtMs': stamp, 'attempts': attempts,
                        'rfEnabled': False, 'receiveDeliveryVerified': False}
            else:
                last_reason = 'Legacy not yet ready'
        except (OSError, json.JSONDecodeError, ValueError, UnicodeError) as error:
            last_reason = type(error).__name__
        remaining = end - monotonic()
        if remaining > 0:
            sleep(min(0.25, remaining))
    raise TimeoutError('bounded Legacy telemetry wait expired: ' + last_reason)


def process_identity(pid, expected_sha):
    root = Path('/proc') / str(pid)
    # start_ticks disambiguates recycled PIDs. MainPID must remain the expected
    # service process, not a manually launched lookalike.
    before = (root / 'stat').read_text().rsplit(') ', 1)[1].split()[19]
    actual_sha = hashlib.sha256((root / 'exe').read_bytes()).hexdigest()
    env = dict(item.decode().split('=', 1) for item in (root / 'environ').read_bytes().split(b'\0') if b'=' in item)
    service = subprocess.check_output(['systemctl', 'show', 'saturn-bridge.service',
        '-p', 'MainPID', '-p', 'ActiveState'], text=True, timeout=2,
        env={'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C.UTF-8'})
    props = dict(line.split('=', 1) for line in service.splitlines() if '=' in line)
    after = (root / 'stat').read_text().rsplit(') ', 1)[1].split()[19]
    if before != after or actual_sha != expected_sha or props.get('MainPID') != str(pid) \
            or props.get('ActiveState') != 'active' or env.get('SATURN_BRIDGE_TX_OWNER') != 'legacy' \
            or env.get('SATURN_REMOTE_TX_RF_ENABLED') != '0' \
            or env.get('SATURN_PS_MANAGED_RF_PERMITTED') != '0':
        raise RuntimeError('Legacy identity, active service or RF inhibit check failed')
    return {'pid': pid, 'start_ticks': before, 'binary_sha256': actual_sha,
            'rf_enabled': False, 'managed_rf_permitted': False, 'owner': 'legacy'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--previous-pid', type=int, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--not-before-ms', type=int, required=True)
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('--telemetry', type=Path, default=Path('/run/saturn-bridge/perf.json'))
    args = parser.parse_args()
    if args.pid <= 0 or args.previous_pid <= 0 or args.pid == args.previous_pid \
            or len(args.sha256) != 64 or any(c not in '0123456789abcdef' for c in args.sha256):
        parser.error('positive actual MainPID and lowercase executable SHA256 required')
    expected = process_identity(args.pid, args.sha256)
    def read():
        with args.telemetry.open('rb') as stream:
            data = stream.read(1048577)
        if len(data) > 1048576:
            raise RuntimeError('oversized telemetry record')
        return json.loads(data)
    result = wait_for_legacy(read, lambda: process_identity(args.pid, args.sha256), expected,
                             args.not_before_ms, args.timeout)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
