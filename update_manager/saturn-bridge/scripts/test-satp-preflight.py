#!/usr/bin/env python3
"""Readiness/argument regression tests; no G2 connection or device access."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import mock_open, patch

SCRIPT = Path(__file__).with_name('preflight-satp-staged.sh')
READINESS = SCRIPT.read_text().split("python3 - <<'PY'\n", 1)[1].split('\nPY\n', 1)[0]


class PreflightTests(unittest.TestCase):
    def ready(self):
        return {'backend': 'xdma', 'status': 'ready', 'updated_at_ms': 999_000,
                'metrics': {'tx_keyed': False, 'tx_stream_active': False}}

    def check(self, state):
        with patch('builtins.open', mock_open(read_data=json.dumps(state))), \
                patch('time.time', return_value=1000), \
                contextlib.redirect_stdout(io.StringIO()):
            exec(compile(READINESS, str(SCRIPT), 'exec'), {})

    def test_fresh_rx_passes(self):
        self.check(self.ready())

    def test_wrong_backend_fails(self):
        state = self.ready()
        state['backend'] = 'p2'
        with self.assertRaisesRegex(SystemExit, 'not using XDMA'):
            self.check(state)

    def test_unready_fails(self):
        state = self.ready()
        state['status'] = 'starting'
        with self.assertRaisesRegex(SystemExit, 'not ready'):
            self.check(state)

    def test_stale_and_future_timestamps_fail(self):
        for timestamp in (995_000, 1_000_001):
            state = self.ready()
            state['updated_at_ms'] = timestamp
            with self.subTest(timestamp=timestamp), self.assertRaisesRegex(SystemExit, 'stale'):
                self.check(state)

    def test_keyed_fails(self):
        state = self.ready()
        state['metrics']['tx_keyed'] = True
        with self.assertRaisesRegex(SystemExit, 'keyed'):
            self.check(state)

    def test_active_stream_fails(self):
        state = self.ready()
        state['metrics']['tx_stream_active'] = True
        with self.assertRaisesRegex(SystemExit, 'stream is active'):
            self.check(state)

    def test_missing_or_non_boolean_tx_flags_fail(self):
        for flag in ('tx_keyed', 'tx_stream_active'):
            for value in (None, 0, 'false'):
                state = self.ready()
                state['metrics'][flag] = value
                with self.subTest(flag=flag, value=value), self.assertRaises(SystemExit):
                    self.check(state)

    def test_no_install_mode_or_implicit_action(self):
        for args in ([], ['--install'], ['--check', '--install']):
            result = subprocess.run(['bash', str(SCRIPT), *args], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn('read-only', result.stderr)

    def checksum_case(self, changed, expected):
        # Exercise the actual shell/checksum commands. Only machine architecture
        # is mocked; each case must fail before any live-service inspection.
        with tempfile.TemporaryDirectory(prefix='satp-preflight-test-') as tmp:
            root = Path(tmp)
            (root / 'preflight.sh').write_text(SCRIPT.read_text())
            bindir = root / 'bin'
            bindir.mkdir()
            uname = bindir / 'uname'
            uname.write_text('#!/bin/sh\necho aarch64\n')
            uname.chmod(0o755)
            for name in ('payload', 'source.txt', 'installed.txt'):
                (root / name).write_text('original\n')

            def manifest(destination, names):
                lines = []
                for name in names:
                    digest = hashlib.sha256((root / name).read_bytes()).hexdigest()
                    lines.append(f'{digest}  {name}\n')
                (root / destination).write_text(''.join(lines))

            manifest('SOURCE-SHA256SUMS', ['source.txt'])
            manifest('installed-before.sha256', ['installed.txt'])
            manifest('SHA256SUMS', ['payload', 'SOURCE-SHA256SUMS', 'installed-before.sha256'])
            (root / changed).write_text('changed\n')
            env = dict(os.environ, PATH=str(bindir) + os.pathsep + os.environ['PATH'])
            result = subprocess.run(['bash', str(root / 'preflight.sh'), '--check'],
                                    env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(expected, result.stderr)

    def test_changed_payload_fails(self):
        self.checksum_case('payload', 'Staged bundle checksum mismatch')

    def test_changed_source_fails(self):
        self.checksum_case('source.txt', 'Staged source changed after build')

    def test_changed_installed_baseline_fails(self):
        self.checksum_case('installed.txt', 'Installed bridge/UI changed since staging')


if __name__ == '__main__':
    unittest.main()
