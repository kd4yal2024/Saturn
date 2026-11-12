#!/usr/bin/env python3
"""Explicit, data-only migration of a reviewed saved PA table; never keys RF."""
import argparse
import json
import math
import os
from pathlib import Path
import tempfile


def profile(value):
    if not isinstance(value, list) or len(value) != 12 or not all(
        type(v) in (int, float) and math.isfinite(v) and 40 <= v <= 80 for v in value
    ):
        raise ValueError('expected twelve finite PA gains in [40,80]')
    return value


def migrate(settings, directory, expected):
    target = directory / 'pa-calibration-v1.txt'
    if os.path.lexists(target):
        return 'existing host preset retained'
    saved = profile(json.loads(settings.read_text())['radioPrefs']['txPaCalibration'])
    reviewed = profile([float(v) for v in expected.split('|')])
    if saved != reviewed:
        raise ValueError('saved PA table changed since review; nothing was migrated')
    text = 'format=saturn-pa-calibration\nversion=1\nprofile=' + '|'.join(str(v) for v in saved) + '\n'
    directory.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.pa-migration-', dir=directory)
    try:
        with os.fdopen(fd, 'w') as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        # Atomic creation, without replacing an existing file or symlink.
        os.link(temporary, target)
        parent = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        os.unlink(temporary)
    return 'reviewed PA table migrated; TX authority unchanged'


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--from-settings', type=Path, required=True)
    parser.add_argument('--state-dir', type=Path, required=True)
    parser.add_argument('--expected-profile', required=True)
    args = parser.parse_args()
    print(migrate(args.from_settings, args.state_dir, args.expected_profile))
