"""Rebuild the accepted, immutable source with frozen external build inputs.

No network, host service, device access or radio. Every output directory is new.
The accepted source index is not regenerated or modified by this preparation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
LOCK = json.loads((HERE / 'candidate-lock.json').read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree(path):
    result = {}
    for p in sorted(path.rglob('*')):
        name = str(p.relative_to(path))
        if p.is_symlink():
            result[name] = {'link': os.readlink(p)}
        elif p.is_file():
            result[name] = {'sha256': sha(p)}
    return result


def verify_review(review):
    if sha(review / 'SOURCE_SHA256SUMS') != LOCK['reviewedSourcePin']:
        raise RuntimeError('not the accepted source index')
    record = json.loads((review / 'review.json').read_text())
    if record['sourcePinSha256'] != LOCK['reviewedSourcePin'] or record['armBinarySha256'] != LOCK['reviewedBinarySha256']:
        raise RuntimeError('review identity mismatch')
    for index, root in [(review / 'SHA256SUMS', review),
                        (review / 'SOURCE_SHA256SUMS', review / 'source')]:
        for line in index.read_text().splitlines():
            digest, name = line.split('  ', 1)
            p = root / name
            if p.resolve().is_relative_to(root.resolve()) is False or sha(p) != digest:
                raise RuntimeError(f'checksum failure: {name}')
    if sha(review / 'native/libwdsp.a') != LOCK['nativeArchiveSha256']:
        raise RuntimeError('native archive mismatch')


def inputs(sysroot):
    toolchain = Path(subprocess.check_output(['rustup', 'which', '--toolchain', LOCK['rustToolchain'], 'rustc'], text=True).strip()).parent.parent
    lld = toolchain / 'lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld'
    tools = {'rustc': toolchain / 'bin/rustc', 'cargo': toolchain / 'bin/cargo',
             'clang': Path('/usr/bin/clang'), 'ar': Path('/usr/bin/ar'), 'rust-lld': lld}
    return {'format': 'saturn-no-rf-external-build-inputs-v1',
            'sourcePin': LOCK['reviewedSourcePin'], 'sysroot': tree(sysroot),
            'tools': {name: sha(path) for name, path in tools.items()},
            'rustLibraries': tree(toolchain / 'lib'),
            'rustcVersion': subprocess.check_output([str(tools['rustc']), '-vV'], text=True),
            'clangVersion': subprocess.check_output([str(tools['clang']), '--version'], text=True)}, toolchain, lld


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--review-root', type=Path, required=True)
    parser.add_argument('--sysroot', type=Path, required=True)
    parser.add_argument('--input-lock', type=Path, required=True)
    parser.add_argument('--record-inputs', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    review, sysroot = args.review_root.resolve(), args.sysroot.resolve()
    verify_review(review)
    recorded, toolchain, lld = inputs(sysroot)
    if args.record_inputs:
        with args.input_lock.open('x') as target:
            json.dump(recorded, target, indent=2, sort_keys=True)
            target.write('\n')
        print('external build inputs frozen; accepted source unchanged')
        return
    if recorded != json.loads(args.input_lock.read_text()):
        raise RuntimeError('external build inputs differ from the frozen lock')
    if args.output is None:
        parser.error('--output required for rebuild')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / 'source'
    shutil.copytree(review / 'source', source)
    bridge = source / 'update_manager/saturn-bridge'
    env = {key: os.environ[key] for key in ['HOME', 'CARGO_HOME', 'RUSTUP_HOME'] if key in os.environ}
    env.update({
        'PATH': str(toolchain / 'bin') + ':/usr/bin:/bin', 'LANG': 'C.UTF-8',
        'RUSTUP_TOOLCHAIN': LOCK['rustToolchain'], 'CARGO_NET_OFFLINE': 'true',
        'CARGO_TARGET_DIR': str(output / 'target'),
        'SATURN_ARM_SYSROOT': str(sysroot), 'SATURN_ARM_LLD': str(lld),
        'CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER': str(bridge / 'scripts/link-reviewed-arm.sh'),
        'CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS': '-C target-cpu=cortex-a72',
        'SATURN_WDSP_DIR': str(review / 'native'),
        'SATURN_BRIDGE_WDSP_FLAVOR': 'wdsp2-2.10',
        'SATURN_BRIDGE_WDSP_COMMIT': 'b02d5bac675dd2f33ec2bab2b339f79a597c47dd',
        'SATURN_BRIDGE_TARGET_CPU': LOCK['cpu'], 'SATURN_BUILD_COMMIT': LOCK['gitBase'],
        'SATURN_BUILD_DIRTY': 'true', 'SATURN_BRIDGE_SOURCE_PIN': LOCK['reviewedSourcePin'],
    })
    command = [str(toolchain / 'bin/cargo'), 'build', '--offline', '--locked', '--release',
               '--target', LOCK['target'], '--features', 'managed-puresignal']
    with (output / 'build-command.json').open('x') as log:
        json.dump({'command': command, 'cwd': str(bridge),
                   'buildEnvironment': {k: v for k, v in env.items() if k not in ['HOME', 'CARGO_HOME', 'RUSTUP_HOME']},
                   'inputLockSha256': sha(args.input_lock)}, log, indent=2)
    with (output / 'build.log').open('x') as log:
        subprocess.run(command, cwd=bridge, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    binary = output / 'target' / LOCK['target'] / 'release/saturn-bridge'
    result = {'sourcePin': LOCK['reviewedSourcePin'], 'binarySha256': sha(binary),
              'matchesReviewedBinary': sha(binary) == LOCK['reviewedBinarySha256'],
              'armExecuted': False}
    (output / 'rebuild-result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
    if not result['matchesReviewedBinary']:
        raise RuntimeError('rebuild differs; retain output and investigate, never relabel it accepted')


if __name__ == '__main__':
    main()
