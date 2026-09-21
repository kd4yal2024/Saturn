#!/usr/bin/env python3
"""Matched SATP bridge/UI deployment. CLI has no test/root override."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

FILES = (
    ('saturn-bridge', '/opt/saturn-go/bin/saturn-bridge', 0o755),
    ('update_manager/templates/saturn-remote-next.html', '/var/lib/saturn-web/saturn-remote-next.html', 0o644),
    ('update_manager/remote-web/dist/saturn-remote-next.js', '/var/lib/saturn-web/saturn-remote-next.js', 0o644),
    ('update_manager/remote-web/dist/saturn-remote-next.js.sha256', '/var/lib/saturn-web/saturn-remote-next.js.sha256', 0o644),
    ('update_manager/templates/settings.html', '/var/lib/saturn-web/settings.html', 0o644),
)
SERVICE = 'saturn-bridge.service'


def digest(path):
    with open(path, 'rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def regular(path):
    require(stat.S_ISREG(path.lstat().st_mode), f'Not a regular, non-symlink file: {path}')


def read_manifest(path):
    result = {}
    for line in path.read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        require(match is not None, f'Invalid checksum manifest: {path}')
        value, name = match.groups()
        require(name not in result, f'Duplicate checksum entry: {name}')
        result[name] = value
    return result


def rx_ready(state, since_ms=0):
    stamp = state.get('updated_at_ms', 0)
    metrics = state.get('metrics', {})
    return (state.get('backend') == 'xdma' and state.get('status') == 'ready'
            and since_ms <= stamp and 0 <= time.time() * 1000 - stamp < 5000
            and metrics.get('tx_keyed') is False
            and metrics.get('tx_stream_active') is False)


class System:
    def run(self, *args):
        return subprocess.run(args, check=True, text=True, capture_output=True, timeout=30).stdout.strip()

    def state(self):
        return self.run('systemctl', 'show', SERVICE, '-p', 'ActiveState', '--value')

    def no_p2(self):
        checks = [(('systemctl', 'is-active', '--quiet', 'p2app.service'), (3, 4)),
                  (('pgrep', '-x', 'p2app'), (1,))]
        for args, absent_codes in checks:
            result = subprocess.run(args, capture_output=True, timeout=10)
            require(result.returncode != 0, 'P2/Thetis is active. Select XDMA/TCI before deployment.')
            require(result.returncode in absent_codes, 'Could not verify P2/Thetis ownership.')

    def readiness(self):
        return json.loads(Path('/run/saturn-bridge/xdma-ready.json').read_text())

    def preflight(self, stage):
        subprocess.run(['bash', str(stage / 'preflight.sh'), '--check'], check=True, timeout=30)

    def stop(self):
        self.no_p2()
        self.run('systemctl', 'stop', SERVICE)
        require(self.state() in ('inactive', 'failed'), 'Bridge did not stop; refusing file replacement.')
        require(self.run('systemctl', 'show', SERVICE, '-p', 'MainPID', '--value') == '0', 'Bridge process is still running.')

    def start_ready(self, installed):
        self.no_p2()
        since_ms = int(time.time() * 1000)
        self.run('systemctl', 'start', SERVICE)
        deadline = time.monotonic() + 60
        consecutive = 0
        while time.monotonic() < deadline:
            try:
                pid = self.run('systemctl', 'show', SERVICE, '-p', 'MainPID', '--value')
                good = (self.state() == 'active' and rx_ready(self.readiness(), since_ms)
                        and pid.isdecimal() and int(pid) > 0
                        and digest(Path('/proc') / pid / 'exe') == digest(installed))
            except (OSError, ValueError, subprocess.SubprocessError):
                good = False
            consecutive = consecutive + 1 if good else 0
            if consecutive >= 3:
                return
            time.sleep(1)
        raise RuntimeError('Bridge failed fresh RX readiness/running-binary verification within 60 seconds.')

    def guard_rollback(self):
        self.no_p2()
        current = self.state()
        if current == 'active':
            require(rx_ready(self.readiness()), 'Release PTT/MOX; rollback requires fresh RX readiness.')
        else:
            require(current in ('inactive', 'failed'), f'Bridge is {current}; retry when stable.')
            require(self.run('systemctl', 'show', SERVICE, '-p', 'MainPID', '--value') == '0', 'Bridge is still running.')


class Deployer:
    def __init__(self, stage, system, root=Path('/')):
        # root/system injection is for unit tests only, never exposed by the CLI.
        self.stage, self.system, self.root = stage, system, root
        self.backup_parent = self.path('/opt/saturn-go')

    def path(self, absolute):
        return self.root / absolute.lstrip('/')

    def verify_baseline(self):
        baseline = read_manifest(self.stage / 'installed-before.sha256')
        require(set(baseline) == {target for _, target, _ in FILES}, 'Unexpected installed-baseline targets.')
        for _, target, _ in FILES:
            destination = self.path(target)
            regular(destination)
            require(not destination.parent.is_symlink(), f'Symlink target directory: {destination.parent}')
            require(digest(destination) == baseline[target], f'Installed file changed: {target}')
        return baseline

    def snapshot(self):
        baseline = self.verify_baseline()
        payload = read_manifest(self.stage / 'SHA256SUMS')
        backup = Path(tempfile.mkdtemp(prefix='satp-backup.', dir=self.backup_parent))
        entries = {}
        for folder in ('old', 'new'):
            (backup / folder).mkdir(mode=0o700)
        for source, target, _ in FILES:
            name = Path(target).name
            installed = self.path(target)
            metadata = installed.stat()
            regular(self.stage / source)
            shutil.copyfile(installed, backup / 'old' / name)
            shutil.copyfile(self.stage / source, backup / 'new' / name)
            require(digest(backup / 'old' / name) == baseline[target], f'Backup changed while copying: {target}')
            require(digest(backup / 'new' / name) == payload[source], f'Staged payload changed while copying: {source}')
            entries[name] = {'old': baseline[target], 'new': payload[source],
                             'uid': metadata.st_uid, 'gid': metadata.st_gid,
                             'mode': stat.S_IMODE(metadata.st_mode)}
        record = {'schema': 1, 'files': entries}
        with open(backup / 'rollback.json', 'w') as stream:
            json.dump(record, stream, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.sync()
        print(f'Rollback backup: {backup}', flush=True)
        return backup

    def validate_backup(self, backup):
        require(backup.parent == self.backup_parent and
                re.fullmatch(r'satp-backup\.[A-Za-z0-9_-]+', backup.name) is not None,
                'Backup must be a satp-backup.* directory under /opt/saturn-go.')
        require(not backup.is_symlink() and backup.is_dir(), 'Invalid backup directory.')
        require(backup.stat().st_uid == os.geteuid() and stat.S_IMODE(backup.stat().st_mode) == 0o700,
                'Backup must be owned by the current user (root for deployment) and mode 0700.')
        regular(backup / 'rollback.json')
        record = json.loads((backup / 'rollback.json').read_text())
        require(record['schema'] == 1 and set(record['files']) == {Path(t).name for _, t, _ in FILES}, 'Invalid backup schema/files.')
        for name, item in record['files'].items():
            for folder, key in (('old', 'old'), ('new', 'new')):
                require(not (backup / folder).is_symlink(), 'Symlink backup subdirectory.')
                regular(backup / folder / name)
                require(digest(backup / folder / name) == item[key], f'Backup checksum mismatch: {folder}/{name}')
            require(all(type(item[k]) is int and item[k] >= 0 for k in ('uid', 'gid', 'mode'))
                    and item['mode'] <= 0o777, 'Invalid backup permissions.')
        return record['files']

    def replace(self, source, destination, mode, uid, gid):
        regular(destination)
        require(not destination.parent.is_symlink(), 'Symlink destination directory.')
        fd, tmp = tempfile.mkstemp(prefix='.satp-install-', dir=destination.parent)
        try:
            with os.fdopen(fd, 'wb') as stream, open(source, 'rb') as origin:
                shutil.copyfileobj(origin, stream)
                os.fchmod(stream.fileno(), mode)
                os.fchown(stream.fileno(), uid, gid)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(tmp, destination)
            directory_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)

    def verify_targets(self, entries, which):
        for _, target, _ in FILES:
            require(digest(self.path(target)) == entries[Path(target).name][which], f'Installed checksum mismatch: {target}')

    def restore(self, backup):
        entries = self.validate_backup(backup)
        self.system.stop()  # If stopping fails, do not replace anything.
        try:
            for _, target, _ in FILES:
                name = Path(target).name
                entry = entries[name]
                self.replace(backup / 'old' / name, self.path(target), entry['mode'], entry['uid'], entry['gid'])
            self.verify_targets(entries, 'old')
            self.system.start_ready(self.path(FILES[0][1]))
        except BaseException:
            # Never leave a partially restored or unverified bridge running.
            self.system.stop()
            raise
        print(f'Previous bridge and all web files restored from {backup}', flush=True)

    def install(self):
        self.system.preflight(self.stage)
        backup = self.snapshot()
        entries = self.validate_backup(backup)
        self.system.preflight(self.stage)  # RX/baseline recheck immediately before stop.
        self.verify_baseline()
        try:
            self.system.stop()
            for _, target, mode in FILES:
                name = Path(target).name
                self.replace(backup / 'new' / name, self.path(target), mode, 0, 0)
            self.verify_targets(entries, 'new')
            self.system.start_ready(self.path(FILES[0][1]))
        except BaseException as error:
            print(f'Installation failed: {error}. Restoring the matched backup.', file=sys.stderr, flush=True)
            # Ignore a second terminal signal while recovering. SIGKILL/power
            # loss cannot be trapped; the private backup supports manual recovery.
            with recovery_signals():
                try:
                    self.restore(backup)
                except BaseException as recovery_error:
                    raise RuntimeError(f'Automatic rollback did not complete: {recovery_error}. '
                                       f'Do not transmit; recovery backup: {backup}') from recovery_error
            raise RuntimeError(f'Installation failed; rollback completed. Backup: {backup}') from error
        print('Installed successfully: bridge + Remote HTML/JS/checksum + Settings HTML.', flush=True)
        print('Fresh RX readiness verified. No SATP/radio settings changed; no TX test performed.', flush=True)
        print(f'Manual rollback: sudo bash {self.stage}/deploy.sh --rollback {backup}', flush=True)

    def rollback(self, backup):
        entries = self.validate_backup(backup)
        self.system.guard_rollback()
        for _, target, _ in FILES:
            entry = entries[Path(target).name]
            require(digest(self.path(target)) in (entry['old'], entry['new']),
                    f'{target} changed outside this deployment; refusing to overwrite it.')
        with recovery_signals():
            self.restore(backup)


@contextlib.contextmanager
def recovery_signals():
    signals = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
    old = {sig: signal.signal(sig, signal.SIG_IGN) for sig in signals}
    try:
        yield
    finally:
        for sig, handler in old.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument('--check', action='store_true')
    action.add_argument('--install', action='store_true')
    action.add_argument('--rollback', type=Path, metavar='BACKUP')
    args = parser.parse_args()
    stage = Path(__file__).resolve().parent
    system = System()
    deployer = Deployer(stage, system)
    if args.check:
        system.preflight(stage)
        deployer.verify_baseline()
        print('Installer preflight passed. Nothing installed or restarted.')
        return
    require(os.geteuid() == 0, 'Run --install/--rollback with sudo; --check needs no sudo.')
    with open('/run/lock/saturn-satp-deploy.lock', 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        def interrupted(signum, frame):
            raise RuntimeError(f'Interrupted by signal {signum}')
        for sig in (signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, interrupted)
        if args.install:
            deployer.install()
        else:
            deployer.rollback(args.rollback)


if __name__ == '__main__':
    try:
        main()
    except (Exception, KeyboardInterrupt) as error:
        print(f'Deployment stopped: {error}', file=sys.stderr)
        sys.exit(1)
