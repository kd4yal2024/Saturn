"""Offline paired-window timing review. No sockets, devices, PTT or native calls.

Reads the bounded RF-monitor archive without rewriting it. Delay is a candidate
within an explicit search, not automatically a WDSP setting or TX authorization.
Unknown overload and unresolved timing remain unknown/unresolved in the report.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import struct


def ordinary(path, limit):
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as stream:
        meta = os.fstat(stream.fileno())
        if not stat.S_ISREG(meta.st_mode) or meta.st_size > limit:
            raise ValueError('bounded ordinary file required')
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError('input grew beyond its bound')
    return data


def load(path):
    original = ordinary(path, 128_000)
    meta = json.loads(original)
    if not isinstance(meta, dict):
        raise ValueError('metadata object required')
    if (meta.get('format') != 'saturn-rf-paired-window-v1' or meta.get('pairs') != 16384
            or meta.get('recordBytes') != 24
            or meta.get('recordLayout') != 'ref_I:f32le,ref_Q:f32le,rf_I:f32le,rf_Q:f32le,original_bridge_arrival_ns:u64le'
            or meta.get('scaling') != 'signed24 / 8388608; exact normalized float32, no AGC/resampling'
            or meta.get('timeDomain') != 'original Bridge ingress, not FPGA/RF time'):
        raise ValueError('unsupported raw paired-window contract')
    if any(not re.fullmatch('[a-f0-9]{64}', str(meta.get(k,''))) for k in ('sourcePin','wdspArchiveSha256','rawSha256')):
        raise ValueError('explicit source/native/raw SHA256 identities required')
    name = meta.get('rawFile')
    if not isinstance(name, str) or Path(name).name != name or name in ('.', '..'):
        raise ValueError('raw file must be a local basename')
    raw = ordinary(path.parent / name, 16384 * 24)
    if len(raw) != 16384 * 24 or hashlib.sha256(raw).hexdigest() != meta.get('rawSha256'):
        raise ValueError('raw length/hash mismatch')
    rows = list(struct.iter_unpack('<ffffQ', raw))
    if any(not math.isfinite(v) for row in rows for v in row[:4]):
        raise ValueError('nonfinite IQ')
    spectrum = meta.get('spectrum', {})
    if not isinstance(spectrum, dict) or type(spectrum.get('rateHz')) is not int or spectrum['rateHz'] != 192000:
        raise ValueError('expected original 192kHz paired samples')
    # Input order is retained. A reset/reordering cannot be retimed into validity.
    if any(a[4] > b[4] for a, b in zip(rows, rows[1:])):
        raise ValueError('original arrival times regressed')
    return meta, rows, hashlib.sha256(original).hexdigest()


def levels(data):
    return {'complexRms': math.sqrt(sum(abs(v)**2 for v in data) / len(data)),
            'envelopePeak': max(map(abs, data)),
            'componentPeak': max(max(abs(v.real), abs(v.imag)) for v in data),
            'samplesAtOrAboveFullScale': sum(abs(v.real) >= 1 or abs(v.imag) >= 1 for v in data)}


def correlate(reference, rf, max_lag):
    n = len(reference)
    if n != len(rf) or n < 128 or not 1 <= max_lag <= min(n//4, 4096):
        raise ValueError('invalid bounded correlation geometry')
    scores = []
    for lag in range(-max_lag, max_lag+1):
        first, end = max(0, -lag), min(n, n-lag)
        tx, rx = reference[first:end], rf[first+lag:end+lag]
        mt, mr = sum(tx)/len(tx), sum(rx)/len(rx)
        energy_t = sum(abs(v-mt)**2 for v in tx)
        energy_r = sum(abs(v-mr)**2 for v in rx)
        cross = sum((a-mt).conjugate()*(b-mr) for a, b in zip(tx, rx))
        denominator = math.sqrt(energy_t*energy_r)
        scores.append(abs(cross)/denominator if denominator > 1e-18 else 0.)
    best = max(range(len(scores)), key=scores.__getitem__)
    lag, score = best-max_lag, scores[best]
    # Compare distinct local maxima rather than treating neighboring samples
    # inside a band-limited main lobe as competing propagation paths.
    maxima = [i for i in range(1, len(scores)-1)
              if scores[i] >= scores[i-1] and scores[i] >= scores[i+1]
              and (scores[i] > scores[i-1]+1e-12 or scores[i] > scores[i+1]+1e-12)]
    competitors = [scores[i] for i in maxima if abs(i-best) > 1]
    competitors += [scores[0], scores[-1]] if best not in (0, len(scores)-1) else []
    competing = max(competitors, default=0.)
    left = right = best
    while left > 0 and scores[left-1] >= score/math.sqrt(2): left -= 1
    while right+1 < len(scores) and scores[right+1] >= score/math.sqrt(2): right += 1
    if score < .5:
        validity = 'insufficient-correlated-variation'
    elif best in (0, len(scores)-1):
        validity = 'boundary-peak'
    elif competing >= score*.9 or left == 0 or right == len(scores)-1:
        validity = 'ambiguous'
    else:
        validity = 'unique-peak-in-search-not-qualified-delay'
    return {'candidateRfLagSamples': lag if score >= .5 else None,
            'candidateRfLagSeconds': lag/192000 if score >= .5 else None,
            'correlationMagnitude': score, 'competingPeakMagnitude': competing,
            'searchHalfWidthSamples': max_lag, 'mainLobe3dBWidthSamples': right-left+1,
            'validity': validity, 'integerQuantizationHalfSampleSeconds': .5/192000,
            'totalUncertaintySeconds': None,
            'signConvention': 'positive: RF[n+lag] matches reference[n]; RF arrives later',
            'wdspSettingAuthorized': False}


def analyze(path, max_lag=512, count=4096):
    meta, rows, meta_sha = load(path)
    if not 128 <= count <= len(rows):
        raise ValueError('invalid bounded analysis window')
    tx = [complex(r[0], r[1]) for r in rows]
    rf = [complex(r[2], r[3]) for r in rows]
    tl, rl = levels(tx), levels(rf)
    spectrum = meta['spectrum']
    # Observation confidence is not inferred from sample values or JSON hashes.
    reason = None
    if spectrum.get('continuity') != 'contiguous': reason = 'continuity-unavailable-or-faulted'
    elif spectrum.get('overload') not in ('unknown', 'clear'): reason = 'overload-fault-or-unrecognized-evidence'
    elif tl['samplesAtOrAboveFullScale'] or rl['samplesAtOrAboveFullScale']: reason = 'digital-full-scale-samples'
    elif min(tl['complexRms'], rl['complexRms']) < 1e-6: reason = 'insufficient-level'
    delay = {'validity': 'not-analyzed', 'reason': reason}
    if reason is None: delay = correlate(tx[:count], rf[:count], max_lag)
    return {'format': 'saturn-ps-timing-review-v1', 'sourcePin': meta.get('sourcePin'),
            'wdspArchiveSha256': meta.get('wdspArchiveSha256'), 'metadataSha256': meta_sha,
            'rawSha256': meta['rawSha256'], 'capturePath': str(path),
            'captureContext': {k:spectrum.get(k) for k in ('boot','backend','rateHz','generation','config','txEpoch','centerHz','commandedWatts','attenuationCommandDb','psRequested','stimulus','requestedStimulus')},
            'originalPairs': len(rows), 'analysisPairs': count,
            'arrivalFirstNs': str(rows[0][4]), 'arrivalLastNs': str(rows[-1][4]),
            'timestampDomain': meta['timeDomain'], 'reference': tl, 'rf': rl,
            'overload': spectrum.get('overload'), 'unclippedMeasurementQualified': False,
            'delay': delay, 'hardwarePeakRecommendation': None, 'recommendedHostSettings': None,
            'fittingAuthorized': False, 'rfImdQualified': False,
            'limitations': ['Returned reference peak is not a full-drive hardware-peak calibration.',
                'Unknown overload remains unknown; digital sample clipping is separate.',
                'One bounded window cannot establish delay repeatability or total uncertainty.',
                'No host configuration or TX authorization is generated.']}


def handoff_summary(reports):
    if not 1 <= len(reports) <= 32:
        raise ValueError('nonempty bounded review set required')
    if len({r['rawSha256'] for r in reports}) != len(reports):
        raise ValueError('duplicate captures cannot establish repeatability')
    identity = {(r['sourcePin'],r['wdspArchiveSha256']) for r in reports}
    if len(identity) != 1:
        raise ValueError('mixed source/native identities require separate reviews')
    usable = [r for r in reports if r['delay']['validity']=='unique-peak-in-search-not-qualified-delay']
    contexts = [r['captureContext'] for r in usable]
    settings_keys = ('boot','backend','rateHz','centerHz','commandedWatts','attenuationCommandDb')
    same_settings = (bool(contexts) and all(c.get(k) is not None for c in contexts for k in settings_keys)
        and len({json.dumps({k:c[k] for k in settings_keys},sort_keys=True) for c in contexts})==1
        and len({json.dumps({k:c.get(k) for k in ('stimulus','requestedStimulus')},sort_keys=True) for c in contexts})==1)
    times = sorted((int(r['arrivalFirstNs']),int(r['arrivalLastNs'])) for r in usable)
    distinct_intervals = all(a[1]<b[0] for a,b in zip(times,times[1:]))
    lags = [r['delay']['candidateRfLagSamples'] for r in usable]
    repeated_candidate = (len(lags)>=2 and same_settings and distinct_intervals and max(lags)-min(lags)<=1)
    requirements = []
    if not repeated_candidate:
        requirements.append('Obtain repeatable distinct timing-probe windows at unchanged settings; periodic/weak/boundary captures do not qualify.')
    requirements += [
        'Review total delay uncertainty, clock/path variation and matched-native signed-delay readback.',
        'Establish returned-reference HWPeak scaling against the digital transmit level; do not use RF-feedback peak or LP700 watts.',
        'Review path-specific adaptation level/continuity/overload policy; no synthetic-fixture thresholds become live defaults.',
        'Pin the actual candidate, backend, firmware, frequency, attenuation, drive ceiling and unchanged bench wiring.',
        'While RX and fully cleaned, separately authorize a reviewed host profile and single-calibration transaction.',
        'Use a fresh attended action for RF; a settings transaction must not key or reuse an old TX generation.',
        'Return to two-tone for the correction comparison; timing-probe spectra are not two-tone RF IMD.'
    ]
    return {'format':'saturn-ps-calibration-handoff-review-v1','status':'review-required',
        'captureSourcePin':reports[0]['sourcePin'],'wdspArchiveSha256':reports[0]['wdspArchiveSha256'],
        'windowsReviewed':len(reports),'timingCandidateWindows':len(usable),
        'repeatableIntegerCandidateWithinOneSample':repeated_candidate,
        'candidateLagSamples':lags if repeated_candidate else None,
        'overloadCoverage':[r['overload'] for r in reports],
        'requirementsBeforeSingleCalibration':requirements,
        'hostSettings':None,'fittingAuthorized':False,'rfKeyingAuthorized':False,
        'rfImdQualified':False,'reports':reports}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('metadata', nargs='+', type=Path)
    parser.add_argument('--max-lag', type=int, default=512)
    parser.add_argument('--pairs', type=int, default=4096)
    parser.add_argument('--handoff', action='store_true', help='emit a review-only handoff; never host settings or authorization')
    args = parser.parse_args()
    if len(args.metadata) > 32: parser.error('maximum 32 windows')
    try:
        result = [analyze(p, args.max_lag, args.pairs) for p in args.metadata]
        if args.handoff: result = handoff_summary(result)
    except (OSError, ValueError, KeyError, TypeError) as error: parser.error(str(error))
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == '__main__': main()
