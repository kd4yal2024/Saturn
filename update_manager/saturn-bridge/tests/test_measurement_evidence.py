"""Offline checker regression tests. Synthetic JSON only; no radio/native I/O."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CHECKER = Path(__file__).with_name('verify-measurement-evidence.py')
spec = importlib.util.spec_from_file_location('checker', CHECKER)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
CASES = ['expiry', 'release', 'disconnect', 'missing', 'queue', 'config', 'native', 'gap']
BACKENDS = ['P2', 'DirectXdma']


def fixture(backend, case):
    fence = {'boot': '1', 'generation': 2, 'config': '3', 'txEpoch': 4}
    capture = {'backend': backend, 'sourcePin': 'synthetic-offline-fixture-not-hardware',
        'nativeIdentity': 'WDSP210 archive=8b75784e-test-fixture',
        'feedbackAttenuationCommandDb': 9, 'commandedDriveCode': 5,
        'overloadQualification': 'Unknown', 'overloadProvenUnclipped': False,
        'lp700Measurement': None, 'native': {'fittingEnabled': False, 'automaticEnabled': False,
            'correctionApplied': False, 'verified': True}, 'pairCount': 1,
        'referenceF64Bits': ['3fd0000000000000', '0000000000000000'],
        'rfF64Bits': ['3fc0000000000000', '0000000000000000'],
        'arrivalNs': [10], 'oldestObservedNs': 10, 'recordedNs': 20,
        'interval': {'first': 0, 'endExclusive': 1, 'fence': fence}, 'fence': fence,
        'scaling': {'converter': 'signed24/8388608'}}
    captures = [] if case == 'missing' else [capture]
    if case == 'queue': captures.append(copy.deepcopy(capture))
    return {'format': 'saturn-feedback-measurement-v1', 'rfImdQualified': False,
        'captureFinished': True, 'cleanupCompletedNs': 30,
        'terminalFault': {'disconnect': 'TxAuthority', 'missing': 'Stale', 'queue': 'QueueOverrun',
            'native': 'NativeFailure', 'gap': 'SequenceGap' if backend == 'P2' else 'DmaFraming'}.get(case),
        'recordingComplete': case in {'expiry', 'release', 'config'},
        'retentionComplete': case != 'queue', 'captures': captures}


class EvidenceCheckerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)
        for backend in BACKENDS:
            for case in CASES:
                self.write(backend, case, fixture(backend, case))

    def tearDown(self): self.temp.cleanup()

    def write(self, backend, case, record):
        (self.directory / f'{backend}-{case}.json').write_text(json.dumps(record))

    def cli(self, optimized=False):
        return subprocess.run([sys.executable, *(['-O'] if optimized else []),
            str(CHECKER), str(self.directory)], capture_output=True, text=True, timeout=5)

    def test_positive_controls_and_intentionally_missing_feedback(self):
        result = self.cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        summary = json.loads(result.stdout)
        self.assertEqual(summary['syntheticCaptureFilesVerified'], 16)
        self.assertEqual(summary['pairedBlocksVerified'], 16)
        self.assertFalse(summary['rfImdQualified'])

    def test_single_emptied_positive_file_exits_nonzero(self):
        record = fixture('P2', 'expiry')
        record['captures'] = []
        self.write('P2', 'expiry', record)
        result = self.cli()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('P2-expiry.json: scenario requires at least one capture', result.stderr)

    def test_entire_emptied_set_exits_nonzero(self):
        for backend in BACKENDS:
            for case in CASES:
                record = fixture(backend, case)
                record['captures'] = []
                self.write(backend, case, record)
        result = self.cli()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('"pairedBlocksVerified": 0', result.stdout)

    def test_each_required_scenario_rejects_empty_captures(self):
        for backend in BACKENDS:
            for case in CASES:
                if case == 'missing': continue
                with self.subTest(backend=backend, case=case):
                    record = fixture(backend, case)
                    record['captures'] = []
                    self.write(backend, case, record)
                    with self.assertRaisesRegex(ValueError, 'requires'):
                        checker.verify(self.directory)
                    self.write(backend, case, fixture(backend, case))

    def test_queue_requires_retained_and_overflow_blocks(self):
        for count in [1, 3]:
            record = fixture('DirectXdma', 'queue')
            record['captures'] = [copy.deepcopy(record['captures'][0]) for _ in range(count)]
            self.write('DirectXdma', 'queue', record)
            with self.assertRaisesRegex(ValueError, 'queue scenario requires exactly two'):
                checker.verify(self.directory)

    def test_missing_feedback_scenario_rejects_invented_capture(self):
        record = fixture('P2', 'missing')
        record['captures'] = fixture('P2', 'expiry')['captures']
        self.write('P2', 'missing', record)
        with self.assertRaisesRegex(ValueError, 'requires zero captures'):
            checker.verify(self.directory)

    def test_missing_file_fails(self):
        (self.directory / 'P2-expiry.json').unlink()
        self.assertNotEqual(self.cli().returncode, 0)

    def test_cardinality_checks_survive_python_optimization(self):
        record = fixture('P2', 'expiry')
        record['captures'] = []
        self.write('P2', 'expiry', record)
        self.assertNotEqual(self.cli(optimized=True).returncode, 0)


if __name__ == '__main__': unittest.main(verbosity=2)
