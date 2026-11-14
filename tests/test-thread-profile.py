#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
import tempfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('profile_tool', ROOT / 'update_manager/scripts/saturn-thread-profile.py')
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)


class ThreadAccounting(unittest.TestCase):
    def test_task_exit_between_open_and_read_is_skipped(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / '700001/task/700002').mkdir(parents=True)
            for error in [FileNotFoundError(), ProcessLookupError()]:
                with patch.object(Path, 'read_text', side_effect=error):
                    self.assertEqual(tool.snapshot(700001, root), {})

    def test_process_exit_during_enumeration_or_accounting_is_explicit(self):
        with tempfile.TemporaryDirectory() as directory:
            for action in [tool.snapshot, tool.process_stat]:
                with self.assertRaisesRegex(RuntimeError, 'process ended'):
                    action(700001, Path(directory))
            with patch.object(Path, 'iterdir', side_effect=ProcessLookupError()):
                with self.assertRaisesRegex(RuntimeError, 'enumeration'):
                    tool.snapshot(700001, Path(directory))

    def test_late_and_departing_task_rates_use_observed_intervals_not_the_total_window(self):
        class Clock:
            now = 0.0
            def monotonic(self): return self.now
            def sleep(self, seconds): self.now += seconds
        clock = Clock()
        def sample(pid, proc):
            t = clock.now
            rows = {(7, 10): {'name': 'main', 'ticks': 100 + int(t)*10,
                              'startTicks': 10, 'observedAtMonotonic': t}}
            if t in [1, 2]:
                rows[(8, 11)] = {'name': 'late', 'ticks': int(t)*20,
                                 'startTicks': 11, 'observedAtMonotonic': t}
            if t == 3:
                rows[(9, 12)] = {'name': 'once', 'ticks': 50,
                                 'startTicks': 12, 'observedAtMonotonic': t}
            return rows
        with patch.object(tool.time, 'monotonic', clock.monotonic), patch.object(tool.time, 'sleep', clock.sleep), \
             patch.object(tool, 'snapshot', sample), patch.object(tool, 'perf_snapshot', return_value=None), \
             patch.object(tool, 'process_stat', side_effect=[{'ticks':100,'startTicks':10}, {'ticks':180,'startTicks':10}]), \
             patch.object(tool.os, 'sysconf', return_value=100), patch.object(Path, 'glob', return_value=[]):
            report = tool.collect(7, 4, 1, Path('/unused'))
        rows = {row['name']: row for row in report['threads']}
        self.assertEqual(rows['late']['observedSeconds'], 1)
        self.assertEqual(rows['late']['cpuPercentOfOneCore'], 20)
        self.assertEqual(rows['late']['windowCpuPercentOfOneCore'], 5)
        self.assertIsNone(rows['once']['cpuPercentOfOneCore'])
        self.assertEqual(report['totalSampledCpuPercentOfOneCore'], 15)
        self.assertEqual(report['totalProcessCpuPercentOfOneCore'], 20)

    def test_missing_initial_main_task_and_reused_pid_invalidate_the_run(self):
        with patch.object(tool, 'process_stat', return_value={'ticks':0,'startTicks':10}), \
             patch.object(tool, 'snapshot', return_value={}):
            with self.assertRaisesRegex(RuntimeError, 'process ended or PID was reused'):
                tool.collect(7, .1, .1, Path('/unused'))

    def test_stat_with_spaces_and_parentheses_keeps_field_offsets(self):
        fields = ['S'] + ['0'] * 49
        fields[11], fields[12], fields[19] = '17', '8', '456'
        self.assertEqual(tool.parse_stat('123 (worker (RX) task) ' + ' '.join(fields)),
                         {'name': 'worker (RX) task', 'ticks': 25, 'startTicks': 456})

    def test_cpu_is_percent_of_one_core_not_divided_by_machine_cores(self):
        self.assertEqual(tool.summarize({'ticks': 100, 'startTicks': 7},
                                       {'ticks': 175, 'startTicks': 7}, 2, 100),
                         {'cpuSeconds': 0.75, 'cpuPercentOfOneCore': 37.5})

    def test_reused_tid_or_reversed_counter_is_not_reported_as_zero_cpu(self):
        for last in [{'ticks': 200, 'startTicks': 8}, {'ticks': 99, 'startTicks': 7}]:
            with self.assertRaises(ValueError):
                tool.summarize({'ticks': 100, 'startTicks': 7}, last, 1, 100)


if __name__ == '__main__':
    unittest.main()
