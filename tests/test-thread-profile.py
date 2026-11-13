#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('profile_tool', ROOT / 'update_manager/scripts/saturn-thread-profile.py')
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)


class ThreadAccounting(unittest.TestCase):
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
