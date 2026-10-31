"""Offline format/converter/listening-copy tests, no native or radio access."""
import importlib.util
from pathlib import Path
import struct
import unittest


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).parents[1] / 'scripts' / file)
    result = importlib.util.module_from_spec(spec); spec.loader.exec_module(result)
    return result


converter = module('converter', 'feedback-diagnostics.py')
wav = module('wav', 'diagnostic-pcm-wav.py')


def fixture(backend):
    return {'format': 'saturn-feedback-measurement-v1', 'captures': [{
        'backend': backend, 'fence': {'boot': str(1 << 100), 'generation': 3, 'config': str(1 << 110), 'txEpoch': 4},
        'pairCount': 2, 'rateHz': 192000, 'oldestObservedNs': 10, 'recordedNs': 30, 'arrivalNs': [10, 20],
        'scaling': {'converter': 'signed24/8388608', 'normalizationApplied': False},
        'referenceF64Bits': ['3fd0000000000000', '8000000000000000', '7ff8000000000031', '0000000000000000'],
        'rfF64Bits': ['3fc0000000000000', '0000000000000000'] * 2}]}


class DiagnosticsTests(unittest.TestCase):
    def test_both_backends_keep_exact_iq_and_128_bit_fences(self):
        for backend in ['P2', 'DirectXdma']:
            rows = converter.windows(fixture(backend))
            self.assertEqual(len(rows), 2)
            raw = rows[0][1]
            self.assertEqual(len(raw), 120 + 4 * 8)
            self.assertEqual(int.from_bytes(raw[16:32], 'little'), 1 << 100)
            self.assertEqual(int.from_bytes(raw[40:56], 'little'), 1 << 110)
            self.assertEqual(raw[128:136], bytes.fromhex('0000000000000080'))
            self.assertEqual(raw[136:144], bytes.fromhex('310000000000f87f'))
            self.assertEqual(raw[12:15], bytes([0, 0, 2]))  # unknown evidence/keyed

    def test_empty_or_summary_only_evidence_is_not_a_capture(self):
        for record in [{'format': 'saturn-two-tone-summary-v1'}, {'format': 'saturn-feedback-measurement-v1', 'captures': []}]:
            with self.assertRaises(ValueError): converter.windows(record)

    def test_missing_arrivals_remain_unknown_and_mismatches_rejected(self):
        doc = fixture('P2'); doc['captures'][0]['arrivalNs'] = None
        raw = converter.windows(doc)[0][1]
        self.assertEqual(struct.unpack_from('<Q', raw, 72)[0], converter.UNKNOWN)
        doc['captures'][0]['arrivalNs'] = [20, 40]
        with self.assertRaises(ValueError): converter.windows(doc)

    def test_no_silent_rescaling(self):
        doc = fixture('P2'); doc['captures'][0]['scaling']['normalizationApplied'] = True
        with self.assertRaises(ValueError): converter.windows(doc)

    def test_wav_refuses_complex_iq_and_bad_pcm(self):
        raw = converter.windows(fixture('P2'))[0][1]
        with self.assertRaises(ValueError): wav.pcm(raw)
        header = bytearray(raw[:120]); header[8] = 0
        struct.pack_into('<IId', header, 104, 48000, 2, 1.0)
        rate, encoded = wav.pcm(bytes(header) + struct.pack('<2d', .5, -.5))
        self.assertEqual(rate, 48000); self.assertEqual(len(encoded), 4)
        for values in [(1.1, 0), (float('nan'), 0)]:
            with self.assertRaises(ValueError): wav.pcm(bytes(header) + struct.pack('<2d', *values))


if __name__ == '__main__': unittest.main()
