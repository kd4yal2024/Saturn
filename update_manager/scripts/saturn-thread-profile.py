#!/usr/bin/env python3
"""Bounded, read-only Linux thread CPU baseline. Does not attach perf or key RF."""
import argparse
import json
import os
from pathlib import Path
import time


def parse_stat(text):
    # comm can contain spaces and parentheses; fields after its final ')' start
    # at field 3. A task's starttime distinguishes a reused TID/PID.
    close = text.rindex(')')
    fields = text[close + 2:].split()
    return {
        'name': text[text.index('(') + 1:close],
        'ticks': int(fields[11]) + int(fields[12]),
        'startTicks': int(fields[19]),
    }


def snapshot(pid, proc=Path('/proc')):
    tasks = {}
    try:
        for path in (proc / str(pid) / 'task').iterdir():
            try:
                entry = parse_stat((path / 'stat').read_text())
                entry['observedAtMonotonic'] = time.monotonic()
                try:
                    entry.update(scheduler=os.sched_getscheduler(int(path.name)),
                                 priority=os.sched_getparam(int(path.name)).sched_priority,
                                 allowedCpus=sorted(os.sched_getaffinity(int(path.name))))
                except (PermissionError, ProcessLookupError):
                    entry.update(scheduler=None, priority=None, allowedCpus=None)
                tasks[(int(path.name), entry['startTicks'])] = entry
            except (FileNotFoundError, ProcessLookupError):
                continue  # task exited between enumeration, open and read
    except (FileNotFoundError, ProcessLookupError) as error:
        raise RuntimeError('process ended during task enumeration; baseline incomplete') from error
    return tasks


def process_stat(pid, proc):
    try:
        return parse_stat((proc / str(pid) / 'stat').read_text())
    except (FileNotFoundError, ProcessLookupError) as error:
        raise RuntimeError('process ended during accounting; baseline incomplete') from error


def summarize(first, last, seconds, hz):
    if seconds <= 0 or hz <= 0:
        raise ValueError('positive observed interval and clock tick rate required')
    if first['startTicks'] != last['startTicks'] or last['ticks'] < first['ticks']:
        raise ValueError('task changed identity or CPU counter moved backwards')
    cpu = (last['ticks'] - first['ticks']) / hz
    return {'cpuSeconds': cpu, 'cpuPercentOfOneCore': cpu * 100 / seconds}


def perf_snapshot(path):
    try:
        value = json.loads(path.read_text())
        # Only the existing public performance record, never process environment
        # or command lines that might contain credentials.
        return value
    except (OSError, ValueError):
        return None


def collect(pid, seconds, interval, perf_path, proc=Path('/proc')):
    hz = os.sysconf('SC_CLK_TCK')
    started_ms = int(time.time() * 1000)
    begin = time.monotonic()
    process_first = process_stat(pid, proc)
    process_begin = time.monotonic()
    initial = snapshot(pid, proc)
    identity = (pid, process_first['startTicks'])
    if identity not in initial:
        raise RuntimeError('process ended or PID was reused; baseline incomplete')
    first = dict(initial)
    last = dict(initial)
    samples = {key: 1 for key in first}
    before = perf_snapshot(perf_path)
    states = []
    deadline = begin + seconds
    while time.monotonic() < deadline:
        time.sleep(min(interval, max(0, deadline - time.monotonic())))
        current = snapshot(pid, proc)
        if identity not in current:
            raise RuntimeError('process ended or PID was reused; baseline incomplete')
        for key, value in current.items():
            first.setdefault(key, value)
            last[key] = value
            samples[key] = samples.get(key, 0) + 1
        observed = perf_snapshot(perf_path)
        if observed:
            metrics = observed.get('metrics', {})
            state = {key: observed.get(key) for key in ('phase', 'backend', 'tx_owner')}
            state.update({key: metrics.get(key) for key in
                          ('build_git_sha', 'frequency_hz', 'sample_rate_hz', 'processing_mode', 'tx_keyed', 'tx_stream')})
            if not states or states[-1]['state'] != state:
                states.append({'elapsedSeconds': time.monotonic() - begin, 'state': state})
    elapsed = time.monotonic() - begin
    process_last = process_stat(pid, proc)
    process_elapsed = time.monotonic() - process_begin
    process_cpu = summarize(process_first, process_last, process_elapsed, hz)
    rows = []
    for key, value in last.items():
        tid, start = key
        observed_seconds = value['observedAtMonotonic'] - first[key]['observedAtMonotonic']
        cpu = summarize(first[key], value, observed_seconds, hz) if observed_seconds > 0 else {
            'cpuSeconds': 0.0, 'cpuPercentOfOneCore': None}
        rows.append({'tid': tid, 'startTicks': start, 'name': value['name'],
                     'scheduler': value.get('scheduler'), 'priority': value.get('priority'),
                     'allowedCpus': value.get('allowedCpus'),
                     'samples': samples[key], 'observedSeconds': observed_seconds,
                     'firstObservedSeconds': first[key]['observedAtMonotonic'] - begin,
                     'lastObservedSeconds': value['observedAtMonotonic'] - begin,
                     'windowCpuPercentOfOneCore': cpu['cpuSeconds'] * 100 / elapsed,
                     **cpu})
    rows.sort(key=lambda row: row['cpuSeconds'], reverse=True)
    governors = {}
    for path in Path('/sys/devices/system/cpu').glob('cpu[0-9]*/cpufreq/scaling_governor'):
        governors[path.parent.parent.name] = path.read_text().strip()
    return {
        'format': 'saturn-thread-cpu-baseline-v2', 'pid': pid,
        'startedAtUnixMs': started_ms, 'finishedAtUnixMs': int(time.time() * 1000),
        'elapsedSeconds': elapsed, 'sampleIntervalSeconds': interval,
        'clockTicksPerSecond': hz, 'governors': governors, 'threads': rows,
        'totalSampledCpuPercentOfOneCore': sum(row['windowCpuPercentOfOneCore'] for row in rows),
        'totalProcessCpuPercentOfOneCore': process_cpu['cpuPercentOfOneCore'],
        'processObservedSeconds': process_elapsed,
        'scope': 'read-only /proc CPU accounting; one core = 100 percent; per-thread rate uses its observed interval, null for a single observation; sampled total sums whole-window contributions, not different-interval rates; process total includes all threads from /proc/PID/stat; unseen CPU before a task first appears or after it disappears is not attributed to that task; no call stacks or scheduling-latency measurement',
        'performanceCounterScope': 'before/after public snapshots; histogram percentiles are cumulative process-lifetime values, not reset window percentiles',
        'performanceBefore': before, 'performanceAfter': perf_snapshot(perf_path),
        'observedWorkloadTransitions': states,
        'rfKeyedByTool': False,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--seconds', type=float, default=12)
    parser.add_argument('--interval', type=float, default=1)
    parser.add_argument('--perf-path', type=Path, default=Path('/run/saturn-bridge/perf.json'))
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.pid <= 0 or not 0.1 <= args.seconds <= 120 or not 0.1 <= args.interval <= 10:
        parser.error('pid must be positive; seconds 0.1..120; interval 0.1..10')
    report = json.dumps(collect(args.pid, args.seconds, args.interval, args.perf_path), indent=2) + '\n'
    if args.output:
        args.output.write_text(report)
    else:
        print(report, end='')


if __name__ == '__main__':
    main()
