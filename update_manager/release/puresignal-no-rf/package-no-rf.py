"""Assemble an immutable no-RF preparation kit; no install or radio operation."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import zipfile

HERE = Path(__file__).resolve().parent
LOCK = json.loads((HERE / 'candidate-lock.json').read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--review-zip', type=Path, required=True)
    parser.add_argument('--review-root', type=Path, required=True)
    parser.add_argument('--rebuild-root', type=Path, required=True)
    parser.add_argument('--input-lock', type=Path, required=True)
    parser.add_argument('--test-log', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if sha(args.review_zip) != LOCK['reviewedBundleSha256']:
        raise RuntimeError('accepted review ZIP mismatch')
    rebuilt = args.rebuild_root / 'target' / LOCK['target'] / 'release/saturn-bridge'
    accepted = args.review_root / 'arm-candidate/saturn-bridge'
    if sha(rebuilt) != LOCK['reviewedBinarySha256'] or sha(accepted) != LOCK['reviewedBinarySha256']:
        raise RuntimeError('rebuild not identical to accepted candidate')
    rebuild_record = json.loads((args.rebuild_root / 'rebuild-result.json').read_text())
    if rebuild_record['matchesReviewedBinary'] is not True:
        raise RuntimeError('rebuild result did not pass')
    output = args.output.resolve()
    if output.exists() or Path(str(output) + '.zip').exists():
        raise RuntimeError('output and archive must both be new')
    output.mkdir(parents=True)
    for p in sorted(HERE.iterdir()):
        if p.is_file() and p.suffix in ['.py', '.md', '.json']:
            shutil.copy2(p, output / p.name)
    (output / 'arm-candidate').mkdir()
    shutil.copy2(accepted, output / 'arm-candidate/saturn-bridge')
    shutil.copy2(args.review_zip, output / 'accepted-runtime-review.zip')
    shutil.copy2(args.input_lock, output / 'build-inputs.json')
    evidence = output / 'evidence'
    evidence.mkdir()
    for name in ['build-command.json', 'build.log', 'rebuild-result.json']:
        shutil.copy2(args.rebuild_root / name, evidence / name)
    shutil.copy2(args.test_log, evidence / 'preparation-tests.txt')
    record = {'format': 'saturn-puresignal-no-rf-preparation-result-v1',
              'acceptedSourcePin': LOCK['reviewedSourcePin'],
              'acceptedBinarySha256': sha(accepted), 'rebuiltBinarySha256': sha(rebuilt),
              'cleanRebuildMatchesAccepted': True, 'buildInputsSha256': sha(args.input_lock),
              'bridgeSourceChanged': False, 'armExecuted': False, 'installedOnG2': False,
              'hardwareExecutionAuthorized': False, 'rfTestingAuthorized': False,
              'feedbackDiagnosticImplemented': False, 'benchConfirmation': 'pending',
              'serviceChangesExecuted': False, 'firmwareDriverPollingChanges': False}
    (output / 'preparation.json').write_text(json.dumps(record, indent=2) + '\n')
    files = sorted(p for p in output.rglob('*') if p.is_file())
    (output / 'SHA256SUMS').write_text(''.join(f'{sha(p)}  {p.relative_to(output)}\n' for p in files))
    archive = Path(str(output) + '.zip')
    with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED) as bundle:
        for p in sorted(output.rglob('*')):
            if p.is_file():
                bundle.write(p, p.relative_to(output))
    print(json.dumps({'archive': str(archive), 'sha256': sha(archive),
                      'filesVerifiedByManifest': len(files), 'sourcePinUnchanged': LOCK['reviewedSourcePin']}))


if __name__ == '__main__':
    main()
