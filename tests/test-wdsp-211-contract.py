#!/usr/bin/env python3
"""Run real WDSP 2.11 regressions and optional NNR stage profiling offline."""
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
native = Path(os.environ['SATURN_WDSP_DIR']).resolve()
symbols = subprocess.check_output(['nm', '-g', '--defined-only', str(native / 'libwdsp.a')], text=True)
with tempfile.TemporaryDirectory(prefix='saturn-wdsp211-contract-') as name:
    work = Path(name)
    command = [os.environ.get('CC', 'cc'), '-std=gnu11', '-D_GNU_SOURCE', '-O2',
               '-Wall', '-Wextra', '-Werror', '-I', str(native)]
    if 'report_profile_nnet' in symbols:
        command.append('-DSATURN_TEST_NNR_PROFILE')
    command += [str(root / 'tests/native/test-wdsp-211-contract.c'),
                str(native / 'libwdsp.a'), '-lfftw3', '-lm', '-lpthread',
                '-o', str(work / 'contract')]
    subprocess.run(command, check=True, timeout=30)
    subprocess.run([str(work / 'contract')], cwd=work, check=True, timeout=180)
    profiles = sorted(work.glob('nnr-profile-model-*.txt'))
    if 'SATURN_WDSP_PROFILE_OUTPUT' in os.environ:
        output = Path(os.environ['SATURN_WDSP_PROFILE_OUTPUT'])
        output.mkdir(parents=True, exist_ok=True)
        for profile in profiles:
            (output / profile.name).write_bytes(profile.read_bytes())
    print('NNR stage profile reports:', len(profiles))
