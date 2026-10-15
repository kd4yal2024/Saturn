"""Pin a source snapshot, then assemble a NON-INSTALLABLE ARM review bundle.

No network, service, radio or device operations. Existing output snapshots are
never overwritten. Source drift between pin and assembly is a hard failure.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import zipfile

BASE = 'cc09c61f91030d4c5a38d6c570a20c28b5d97e03'
NATIVE_SHA = 'f5f21c7cb76df1add17647c15a700f564a7f045e930e84be870333fa6e74973e'
REPO = Path(__file__).resolve().parents[3]
ROOTS = ['update_manager/saturn-bridge', 'update_manager/remote-web',
         'update_manager/docs/puresignal', 'update_manager/templates/saturn-remote-next.html']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_files():
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others',
                                     '--exclude-standard', '--', *ROOTS], cwd=REPO)
    return sorted(set(Path(n.decode()) for n in names.split(b'\0') if n))


def source_index():
    return ''.join(f'{sha(REPO / name)}  {name.as_posix()}\n'
                   for name in source_files() if (REPO / name).is_file())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--pin-source', action='store_true')
    parser.add_argument('--assemble', action='store_true')
    parser.add_argument('--native', type=Path)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--evidence', type=Path, action='append', default=[])
    parser.add_argument('--wiring-review', action='store_true', help='new default-off production-wiring candidate, not the historical base bundle')
    parser.add_argument('--web-bundle', type=Path)
    parser.add_argument('--runtime-review', action='store_true', help='actual shared_backend run_inner orchestration candidate')
    args = parser.parse_args()
    if args.runtime_review and not args.wiring_review:
        parser.error('--runtime-review requires --wiring-review')
    if args.pin_source == args.assemble:
        parser.error('select exactly one of --pin-source and --assemble')
    if subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip() != BASE:
        raise RuntimeError('unexpected reconciliation base')
    output = args.output.resolve()
    index = source_index()
    pin = hashlib.sha256(index.encode()).hexdigest()
    if args.pin_source:
        if output.exists() and any(output.iterdir()):
            raise RuntimeError('pin output must be new or empty')
        output.mkdir(parents=True, exist_ok=True)
        for name in source_files():
            if not (REPO / name).is_file():
                continue
            target = output / 'source' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(REPO / name, target)
        (output / 'SOURCE_SHA256SUMS').write_text(index)
        print(pin)
        return

    if (output / 'SOURCE_SHA256SUMS').read_text() != index:
        raise RuntimeError('source changed since pin; make a fresh snapshot and rebuild')
    if not args.native or not args.binary:
        parser.error('--assemble requires --native and --binary')
    if sha(args.native / 'libwdsp.a') != NATIVE_SHA:
        raise RuntimeError('unexpected ARM native archive')
    if pin.encode() not in args.binary.read_bytes():
        raise RuntimeError('ARM binary does not contain this source pin')
    elf = subprocess.check_output(['readelf', '-h', '-l', '-d', '-V', str(args.binary)], text=True)
    if 'AArch64' not in elf:
        raise RuntimeError('candidate must be an AArch64 ELF; tests remain local x86 evidence')
    if args.wiring_review:
        if b'opt-in-review' not in args.binary.read_bytes():
            raise RuntimeError('managed-puresignal wiring marker missing from candidate')
        if not args.web_bundle or not args.web_bundle.is_file():
            parser.error('--wiring-review assembly requires a built --web-bundle')
        abi_logs = [p for p in args.evidence if 'arm-abi-command-status' in p.name]
        if len(abi_logs) != 1 or 'exit_status=0' not in abi_logs[0].read_text():
            raise RuntimeError('a nonempty successful command/status ABI transcript is required')
    binary_dir = output / 'arm-candidate'
    binary_dir.mkdir()
    shutil.copy2(args.binary, binary_dir / 'saturn-bridge')
    if args.wiring_review:
        web_dir=output / 'web-candidate'
        web_dir.mkdir()
        shutil.copy2(args.web_bundle, web_dir / args.web_bundle.name)
    native_dir = output / 'native'
    native_dir.mkdir()
    for path in sorted(args.native.iterdir()):
        if path.suffix in ['.c', '.h'] or path.name in ['libwdsp.a', 'native-build.json']:
            shutil.copy2(path, native_dir / path.name)
    evidence_dir = output / 'evidence'
    evidence_dir.mkdir()
    evidence = {}
    for path in args.evidence:
        if (evidence_dir / path.name).exists():
            raise RuntimeError('duplicate evidence filename')
        shutil.copy2(path, evidence_dir / path.name)
        evidence[path.name] = sha(path)
    (evidence_dir / 'arm-elf.txt').write_text(elf)
    patch = subprocess.check_output(['git', 'diff', '--binary', BASE, '--', *ROOTS], cwd=REPO)
    (output / 'tracked-reconciliation.patch').write_bytes(patch)
    record = {'format': 'saturn-puresignal-production-base-review-v1',
              'gitBase': BASE, 'sourcePinSha256': pin, 'uncommittedCandidate': True,
              'armBinarySha256': sha(args.binary), 'armNativeArchiveSha256': NATIVE_SHA,
              'buildFeatures': ['offline-puresignal-owner-native'],
              'defaultTxOwner': 'legacy', 'productionManagedWiring': 'unfinished',
              'armExecuted': False, 'installedOnG2': False, 'rfQualified': False,
              'installerBundle': False, 'evidenceSha256': evidence,
              'remaining': ['explicit production startup owner selection',
                            'real generation-fenced feedback acquisition on both backends',
                            'production native cleanup observer',
                            'reviewed path-specific feedback limits and hardware qualification']}
    if args.wiring_review:
        record.update({
            'format':'saturn-puresignal-production-wiring-review-v1',
            'buildFeatures':['managed-puresignal'],
            'productionManagedWiring':'implemented-opt-in-awaiting-review',
            'liveAdmissionApproved':False,
            'managedRfPermissionDefault':False,
            'feedbackTopologyAttestationDefault':False,
            'measurementUnclippedQualification':'unavailable-with-existing-feedback-evidence',
            'webBundleSha256':sha(args.web_bundle),
            'nativeObserverSourceSha256':sha(REPO / 'update_manager/saturn-bridge/src/wdsp/ps_observer.c'),
            'nativePrivateHeaderPinsSha256':sha(REPO / 'update_manager/saturn-bridge/src/wdsp/ps-native-header-pins.txt'),
            'executedTests':{'architecture':'x86_64-local','managedStubBinary':551,'managedStubLibrary':48,
                             'replayHarness':14,'legacyStubBinary':464,'legacyStubLibrary':24,
                             'web':812,'realNativeRegression':7,'matchedProductionPathTests':2},
            'remaining':['independent source/package review before any controlled G2 test',
                         'reviewed path-specific feedback level/attenuation/delay policy',
                         'ARM execution, real RXC1 pair integrity and G2 response timing',
                         'physical RF-off and correction/spectral qualification',
                         'P2 queued-wire-generation ambiguity; bridge timestamps are not RF timestamps',
                         'full shared-backend RX/performance/telemetry parity and release packaging']})
    if args.runtime_review:
        record['format'] = 'saturn-puresignal-production-runtime-review-v1'
        record['actualRuntimeLoopTestedOffline'] = True
        record['controlledG2Procedure'] = 'source/update_manager/docs/puresignal/CONTROLLED_G2_PURESIGNAL_PROCEDURE_20261013.md'
        record['hardwareFeedbackLimitsApproved'] = False
        record['executedTests'].update({'managedStubLibrary':49, 'legacyStubLibrary':25,
                                       'actualRuntimeNativeTests':6, 'runInnerBackendScenarios':8})
    (output / 'review.json').write_text(json.dumps(record, indent=2) + '\n')
    report = ('source/update_manager/docs/puresignal/PRODUCTION_WIRING_ARM_REVIEW_20261013.md' if args.wiring_review else
              'source/update_manager/docs/puresignal/PRODUCTION_BASE_ARM_REVIEW_20261013.md')
    if args.runtime_review:
        report = 'source/update_manager/docs/puresignal/PRODUCTION_RUNTIME_ARM_REVIEW_20261013.md'
    (output / 'README.txt').write_text(
        'NON-INSTALLABLE review bundle. DO NOT activate or transmit with this candidate.\n'
        f'Read {report} and review.json. Verify SHA256SUMS before inspection.\n'
        'SOURCE_SHA256SUMS paths are relative to source/. The content pin is the SHA256 of that index.\n'
        'The tracked patch omits new files; the complete selected source snapshot includes them.\n'
        'No firmware, driver or polling changes. The G2 has not been modified.\n')
    files = sorted(p for p in output.rglob('*') if p.is_file())
    (output / 'SHA256SUMS').write_text(''.join(
        f'{sha(p)}  {p.relative_to(output).as_posix()}\n' for p in files))
    archive = Path(str(output) + '.zip')
    with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED) as bundle:
        for path in sorted(p for p in output.rglob('*') if p.is_file()):
            bundle.write(path, path.relative_to(output).as_posix())
    print(json.dumps({'archive': str(archive), 'sha256': sha(archive),
                      'sourcePin': pin, 'armBinarySha256': sha(args.binary)}))


if __name__ == '__main__':
    main()
