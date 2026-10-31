import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('shared_review',Path(__file__).resolve().parents[1]/'scripts/review-shared-signal.py')
REVIEW = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REVIEW)


class SharedSignalReview(unittest.TestCase):
    def fixture(self, path):
        (path/'context.json').write_text(json.dumps({'format':'saturn-shared-signal-context-v1','sourcePin':'a'*64}))
        for i,kind in enumerate(['Pcm','Wdsp','H2c','Pairs']):
            values = [0.1]*512 if kind=='Pcm' else [0.2,0.1]*128 if kind=='Wdsp' else [0.2,0.1,0.1,0.05]*128 if kind=='Pairs' else []
            data=struct.pack('<'+'d'*len(values),*values)
            packed=(b'\x00\x10\x00\x00\x20\x00'*240) if kind=='H2c' else b''
            arrivals=struct.pack('<128Q',*([10]*128)) if kind=='Pairs' else b''
            stem=f'capture-{i:04d}'
            m={'format':'saturn-shared-signal-capture-v1','kind':kind,'backend':'DirectXdma','boot':'1','config':'2',
               'generation':'3','txEpoch':'4','outputGeneration':'5','outputRevision':'6','values':len(values),
               'packedBytes':len(packed),'arrivalCount':len(arrivals)//8,'sourceOldestNs':10,'recordedNs':20,
               'submittedNs':11,'completedNs':12,'keyedSnapshot':True,'completedBytes':len(packed),'requestedBytes':len(packed),
               'firstReturnedPair':100,'returnedIntervalEnd':228,'arrivalTimesRecorded':kind=='Pairs',
               'valuesSha256':hashlib.sha256(data).hexdigest(),'packedSha256':hashlib.sha256(packed).hexdigest(),
               'arrivalSha256':hashlib.sha256(arrivals).hexdigest()}
            (path/(stem+'.json')).write_text(json.dumps(m));(path/(stem+'.f64le')).write_bytes(data)
            if packed:(path/(stem+'.packed')).write_bytes(packed)
            if arrivals:(path/(stem+'.arrival-u64le')).write_bytes(arrivals)
        return path

    def test_positive_chain_is_not_hwpeak_or_rf_qualification(self):
        with tempfile.TemporaryDirectory() as d:
            result=REVIEW.review(self.fixture(Path(d)))
            self.assertTrue(result['captureChainPresent']);self.assertIsNone(result['recommendedHwPeak'])
            self.assertFalse(result['rfImdQualified']);self.assertFalse(result['sampleAligned'])

    def test_native_generator_requires_zero_pcm_and_explicit_same_stimulus(self):
        with tempfile.TemporaryDirectory() as d:
            path=self.fixture(Path(d))
            for file in path.glob('capture-*.json'):
                m=json.loads(file.read_text());m['stimulus']='wdsp-postgen-two-tone-v1'
                if m['kind']=='Pcm':
                    data=b'\0'*(512*8);file.with_suffix('.f64le').write_bytes(data);m['valuesSha256']=hashlib.sha256(data).hexdigest()
                file.write_text(json.dumps(m))
            self.assertTrue(REVIEW.review(path)['captureChainPresent'])
            file=path/'capture-0000.json';m=json.loads(file.read_text());data=struct.pack('<512d',*([.1]*512))
            file.with_suffix('.f64le').write_bytes(data);m['valuesSha256']=hashlib.sha256(data).hexdigest();file.write_text(json.dumps(m))
            with self.assertRaises(ValueError):REVIEW.review(path)

    def test_empty_set_and_missing_point_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            path=self.fixture(Path(d));(path/'capture-0002.json').unlink()
            with self.assertRaises(ValueError):REVIEW.review(path)
            for p in path.glob('capture-*.json'):p.unlink()
            with self.assertRaises(ValueError):REVIEW.review(path)

    def test_tampered_payload_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            path=self.fixture(Path(d));(path/'capture-0001.f64le').write_bytes(b'\x00'*2048)
            with self.assertRaises(ValueError):REVIEW.review(path)

    def test_stale_snapshot_preserves_wide_times_and_unknown_not_zero(self):
        with tempfile.TemporaryDirectory() as d:
            path=self.fixture(Path(d));m=json.loads((path/'capture-0000.json').read_text())
            words=[3,2**60+17,42,2**64-1,2**55+31,45000000000,250000000,150000000]
            values=[float(v) for w in words for v in (w&0xffffffff,w>>32)]
            data=struct.pack('<16d',*values)
            m.update(kind='Stale',values=16,packedBytes=0,arrivalCount=0,valuesSha256=hashlib.sha256(data).hexdigest(),
                     packedSha256=hashlib.sha256(b'').hexdigest(),arrivalSha256=hashlib.sha256(b'').hexdigest())
            (path/'capture-0004.json').write_text(json.dumps(m));(path/'capture-0004.f64le').write_bytes(data)
            event=REVIEW.review(path)['staleEvents'][0]
            self.assertEqual(event['origin'],'retained-mic')
            self.assertEqual(event['snapshot']['nowNs'],str(2**60+17))
            self.assertEqual(event['snapshot']['micOldestNs'],str(2**55+31))
            self.assertIsNone(event['snapshot']['lastArrivalNs'])

    def test_short_write_not_accepted_as_completed_output(self):
        with tempfile.TemporaryDirectory() as d:
            path=self.fixture(Path(d));p=path/'capture-0002.json';m=json.loads(p.read_text());m['completedBytes']=6;p.write_text(json.dumps(m))
            with self.assertRaises(ValueError):REVIEW.review(path)

    def test_cross_generation_cannot_complete_a_chain(self):
        with tempfile.TemporaryDirectory() as d:
            path=self.fixture(Path(d));p=path/'capture-0003.json';m=json.loads(p.read_text());m['outputGeneration']='7';p.write_text(json.dumps(m))
            with self.assertRaises(ValueError):REVIEW.review(path)

    def test_export_errors_or_missing_last_capture_are_not_complete(self):
        for errors,count in [(1,4),(0,5)]:
            with tempfile.TemporaryDirectory() as d:
                path=self.fixture(Path(d));(path/'capture-summary.json').write_text(json.dumps({'drained':True,'exportErrors':errors,'recordsConsumed':count}))
                with self.assertRaises(ValueError):REVIEW.review(path)


if __name__=='__main__':unittest.main()
