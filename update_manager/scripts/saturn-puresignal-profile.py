#!/usr/bin/env python3
"""Compose a persistent PS service profile from an explicit, verified setup.

This tool writes an inactive profile only. Deployment owns backups, service
handoff and rollback. It never invokes systemd, opens devices or keys RF.
"""
import argparse
import hashlib
import json
from pathlib import Path

RADIO_KEYS = {
    'SATURN_BRIDGE_TX_OWNER', 'SATURN_REMOTE_TX_RF_ENABLED',
    'SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED',
    'SATURN_BRIDGE_DDC0_FREQUENCY_HZ',
}
REQUIRED = {
    'SATURN_PS_HARDWARE_PEAK', 'SATURN_PS_DELAY_SECONDS',
    'SATURN_PS_LOOP_DELAY_SECONDS', 'SATURN_PS_SETTLING_NS',
    'SATURN_PS_FEEDBACK_MAX_AGE_NS', 'SATURN_PS_MIC_MAX_AGE_NS',
    'SATURN_PS_SOURCE_STALL_NS', 'SATURN_PS_TX_DURATION_NS',
    'SATURN_PS_AUTHORITY_TIMEOUT_NS', 'SATURN_PS_CLEANUP_TIMEOUT_NS',
    'SATURN_PS_STARTUP_TIMEOUT_NS', 'SATURN_PS_MAX_WATTS',
    'SATURN_PS_FEEDBACK_ATTENUATION_DB',
}


def compose(environment, state_root):
    if not isinstance(environment, dict):
        raise ValueError('Environment must be a mapping')
    state = Path(state_root)
    if not state.is_absolute() or '..' in state.parts or str(state).startswith('/run/'):
        raise ValueError('State root must be persistent and absolute')
    # satp.conf owns transport and default source settings. EnvironmentFile
    # assignments would override that drop-in even after Settings saved it.
    result = {k: v for k, v in environment.items() if k.startswith('SATURN_PS_') or k in RADIO_KEYS}
    if REQUIRED - result.keys():
        raise ValueError('Missing native/lifecycle settings: ' + ', '.join(sorted(REQUIRED - result.keys())))
    if result.get('SATURN_BRIDGE_TX_OWNER') != 'shared':
        raise ValueError('Explicit shared owner required')
    if result.get('SATURN_PS_MEASUREMENT_ONLY', '0') != '0' or result.get('SATURN_PS_GENERATED_TRIALS_ONLY', '0') != '0':
        raise ValueError('A measurement-only host is not a daily-driver profile')
    result.update({
        'SATURN_BRIDGE_STATE_DIR': str(state / 'receive-state'),
        'SATURN_BRIDGE_FFTW_WISDOM_PATH': str(state / 'wdspWisdom01'),
        'SATURN_TX_DIAGNOSTIC_DIRECTORY': str(state / 'digital'),
        'SATURN_RF_MONITOR_CAPTURE_DIRECTORY': str(state / 'pairs'),
        'SATURN_PS_MEASUREMENT_DIR': str(state / 'captures'),
    })
    lines = ['# Explicit persistent G2 PureSignal configuration; no key request.']
    for key, value in sorted(result.items()):
        if not isinstance(value, str) or any(c in value for c in '\r\n\0"\\%'):
            raise ValueError('Unsafe environment value: ' + key)
        if value.startswith('/run/'):
            raise ValueError('Runtime path is not persistent: ' + key)
        lines.append(key + '="' + value + '"')
    profile = '\n'.join(lines) + '\n'
    return {
        'puresignal.env': profile,
        'saturn-bridge.conf': '[Service]\nEnvironmentFile=/etc/default/saturn-puresignal\n',
        'p2app.conf': '[Service]\nEnvironment=SATURN_P3_RT_AUDIO_ENABLE=1\nEnvironment=SATURN_P3_RT_AUDIO_POLICY=rr\nEnvironment=SATURN_P3_RT_AUDIO_PRIORITY=10\nEnvironment=SATURN_P3_RT_AUDIO_CPUS=2\n',
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--environment-json', type=Path, required=True)
    p.add_argument('--state-root', required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    files = compose(json.loads(a.environment_json.read_text()), a.state_root)
    a.output.mkdir(parents=True, exist_ok=False)
    for name, value in files.items():
        (a.output / name).write_text(value)
    record = {'format': 'saturn-persistent-puresignal-profile-v1',
              'sha256': {k: hashlib.sha256(v.encode()).hexdigest() for k, v in files.items()},
              'backendSelectionFrozen': False, 'pttCommandsSent': 0,
              'calibrationCoefficientsPersisted': False}
    (a.output / 'profile.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record))


if __name__ == '__main__':
    main()
