"""Rebuild frozen reviewed WDSP sources for the explicitly supplied ARM sysroot.

Writes a NEW output directory only. No package install, service, device, radio
or remote command. The sysroot is input, not downloaded or modified here.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--review-root', type=Path, required=True)
    parser.add_argument('--sysroot', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--compiler', default='/usr/bin/clang')
    args = parser.parse_args()
    review, sysroot, output = (p.resolve() for p in [args.review_root, args.sysroot, args.output])
    source = review / 'offline-native/wdsp-build'
    manifest = dict((name, value) for value, name in
                    (line.split('  ', 1) for line in (review / 'SHA256SUMS').read_text().splitlines()))
    inputs = sorted(p for p in source.iterdir() if p.suffix in ['.c', '.h'])
    assert inputs and digest(source / 'libwdsp.a') == '8b75784e63e1c3c9baba993dc866db0002cee0037d42b0d80b88c059d58bb248'
    for path in inputs:
        assert digest(path) == manifest[str(path.relative_to(review))], path
    assert (sysroot / 'usr/include/fftw3.h').is_file()
    output.mkdir(parents=True, exist_ok=False)
    for path in inputs:
        shutil.copy2(path, output / path.name)
    flags = ['--target=aarch64-linux-gnu', f'--sysroot={sysroot}',
             '-isystem', str(sysroot / 'usr/include/aarch64-linux-gnu'),
             '-pthread', '-O3', '-mcpu=cortex-a72', '-D_GNU_SOURCE', '-Wno-parentheses']
    commands = []
    def compile_one(path):
        command = [args.compiler, *flags, '-I', str(output), '-c', str(output / path.name),
                   '-o', str(output / (path.stem + '.o'))]
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if result.returncode:
            raise RuntimeError(path.name + '\n' + result.stdout)
        return command
    with ThreadPoolExecutor(max_workers=2) as pool:
        commands.extend(pool.map(compile_one, [p for p in inputs if p.suffix == '.c']))
    objects = sorted(output.glob('*.o'))
    subprocess.run(['ar', 'rcs', str(output / 'libwdsp.a'), *(str(p) for p in objects)], check=True)
    record = {'format': 'saturn-reviewed-arm-native-v1',
              'reviewedArchiveSha256': 'c6503f40a278973a4116a272946390ad7f9b257f92f4b18b20936963081a4f48',
              'upstreamWdspCommit': 'b02d5bac675dd2f33ec2bab2b339f79a597c47dd',
              'target': 'aarch64-unknown-linux-gnu', 'cpu': 'cortex-a72',
              'compiler': subprocess.check_output([args.compiler, '--version'], text=True),
              'sourceFiles': {p.name: digest(p) for p in inputs},
              'fftwHeaderSha256': digest(sysroot / 'usr/include/fftw3.h'),
              'archiveSha256': digest(output / 'libwdsp.a'), 'commands': commands,
              'rfQualified': False, 'installedOnG2': False}
    (output / 'native-build.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({k: record[k] for k in ['format', 'target', 'cpu', 'archiveSha256', 'rfQualified']}))


if __name__ == '__main__':
    main()
