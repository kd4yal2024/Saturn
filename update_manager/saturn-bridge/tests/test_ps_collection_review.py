import cmath
import importlib.util
import math
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location('collection_review',Path(__file__).resolve().parents[1]/'scripts/review-ps-collection.py')
REVIEW = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REVIEW)


class CollectionReview(unittest.TestCase):
    def tones(self, first, second):
        return [first*cmath.exp(2j*math.pi*700*n/192000)+second*cmath.exp(2j*math.pi*1900*n/192000) for n in range(16384)]

    def test_balanced_positive_control_fills_all_buckets(self):
        tx = self.tones(.333,.333)
        report = REVIEW.histogram(tx,tx,.683)
        self.assertTrue(report['allBucketsHave256InRetainedWindow'])
        self.assertTrue(REVIEW.envelope_constraints(tx,tx)['anyScalarCanReachBothExtremeBucketsInThisWindow'])

    def test_measured_imbalance_has_no_scalar_covering_both_extremes(self):
        tx = self.tones(.359045,.308491)
        self.assertFalse(REVIEW.envelope_constraints(tx,tx)['anyScalarCanReachBothExtremeBucketsInThisWindow'])
        self.assertEqual(REVIEW.histogram(tx,tx,.68292)['counts'][0],0)

    def test_placeholder_misses_upper_buckets(self):
        tx = self.tones(.359045,.308491)
        self.assertEqual(REVIEW.histogram(tx,tx,1)['bucketsBelow256'],list(range(11,16)))

    def test_overrange_is_retained_and_reported_not_called_clear(self):
        r = REVIEW.histogram([1+0j]*256,[.1+0j]*256,.5)
        self.assertEqual(r['counts'][15],256)
        self.assertEqual(r['normalizedReferenceAtOrAboveOne'],256)
        self.assertFalse(r['allBucketsHave256InRetainedWindow'])

    def test_missing_rf_is_not_collection_excitation(self):
        with self.assertRaises(ValueError): REVIEW.histogram([.5+0j]*256,[0j]*256,.6)

    def test_exact_bucket_boundary_goes_to_successor(self):
        r = REVIEW.histogram([.0625+0j]*256,[.1+0j]*256,1)
        self.assertEqual(r['counts'][0],0)
        self.assertEqual(r['counts'][1],256)

    def test_empty_nonfinite_unequal_and_invalid_peak_refused(self):
        for tx,rf,peak in [([],[],.6),([1j],[],.6),([complex(float('nan'),0)],[1j],.6),([1j],[1j],0),([1j],[1j],float('nan'))]:
            with self.assertRaises(ValueError): REVIEW.histogram(tx,rf,peak)


if __name__=='__main__': unittest.main()
