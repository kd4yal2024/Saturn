"""Default is static verification only. No service switching or automatic restart.

--identity executes the verified ARM identity query, but opens no radio devices.
--run-no-rf is for a SEPARATELY AUTHORIZED exclusive G2 hardware window only.
The launcher execs the candidate: no supervisor detaches a child on timeout.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import socket
import subprocess

HERE = Path(__file__).resolve().parent
SOURCE_PIN = '84387da737de7dca6e332739e0dc42b30f4ef5baf1a3db68505326faae14d800'
BINARY_SHA = 'c462e688fc81842c152a48d8bcebe566f517a461eb48e898d40addb549b19292'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_candidate(binary, lock):
    if lock['reviewedSourcePin'] != SOURCE_PIN or lock['reviewedBinarySha256'] != BINARY_SHA:
        raise RuntimeError('accepted candidate lock mismatch')
    if digest(binary) != BINARY_SHA:
        raise RuntimeError('candidate binary hash mismatch')
    data = binary.read_bytes()
    if data[:4] != b'\x7fELF' or int.from_bytes(data[18:20], 'little') != 183 or SOURCE_PIN.encode() not in data:
        raise RuntimeError('not the pinned AArch64 candidate')


def no_rf_environment(run):
    # Allowlist, not a copy of the ambient shell/service environment. All values
    # below are startup-only placeholders; correction remains disabled, strict,
    # and unable to key. They are NOT measured feedback calibration defaults.
    return {
        'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C.UTF-8',
        'SATURN_BRIDGE_RADIO_BACKEND': 'xdma', 'SATURN_BRIDGE_TX_OWNER': 'shared',
        'SATURN_REMOTE_TX_RF_ENABLED': '0', 'SATURN_PS_MANAGED_RF_PERMITTED': '0',
        'SATURN_REMOTE_TWOTONE_RF_DISABLED_OK': '0', 'SATURN_PS_ADMISSION': 'strict',
        'SATURN_BRIDGE_TX_AUDIO_SOURCE': 'tci', 'SATURN_BRIDGE_SATP_ENABLED': '0',
        'SATURN_BRIDGE_TX_OPUS_DECODE_ENABLED': '0',
        'SATURN_BRIDGE_TCI_HOST': '127.0.0.1', 'SATURN_BRIDGE_TCI_PORT': '50001',
        'SATURN_PS_FEEDBACK_TOPOLOGY_REVIEWED': '1',
        'SATURN_PS_HARDWARE_PEAK': '1', 'SATURN_PS_DELAY_SECONDS': '0',
        'SATURN_PS_LOOP_DELAY_SECONDS': '0', 'SATURN_PS_SETTLING_NS': '0',
        'SATURN_PS_FEEDBACK_MAX_AGE_NS': '1000000000', 'SATURN_PS_MIC_MAX_AGE_NS': '1000000000',
        'SATURN_PS_SOURCE_STALL_NS': '1000000000', 'SATURN_PS_TX_DURATION_NS': '1000000000',
        'SATURN_PS_AUTHORITY_TIMEOUT_NS': '5000000000', 'SATURN_PS_CLEANUP_TIMEOUT_NS': '5000000000',
        'SATURN_PS_STARTUP_TIMEOUT_NS': '300000000000', 'SATURN_PS_MAX_WATTS': '1',
        'SATURN_PS_FEEDBACK_ATTENUATION_DB': '31',
        'SATURN_BRIDGE_XDMA_READY_PATH': str(run / 'ready.json'),
        'SATURN_BRIDGE_PERF_PATH': str(run / 'perf.json'),
        'SATURN_BRIDGE_FFTW_WISDOM_PATH': str(run / 'wdspWisdom01'),
    }


def validate_identity(record, lock):
    expected = {'format': 'saturn-native-build-info-v1', 'target': 'aarch64',
                'sourcePin': SOURCE_PIN, 'wdspVersion': lock['wdspVersion'],
                'wdspArchiveSha256': lock['nativeArchiveSha256'],
                'wdspHeaderSha256': lock['nativeHeaderSha256'], 'stubNative': False,
                'defaultTxOwner': 'legacy', 'productionManagedWiring': 'opt-in-review',
                'rfQualified': False}
    if any(record.get(k) != v for k, v in expected.items()):
        raise RuntimeError('ARM native identity mismatch')


def require_idle():
    safe_env = {'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C.UTF-8'}
    for unit in ['saturn-bridge.service', 'p2app.service']:
        state = subprocess.run(['systemctl', 'show', '-p', 'ActiveState', '--value', unit],
                               capture_output=True, text=True, check=True, env=safe_env).stdout.strip()
        if state != 'inactive':
            raise RuntimeError(f'{unit} must be inactive, not {state}')
    devices = ['/dev/xdma0_user', '/dev/xdma0_c2h_0', '/dev/xdma0_h2c_0']
    if not all(Path(p).exists() for p in devices):
        raise RuntimeError('expected XDMA devices unavailable')
    users = subprocess.run(['fuser', *devices], capture_output=True, text=True, env=safe_env)
    if users.returncode != 1 or users.stdout.strip() or users.stderr.strip():
        raise RuntimeError('XDMA ownership is occupied or could not be verified: ' + users.stdout + users.stderr)
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 50001))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--identity', action='store_true')
    mode.add_argument('--run-no-rf', action='store_true')
    parser.add_argument('--confirm-hardware-window', action='store_true')
    parser.add_argument('--ack-reviewed-source-layout', action='store_true')
    parser.add_argument('--run-dir', type=Path)
    args = parser.parse_args()
    lock = json.loads((HERE / 'candidate-lock.json').read_text())
    binary = (HERE / 'arm-candidate/saturn-bridge').resolve()
    verify_candidate(binary, lock)
    if not args.identity and not args.run_no_rf:
        print(json.dumps({'staticCheck': 'passed', 'sourcePin': SOURCE_PIN, 'binarySha256': BINARY_SHA,
                          'armExecuted': False, 'radioDevicesOpened': False}))
        return
    if platform.machine() not in ['aarch64', 'arm64']:
        raise RuntimeError('ARM execution must occur on the approved ARM host, not a local emulator')
    if not args.confirm_hardware_window:
        parser.error('separate hardware authorization required; add --confirm-hardware-window only in that window')
    if not os.access(binary, os.X_OK):
        raise RuntimeError('verified binary is not executable; set its executable permission in the approved window')
    env = {'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C.UTF-8'}
    identity = json.loads(subprocess.check_output([str(binary), '--native-build-info'], env=env, text=True, timeout=10))
    validate_identity(identity, lock)
    if args.identity:
        print(json.dumps(identity, indent=2))
        return
    if not args.ack_reviewed_source_layout or args.run_dir is None or not args.run_dir.is_absolute():
        parser.error('explicit source-layout acknowledgment and absolute NEW --run-dir required')
    if os.geteuid() != 0:
        raise RuntimeError('exclusive hardware launch requires root for ownership checks and real-time limits')
    memlock = resource.getrlimit(resource.RLIMIT_MEMLOCK)[0]
    rtprio = resource.getrlimit(resource.RLIMIT_RTPRIO)[0]
    if (memlock != resource.RLIM_INFINITY and memlock < 16777216) or (rtprio != resource.RLIM_INFINITY and rtprio < 20):
        raise RuntimeError('use approved prlimit: MEMLOCK >=16MiB and RTPRIO >=20; launcher does not change limits')
    maintenance = Path('/run/lock/saturn-maintenance/radio.lock')
    with maintenance.open('a') as lock_file:
        fcntl.flock(lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require_idle()
        run = args.run_dir.resolve()
        run.mkdir(parents=True, exist_ok=False)
        environment = no_rf_environment(run)
        (run / 'launch.json').write_text(json.dumps({'nativeIdentity': identity, 'environment': environment,
            'hardwareWindow': 'no-RF only', 'topologyAttestation': 'reviewed source layout; hardware feedback integrity unqualified',
            'calibrationValues': 'inactive startup placeholders, not RF thresholds'}, indent=2) + '\n')
        # The appliance maintenance lock survives exec and stays held until the
        # actual candidate process exits. No restart/rollback races this owner.
        os.set_inheritable(lock_file.fileno(), True)
        log = os.open(run / 'bridge.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.dup2(log, 1); os.dup2(log, 2); os.close(log)
        os.chdir(run)
        os.execve(binary, [str(binary)], environment)


if __name__ == '__main__':
    main()
