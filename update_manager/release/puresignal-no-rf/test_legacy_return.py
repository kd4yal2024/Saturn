"""Deterministic offline checks. No subprocesses, services, devices or sockets."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('legacy', Path(__file__).with_name('wait-legacy-return.py'))
legacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(legacy)


class Clock:
    now = 0.0
    def monotonic(self): return self.now
    def sleep(self, seconds): self.now += seconds
    def wall(self): return 10000 + int(self.now * 1000)


class LegacyReturnTests(unittest.TestCase):
    identity = {'pid': 123, 'start_ticks': '555', 'binary_sha256': 'fixture',
                'rf_enabled': False, 'managed_rf_permitted': False, 'owner': 'legacy'}
    def record(self, stamp=10000, pid=123):
        return {'source': 'saturn-bridge', 'backend': 'xdma', 'status': 'ready',
                'updated_at_ms': stamp, 'metrics': {'pid': pid, 'tx_keyed': False, 'tx_stream': False}}
    def run_wait(self, read, process=None, clock=None, timeout=1):
        clock = clock or Clock()
        return legacy.wait_for_legacy(read, process or (lambda: self.identity), self.identity,
            10000, timeout, monotonic=clock.monotonic, wall_ms=clock.wall, sleep=clock.sleep)
    def test_missing_then_torn_then_fresh_process_file(self):
        calls = [FileNotFoundError(), json.JSONDecodeError('partial', '{', 1), self.record(10500)]
        def read():
            value = calls.pop(0)
            if isinstance(value, Exception): raise value
            return value
        result = self.run_wait(read)
        self.assertEqual(result['attempts'], 3)
        self.assertFalse(result['receiveDeliveryVerified'])
    def test_old_pid_and_predating_file_cannot_complete(self):
        clock = Clock()
        calls = [self.record(pid=122), self.record(stamp=9999), self.record(stamp=10500)]
        self.assertEqual(self.run_wait(lambda: calls.pop(0), clock=clock)['attempts'], 3)
    def test_stalled_publication_and_future_time_are_bounded(self):
        for record in [self.record(stamp=10000-4000), self.record(stamp=20000)]:
            clock = Clock()
            with self.assertRaises(TimeoutError): self.run_wait(lambda: record, clock=clock)
            self.assertEqual(clock.now, 1.0)
    def test_changed_pid_start_time_even_same_executable_fails(self):
        bad = {**self.identity, 'start_ticks': '556'}
        with self.assertRaisesRegex(RuntimeError, 'identity changed'):
            self.run_wait(self.record, process=lambda: bad)
    def test_process_replaced_between_check_and_read_fails(self):
        values = [self.identity, {**self.identity, 'pid': 124}]
        with self.assertRaisesRegex(RuntimeError, 'changed while reading'):
            self.run_wait(self.record, process=lambda: values.pop(0))
    def test_fault_keyed_and_missing_tx_state_cannot_pass(self):
        for field, value in [('status', 'failed'), ('tx_keyed', True), ('tx_stream', None)]:
            record = self.record()
            (record if field == 'status' else record['metrics'])[field] = value
            with self.assertRaises(RuntimeError): self.run_wait(lambda: record)
    def test_fresh_matching_ready_record_passes_first_attempt(self):
        self.assertEqual(self.run_wait(self.record)['attempts'], 1)
    def test_no_unbounded_wait(self):
        for value in [0, -1, 61, float('inf')]:
            with self.assertRaises(ValueError): self.run_wait(self.record, timeout=value)


if __name__ == '__main__':
    unittest.main(verbosity=2)
