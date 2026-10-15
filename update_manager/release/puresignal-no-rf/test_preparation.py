"""Local preparation checks only: never execute ARM, systemctl, fuser or RF."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('launcher', HERE / 'exec-no-rf.py')
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


class PreparationTests(unittest.TestCase):
    def test_no_rf_environment_ignores_inherited_rf_settings(self):
        with patch.dict('os.environ', {'SATURN_REMOTE_TX_RF_ENABLED': '1',
                                     'SATURN_PS_MANAGED_RF_PERMITTED': '1',
                                     'SATURN_PS_ADMISSION': 'operational', 'LD_PRELOAD': '/bad.so'}):
            env = launcher.no_rf_environment(Path('/tmp/new-no-rf-run'))
        self.assertEqual(env['SATURN_REMOTE_TX_RF_ENABLED'], '0')
        self.assertEqual(env['SATURN_PS_MANAGED_RF_PERMITTED'], '0')
        self.assertEqual(env['SATURN_REMOTE_TWOTONE_RF_DISABLED_OK'], '0')
        self.assertEqual(env['SATURN_PS_ADMISSION'], 'strict')
        self.assertEqual(env['SATURN_BRIDGE_TX_OWNER'], 'shared')
        self.assertEqual(env['SATURN_BRIDGE_TCI_HOST'], '127.0.0.1')
        self.assertNotIn('LD_PRELOAD', env)
        self.assertNotIn('SATURN_PS_FEEDBACK_RMS_FLOOR', env)

    def test_candidate_hash_and_lock_cannot_be_redefined(self):
        lock = json.loads((HERE / 'candidate-lock.json').read_text())
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'wrong'
            binary.write_bytes(b'not an ARM candidate')
            with self.assertRaisesRegex(RuntimeError, 'binary hash mismatch'):
                launcher.verify_candidate(binary, lock)
            lock['reviewedSourcePin'] = '0' * 64
            with self.assertRaisesRegex(RuntimeError, 'lock mismatch'):
                launcher.verify_candidate(binary, lock)

    def test_native_identity_requires_every_reviewed_field(self):
        lock = json.loads((HERE / 'candidate-lock.json').read_text())
        record = {'format': 'saturn-native-build-info-v1', 'target': 'aarch64',
                  'sourcePin': launcher.SOURCE_PIN, 'wdspVersion': 210,
                  'wdspArchiveSha256': lock['nativeArchiveSha256'],
                  'wdspHeaderSha256': lock['nativeHeaderSha256'], 'stubNative': False,
                  'defaultTxOwner': 'legacy', 'productionManagedWiring': 'opt-in-review', 'rfQualified': False}
        launcher.validate_identity(record, lock)
        for key in record:
            bad = record.copy()
            bad[key] = None
            with self.assertRaisesRegex(RuntimeError, 'identity mismatch'):
                launcher.validate_identity(bad, lock)


if __name__ == '__main__':
    unittest.main(verbosity=2)
