#!/usr/bin/env python3
"""Filesystem/failure-injection tests; never run systemctl or access radio devices."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('installer', HERE / 'deploy-satp-staged.py')
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class FakeSystem:
    def __init__(self):
        self.events = []
        self.running = True
        self.starts_to_fail = 0
        self.fail_preflight_at = None
        self.fail_stop = False
        self.fail_guard = False

    def preflight(self, stage):
        self.events.append('check')
        if self.events.count('check') == self.fail_preflight_at:
            raise RuntimeError('RX preflight rejected')

    def stop(self):
        self.events.append('stop')
        if self.fail_stop:
            raise RuntimeError('stop failed')
        self.running = False

    def start_ready(self, installed):
        self.events.append('start')
        self.running = True
        if self.starts_to_fail:
            self.starts_to_fail -= 1
            raise RuntimeError('readiness failed')

    def guard_rollback(self):
        if self.fail_guard:
            raise RuntimeError('TX is keyed')


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='satp-installer-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.stage = self.root / 'stage'
        self.stage.mkdir()
        payload, baseline = [], []
        for source, target, mode in installer.FILES:
            src = self.stage / source
            dst = self.root / target.lstrip('/')
            src.parent.mkdir(parents=True, exist_ok=True)
            dst.parent.mkdir(parents=True, exist_ok=True)
            src.write_text('new ' + dst.name)
            dst.write_text('old ' + dst.name)
            dst.chmod(mode)
            payload.append(f'{installer.digest(src)}  {source}\n')
            baseline.append(f'{installer.digest(dst)}  {target}\n')
        (self.stage / 'SHA256SUMS').write_text(''.join(payload))
        (self.stage / 'installed-before.sha256').write_text(''.join(baseline))
        self.system = FakeSystem()
        self.deployer = installer.Deployer(self.stage, self.system, self.root)
        # Test under an ordinary user; verify ownership calls without chown root.
        for mocked in (patch.object(installer.os, 'fchown'), patch.object(installer.os, 'sync'),
                       patch.object(installer.subprocess, 'run', side_effect=AssertionError('No real commands in unit tests'))):
            mocked.start()
            self.addCleanup(mocked.stop)
        output = contextlib.redirect_stdout(io.StringIO())
        output.__enter__()
        self.addCleanup(output.__exit__, None, None, None)
        errors = contextlib.redirect_stderr(io.StringIO())
        errors.__enter__()
        self.addCleanup(errors.__exit__, None, None, None)

    def backup(self):
        return next((self.root / 'opt/saturn-go').glob('satp-backup.*'))

    def contents(self, version):
        for _, target, mode in installer.FILES:
            path = self.deployer.path(target)
            self.assertEqual(path.read_text(), version + ' ' + path.name)
            self.assertEqual(path.stat().st_mode & 0o777, mode)

    def test_success_installs_all_five_and_retains_backup(self):
        self.deployer.install()
        self.contents('new')
        self.deployer.validate_backup(self.backup())
        self.assertEqual(self.system.events, ['check', 'check', 'stop', 'start'])
        self.assertTrue(self.system.running)

    def test_bad_payload_never_stops_service(self):
        (self.stage / 'saturn-bridge').write_text('corrupt')
        with self.assertRaisesRegex(RuntimeError, 'payload changed'):
            self.deployer.install()
        self.assertNotIn('stop', self.system.events)
        self.contents('old')

    def test_changed_baseline_never_stops_service(self):
        self.deployer.path(installer.FILES[0][1]).write_text('other update')
        with self.assertRaisesRegex(RuntimeError, 'Installed file changed'):
            self.deployer.install()
        self.assertNotIn('stop', self.system.events)

    def test_rx_recheck_failure_never_stops_service(self):
        self.system.fail_preflight_at = 2
        with self.assertRaisesRegex(RuntimeError, 'preflight'):
            self.deployer.install()
        self.assertNotIn('stop', self.system.events)
        self.contents('old')

    def test_startup_failure_restores_entire_matched_set(self):
        self.system.starts_to_fail = 1
        with self.assertRaisesRegex(RuntimeError, 'rollback completed'):
            self.deployer.install()
        self.contents('old')
        self.assertTrue(self.system.running)
        self.assertEqual(self.system.events[-4:], ['stop', 'start', 'stop', 'start'])

    def test_mid_copy_failure_restores_entire_set(self):
        real_replace = self.deployer.replace
        calls = 0
        def fail_once(*args):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise OSError('disk write failed')
            real_replace(*args)
        with patch.object(self.deployer, 'replace', side_effect=fail_once):
            with self.assertRaisesRegex(RuntimeError, 'rollback completed'):
                self.deployer.install()
        self.contents('old')
        self.assertTrue(self.system.running)

    def test_failed_rollback_start_leaves_bridge_stopped(self):
        self.system.starts_to_fail = 2
        with self.assertRaisesRegex(RuntimeError, 'rollback did not complete'):
            self.deployer.install()
        self.contents('old')
        self.assertFalse(self.system.running)

    def test_rollback_write_failure_leaves_bridge_stopped_and_backup_valid(self):
        real_replace = self.deployer.replace
        calls = 0
        def fail_twice(*args):
            nonlocal calls
            calls += 1
            if calls in (3, 5):
                raise OSError('write failed')
            real_replace(*args)
        with patch.object(self.deployer, 'replace', side_effect=fail_twice):
            with self.assertRaisesRegex(RuntimeError, 'rollback did not complete'):
                self.deployer.install()
        self.assertFalse(self.system.running)
        self.deployer.validate_backup(self.backup())

    def test_interrupt_after_stop_restores_old_files(self):
        real_replace = self.deployer.replace
        calls = 0
        def interrupt_once(*args):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise KeyboardInterrupt()
            real_replace(*args)
        with patch.object(self.deployer, 'replace', side_effect=interrupt_once):
            with self.assertRaisesRegex(RuntimeError, 'rollback completed'):
                self.deployer.install()
        self.contents('old')
        self.assertTrue(self.system.running)

    def test_failed_stop_never_replaces_files(self):
        self.system.fail_stop = True
        with self.assertRaisesRegex(RuntimeError, 'rollback did not complete'):
            self.deployer.install()
        self.contents('old')

    def test_manual_rollback_restores_all_files(self):
        self.deployer.install()
        self.deployer.rollback(self.backup())
        self.contents('old')
        self.assertTrue(self.system.running)

    def test_manual_rollback_refuses_unrelated_newer_update(self):
        self.deployer.install()
        target = self.deployer.path(installer.FILES[0][1])
        target.write_text('unrelated later update')
        events = list(self.system.events)
        with self.assertRaisesRegex(RuntimeError, 'outside this deployment'):
            self.deployer.rollback(self.backup())
        self.assertEqual(self.system.events, events)
        self.assertEqual(target.read_text(), 'unrelated later update')

    def test_manual_rollback_refuses_keyed_tx(self):
        self.deployer.install()
        self.system.fail_guard = True
        events = list(self.system.events)
        with self.assertRaisesRegex(RuntimeError, 'keyed'):
            self.deployer.rollback(self.backup())
        self.assertEqual(self.system.events, events)
        self.contents('new')

    def test_corrupt_backup_refuses_rollback_before_stopping(self):
        self.deployer.install()
        (self.backup() / 'old/settings.html').write_text('corrupt')
        events = list(self.system.events)
        with self.assertRaisesRegex(RuntimeError, 'checksum mismatch'):
            self.deployer.rollback(self.backup())
        self.assertEqual(self.system.events, events)

    def test_symlink_target_refused(self):
        path = self.deployer.path(installer.FILES[0][1])
        path.unlink()
        path.symlink_to(self.stage / 'saturn-bridge')
        with self.assertRaisesRegex(RuntimeError, 'non-symlink'):
            self.deployer.install()
        self.assertNotIn('stop', self.system.events)

    def test_readiness_requires_fresh_post_start_unkeyed_xdma(self):
        stamp = time.time() * 1000
        good = {'status': 'ready', 'backend': 'xdma', 'updated_at_ms': stamp,
                'metrics': {'tx_keyed': False, 'tx_stream_active': False}}
        self.assertTrue(installer.rx_ready(good, stamp - 1))
        self.assertFalse(installer.rx_ready(good, stamp + 1))
        for field, value in [('updated_at_ms', stamp - 6000), ('backend', 'p2'), ('status', 'starting')]:
            self.assertFalse(installer.rx_ready(dict(good, **{field: value})))
        for flag in ('tx_keyed', 'tx_stream_active'):
            for value in (True, None, 0):
                state = dict(good, metrics=dict(good['metrics'], **{flag: value}))
                self.assertFalse(installer.rx_ready(state))


if __name__ == '__main__':
    unittest.main()
