"""Offline WDSP collection coverage; never creates live settings or RF authority."""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path

SPEC = importlib.util.spec_from_file_location('timing_review', Path(__file__).with_name('ps-timing-review.py'))
TIMING = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TIMING)
CALCC_SHA = '1aa8470294640e7109824db7cc41786a02e21a5f0b9cf54bbbcfaf2df190ab66'


def histogram(reference, rf, peak):
    if not math.isfinite(peak) or not 0 < peak <= 1 or not reference or len(reference) != len(rf):
        raise ValueError('bounded nonempty paired samples and valid HWPeak required')
    counts, ignored, over = [0]*16, 0, 0
    envs = []
    for tx, rx in zip(reference, rf):
        if not all(math.isfinite(v) for v in (tx.real, tx.imag, rx.real, rx.imag)):
            raise ValueError('nonfinite IQ')
        env, feedback = abs(tx), abs(rx)
        if env < 1e-30 or feedback < 1e-30 or env/peak < .005:
            ignored += 1
            continue
        envs.append(env)
        norm = env/peak
        # Exact pinned putSample policy: overrange is retained in bucket15.
        # Count it separately; retention is NOT proof of unclipped samples.
        over += norm >= 1
        counts[min(15, int(norm*16))] += 1
    if not envs:
        raise ValueError('no eligible reference/RF samples')
    return {'hardwarePeakUnderReview': peak, 'counts': counts,
            'bucketsBelow256': [i for i,n in enumerate(counts) if n < 256],
            'allBucketsHave256InRetainedWindow': all(n >= 256 for n in counts),
            'ignoredSamples': ignored, 'normalizedReferenceAtOrAboveOne': over,
            'eligibleReferenceEnvelopeMin': min(envs), 'eligibleReferenceEnvelopeMax': max(envs)}


def envelope_constraints(reference, rf):
    envs = [abs(t) for t,r in zip(reference,rf) if abs(t) >= 1e-30 and abs(r) >= 1e-30]
    if not envs:
        raise ValueError('no nonzero paired envelope')
    # Necessary conditions only: bottom bucket norm<1/16, top norm>=15/16.
    lower, upper = min(envs)*16, max(envs)*16/15
    return {'hardwarePeakMustExceedForAnyBottomBucketSample': lower,
            'hardwarePeakMustNotExceedForAnyTopBucketSample': upper,
            'anyScalarCanReachBothExtremeBucketsInThisWindow': lower < upper,
            'conditionsAreNecessaryNotSufficient': True}


def review(path, peaks):
    meta, rows, digest = TIMING.load(path)
    spectrum = meta['spectrum']
    if spectrum.get('continuity') != 'contiguous':
        raise ValueError('contiguous retained interval required')
    reference = [complex(r[0],r[1]) for r in rows]
    rf = [complex(r[2],r[3]) for r in rows]
    return {'metadataSha256': digest, 'rawSha256': meta['rawSha256'],
            'sourcePin': meta['sourcePin'], 'nativeArchiveSha256': meta['wdspArchiveSha256'],
            'context': {k:spectrum.get(k) for k in ('generation','config','txEpoch','sourceBoot','backend','centerHz','rateHz')},
            'overload': spectrum.get('overload'), 'pairs':len(rows),
            'profiles': [histogram(reference,rf,p) for p in peaks],
            'envelopeConstraints': envelope_constraints(reference,rf),
            'capturePath':str(path)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('metadata', type=Path, nargs='+')
    parser.add_argument('--wdsp-source', type=Path, required=True)
    parser.add_argument('--peak', type=float, action='append', required=True)
    args = parser.parse_args()
    try:
        if not 1 <= len(args.metadata) <= 16 or not 1 <= len(args.peak) <= 8:
            raise ValueError('bounded nonempty capture/profile set required')
        source = TIMING.ordinary(args.wdsp_source, 256_000)
        if hashlib.sha256(source).hexdigest() != CALCC_SHA:
            raise ValueError('unreviewed WDSP collection source')
        reports = [review(p,args.peak) for p in args.metadata]
        if len({r['rawSha256'] for r in reports}) != len(reports):
            raise ValueError('duplicate evidence does not establish repeatability')
        result = {'format':'saturn-ps-collection-review-v1', 'calccSourceSha256':CALCC_SHA,
                  'reports':reports, 'hostSettings':None, 'fittingAuthorized':False,
                  'rfImdQualified':False,
                  'limitations':['Histogram of retained samples, not a successful native fit.',
                     'Separate captures stay separate; samples and their scales are unchanged.',
                     'Unknown overload stays unknown; no live feedback limits are inferred.',
                     'Collection requirements are those of the exact pinned source, not generic WDSP.']}
    except (ValueError,KeyError,TypeError,OSError) as error:
        parser.error(str(error))
    print(json.dumps(result,indent=2,allow_nan=False))


if __name__=='__main__':
    main()
