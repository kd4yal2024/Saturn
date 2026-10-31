import cmath
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import random
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('timing_review', Path(__file__).resolve().parents[1]/'scripts/ps-timing-review.py')
review = importlib.util.module_from_spec(spec)
spec.loader.exec_module(review)


def delayed(values, lag):
    return [values[n-lag]*.5*cmath.exp(.4j) if 0 <= n-lag < len(values) else 0j for n in range(len(values))]


class CorrelationTests(unittest.TestCase):
    def test_signed_delay_and_unknown_total_uncertainty(self):
        rng = random.Random(14)
        samples = [complex(rng.uniform(-.2,.2),rng.uniform(-.2,.2)) for _ in range(512)]
        for lag in [-7,0,7]:
            result = review.correlate(samples,delayed(samples,lag),32)
            self.assertEqual(result['candidateRfLagSamples'],lag)
            self.assertEqual(result['validity'],'unique-peak-in-search-not-qualified-delay')
            self.assertIsNone(result['totalUncertaintySeconds'])
            self.assertFalse(result['wdspSettingAuthorized'])

    def test_two_tone_periodicity_stays_ambiguous(self):
        samples = [.15*(cmath.exp(2j*math.pi*700*n/192000)+cmath.exp(2j*math.pi*1900*n/192000)) for n in range(2048)]
        result = review.correlate(samples,delayed(samples,9),400)
        self.assertEqual(result['validity'],'ambiguous')

    def test_bandlimited_irregular_multisine_has_a_broad_but_distinct_peak(self):
        rng = random.Random(0x53465031)
        frequencies = [400+(k+.1+.8*rng.random())*2200/32 for k in range(32)]
        phases = [rng.random()*2*math.pi for _ in frequencies]
        samples = [.008*sum(cmath.exp(1j*(2*math.pi*f*n/192000+p)) for f,p in zip(frequencies,phases)) for n in range(2048)]
        result = review.correlate(samples,delayed(samples,-13),256)
        self.assertEqual(result['candidateRfLagSamples'],-13)
        self.assertEqual(result['validity'],'unique-peak-in-search-not-qualified-delay')
        self.assertGreater(result['mainLobe3dBWidthSamples'],2)

    def test_boundary_is_not_a_resolved_delay(self):
        rng = random.Random(9)
        samples = [complex(rng.random(),rng.random()) for _ in range(256)]
        self.assertEqual(review.correlate(samples,delayed(samples,16),16)['validity'],'boundary-peak')

    def test_constant_carrier_and_unrelated_signal_are_not_timing_evidence(self):
        self.assertEqual(review.correlate([.1j]*256,[.2j]*256,16)['validity'],'insufficient-correlated-variation')
        rng = random.Random(9)
        a,b = [[complex(rng.random(),rng.random()) for _ in range(512)] for _ in range(2)]
        self.assertEqual(review.correlate(a,b,32)['validity'],'insufficient-correlated-variation')


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.path = self.root/'window.json'
        self.meta = {'format':'saturn-rf-paired-window-v1','sourcePin':'a'*64,'wdspArchiveSha256':'b'*64,
          'pairs':16384,'recordBytes':24,
          'recordLayout':'ref_I:f32le,ref_Q:f32le,rf_I:f32le,rf_Q:f32le,original_bridge_arrival_ns:u64le',
          'scaling':'signed24 / 8388608; exact normalized float32, no AGC/resampling',
          'timeDomain':'original Bridge ingress, not FPGA/RF time','rawFile':'window.pairs',
          'spectrum':{'rateHz':192000,'continuity':'contiguous','overload':'unknown','generation':'1','config':'2','txEpoch':'3'}}
        rng = random.Random(99)
        self.values = [complex(rng.uniform(-.2,.2),rng.uniform(-.2,.2)) for _ in range(16384)]
        self.arrivals = list(range(16384))
        self.save()

    def tearDown(self): self.directory.cleanup()

    def save(self):
        raw = b''.join(struct.pack('<ffffQ',v.real,v.imag,v.real/2,v.imag/2,t) for v,t in zip(self.values,self.arrivals))
        (self.root/'window.pairs').write_bytes(raw)
        self.meta['rawSha256'] = hashlib.sha256(raw).hexdigest()
        self.path.write_text(json.dumps(self.meta))

    def test_originals_preserved_unknown_overload_and_no_automatic_handoff(self):
        before = self.path.read_bytes(),(self.root/'window.pairs').read_bytes()
        for backend in ['P2','DirectXdma']:
            self.meta['spectrum']['backend'] = backend
            self.path.write_text(json.dumps(self.meta))
            result = review.analyze(self.path,32,256)
            self.assertEqual(result['overload'],'unknown')
            self.assertFalse(result['unclippedMeasurementQualified'])
            self.assertFalse(result['fittingAuthorized'])
            self.assertIsNone(result['hardwarePeakRecommendation'])
            self.assertIsNone(result['recommendedHostSettings'])
        self.assertEqual(before[1],(self.root/'window.pairs').read_bytes())

    def test_corrupt_raw_rejected(self):
        with (self.root/'window.pairs').open('ab') as stream: stream.write(b'x')
        with self.assertRaises(ValueError): review.analyze(self.path,32,256)

    def test_requested_timing_identity_survives_analysis_without_certifying_the_signal(self):
        self.meta['spectrum'].update(stimulus='saturn-timing-multisine-v1',requestedStimulus={
            'version':'saturn-timing-multisine-v1','seed':0x53465031,'pcmRateHz':48000,'repeatMs':1000})
        self.path.write_text(json.dumps(self.meta))
        result=review.analyze(self.path,32,256)
        self.assertEqual(result['captureContext']['stimulus'],'saturn-timing-multisine-v1')
        self.assertEqual(result['captureContext']['requestedStimulus']['repeatMs'],1000)
        self.assertFalse(result['fittingAuthorized'])

    def test_silent_startup_window_is_not_analyzed(self):
        self.values = [0j]*16384;self.save()
        self.assertEqual(review.analyze(self.path,32,256)['delay']['reason'],'insufficient-level')

    def test_regression_discontinuity_and_overload_do_not_certify_delay(self):
        self.arrivals[100] = 0;self.save()
        with self.assertRaises(ValueError): review.analyze(self.path,32,256)
        self.arrivals = list(range(16384));self.save()
        for key,value,reason in [('continuity','gap','continuity-unavailable-or-faulted'),
                                 ('overload','clipped','overload-fault-or-unrecognized-evidence')]:
            self.meta['spectrum'][key] = value;self.path.write_text(json.dumps(self.meta))
            self.assertEqual(review.analyze(self.path,32,256)['delay']['reason'],reason)
            self.meta['spectrum'][key] = 'contiguous' if key=='continuity' else 'unknown'

    def test_digital_full_scale_and_nonfinite_are_not_clear(self):
        self.values[50] = complex(1.,0);self.save()
        self.assertEqual(review.analyze(self.path,32,256)['delay']['reason'],'digital-full-scale-samples')
        self.values[50] = complex(float('nan'),0);self.save()
        with self.assertRaises(ValueError): review.analyze(self.path,32,256)

    def test_symlinks_and_raw_path_traversal_rejected(self):
        alias = self.root/'alias.json';alias.symlink_to(self.path)
        with self.assertRaises(OSError): review.load(alias)
        self.meta['rawFile'] = '../window.pairs';self.path.write_text(json.dumps(self.meta))
        with self.assertRaises(ValueError): review.load(self.path)


class NativeOutputTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get('SATURN_PS_TIMING_NATIVE_IQ'),'explicit offline native output required')
    def test_real_wdsp_probe_output_resolves_injected_delays_but_is_not_rf_evidence(self):
        raw = review.ordinary(Path(os.environ['SATURN_PS_TIMING_NATIVE_IQ']),2_000_000)
        data = [complex(a,b) for a,b in struct.iter_unpack('<ff',raw)]
        samples = data[len(data)//3:len(data)//3+4096]
        self.assertEqual(len(samples),4096)
        for lag in [-17,23]:
            result = review.correlate(samples,delayed(samples,lag),512)
            self.assertEqual(result['candidateRfLagSamples'],lag)
            self.assertEqual(result['validity'],'unique-peak-in-search-not-qualified-delay')
            self.assertFalse(result['wdspSettingAuthorized'])


class HandoffTests(unittest.TestCase):
    @staticmethod
    def report(index, lag=4, validity='unique-peak-in-search-not-qualified-delay'):
        return {'sourcePin':'a'*64,'wdspArchiveSha256':'b'*64,'rawSha256':hashlib.sha256(str(index).encode()).hexdigest(),
            'arrivalFirstNs':str(index*1000),'arrivalLastNs':str(index*1000+500),
            'delay':{'candidateRfLagSamples':lag,'validity':validity},'overload':'unknown',
            'captureContext':{'boot':'1','backend':'DirectXdma','rateHz':192000,'centerHz':7200000,'commandedWatts':5,'attenuationCommandDb':0}}

    def test_empty_and_duplicate_evidence_are_rejected(self):
        with self.assertRaises(ValueError): review.handoff_summary([])
        with self.assertRaises(ValueError): review.handoff_summary([self.report(1),self.report(1)])

    def test_repeatable_candidate_still_cannot_authorize_fitting_or_rf(self):
        result = review.handoff_summary([self.report(1),self.report(2,5)])
        self.assertTrue(result['repeatableIntegerCandidateWithinOneSample'])
        self.assertFalse(result['fittingAuthorized']);self.assertFalse(result['rfKeyingAuthorized'])
        self.assertIsNone(result['hostSettings']);self.assertEqual(result['overloadCoverage'],['unknown','unknown'])

    def test_ambiguous_different_settings_and_overlapping_windows_do_not_prove_repeatability(self):
        for change in ['ambiguous','settings','time','lag','missing','boot']:
            a,b = self.report(1),self.report(2)
            if change=='ambiguous': b['delay']['validity']='ambiguous'
            if change=='settings': b['captureContext']['commandedWatts']=3
            if change=='time': b['arrivalFirstNs']='1400'
            if change=='lag': b['delay']['candidateRfLagSamples']=10
            if change=='missing': b['captureContext']['attenuationCommandDb']=None
            if change=='boot': b['captureContext']['boot']='2'
            self.assertFalse(review.handoff_summary([a,b])['repeatableIntegerCandidateWithinOneSample'])

    def test_mixed_native_or_source_pins_are_rejected(self):
        for field in ['sourcePin','wdspArchiveSha256']:
            a,b = self.report(1),self.report(2);b[field]='c'*64
            with self.assertRaises(ValueError): review.handoff_summary([a,b])


if __name__ == '__main__': unittest.main()
