#!/usr/bin/env python3
"""Run the deterministic native TX-worker lifetime test without radio access."""
import os
import subprocess
import tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='saturn-wdsp-shutdown-') as name:
    binary = Path(name) / 'shutdown-test'
    subprocess.run([
        os.environ.get('CC', 'cc'), '-std=gnu11', '-D_GNU_SOURCE',
        '-Wall', '-Wextra', '-Werror', '-pthread',
        str(root / 'tests/native/test-wdsp-channel-shutdown.c'), '-o', str(binary),
    ], check=True, timeout=30)
    subprocess.run([str(binary)], check=True, timeout=10)
