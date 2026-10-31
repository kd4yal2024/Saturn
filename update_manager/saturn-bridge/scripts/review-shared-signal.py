"""Read-only shared-owner scaling evidence review. No radio/control/native access.

Reports retained levels, not a full-scale HWPeak calibration or RF IMD.
"""
import argparse
import hashlib
import json
import math
import re
from pathlib import Path
import struct


def checked(directory, stem, suffix, count, digest):
    path = directory / (stem + suffix)
    if count == 0 and not path.exists():
        data = b''
    else:
        if not path.is_file() or path.is_symlink() or path.stat().st_size != count:
            raise ValueError(f'missing/incorrect bounded payload: {path.name}')
        data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f'payload hash mismatch: {path.name}')
    return data


def level(values):
    if not values or len(values) % 2 or not all(math.isfinite(v) for v in values):
        raise ValueError('missing/nonfinite/unequal complex samples')
    power = [i*i + q*q for i, q in zip(values[::2], values[1::2])]
    return {'pairs': len(power), 'complexRms': math.sqrt(sum(power)/len(power)),
            'envelopePeak': math.sqrt(max(power)),
            'componentsAtFullScale': sum(abs(v) >= 1 for v in values)}


def review(directory, backend='DirectXdma'):
    context = json.loads((directory / 'context.json').read_text())
    if context.get('format') != 'saturn-shared-signal-context-v1':
        raise ValueError('unsupported capture context')
    paths = sorted(p for p in directory.glob('capture-*.json') if re.fullmatch(r'capture-[0-9]{4}\.json', p.name))
    if not 1 <= len(paths) <= 256:
        raise ValueError('nonempty bounded capture set required')
    if any(p.name != f'capture-{i:04d}.json' for i,p in enumerate(paths)):
        raise ValueError('missing capture sequence')
    summary = directory / 'capture-summary.json'
    complete = False
    if summary.exists():
        s = json.loads(summary.read_text())
        if s.get('exportErrors') != 0 or s.get('recordsConsumed') != len(paths) or s.get('drained') is not True:
            raise ValueError('incomplete/failed capture export')
        complete = True
    groups = {}
    stale_events = []
    for path in paths:
        if path.stat().st_size > 32000 or path.is_symlink():
            raise ValueError('invalid capture metadata')
        m = json.loads(path.read_text())
        if m.get('format') != 'saturn-shared-signal-capture-v1' or m.get('backend') != backend:
            raise ValueError('wrong capture format/backend')
        n, size, arrivals = m['values'], m['packedBytes'], m['arrivalCount']
        if not (0 <= n <= 8192 and 0 <= size <= 6144 and 0 <= arrivals <= 1024):
            raise ValueError('oversized capture')
        stem = path.stem
        data = checked(directory, stem, '.f64le', n*8, m['valuesSha256'])
        packed = checked(directory, stem, '.packed', size, m['packedSha256'])
        times = checked(directory, stem, '.arrival-u64le', arrivals*8, m['arrivalSha256'])
        values = list(struct.unpack('<' + 'd'*n, data))
        key = tuple(m[k] for k in ['boot', 'config', 'outputGeneration', 'outputRevision'])
        g = groups.setdefault(key, {'points': {}, 'rejectedWrites': 0, 'stimulus': m.get('stimulus')})
        if g['stimulus'] != m.get('stimulus'):
            raise ValueError('stimulus changed within a sealed arm')
        point = m['kind']
        levels = None
        if point == 'Stale':
            if len(values) != 16 or any(not math.isfinite(v) or v != int(v) or not 0 <= v < 2**32 for v in values):
                raise ValueError('invalid exact stale snapshot')
            words = [int(values[i]) | int(values[i+1]) << 32 for i in range(0,16,2)]
            origins = ['unknown','duration','source-stall','retained-mic','ingress','output','lifecycle']
            if words[0] >= len(origins): raise ValueError('unknown stale origin code')
            stale_events.append({'file':path.name,'identity':key,'generation':m['generation'],'txEpoch':m['txEpoch'],
                'origin':origins[words[0]],'snapshot':{name:None if i in (1,2,3) and v==2**64-1 else str(v)
                    for i,(name,v) in enumerate(zip(['nowNs','armedAtNs','lastArrivalNs','micOldestNs','txLimitNs','sourceLimitNs','micLimitNs'],words[1:]))}})
        elif point == 'Pcm':
            if len(values) != 512 or not all(math.isfinite(v) for v in values):
                raise ValueError('invalid PCM block')
            levels = {'samples': len(values), 'rms': math.sqrt(sum(v*v for v in values)/len(values)),
                      'peak': max(map(abs, values)), 'samplesAtFullScale': sum(abs(v) >= 1 for v in values)}
        elif point == 'Wdsp':
            levels = level(values)
        elif point == 'H2c':
            if size == 0 or size % 6 or m['submittedNs'] is None or m['completedNs'] < m['submittedNs']:
                raise ValueError('invalid write boundary')
            if m['completedBytes'] != m['requestedBytes'] or m['completedBytes'] != size:
                g['rejectedWrites'] += 1
                continue
            if m['keyedSnapshot'] is not True:
                continue  # preserve pre-key records; do not call them keyed RF output
            iq = []
            for p in range(0, size, 6):
                q = int.from_bytes(packed[p:p+3], 'big', signed=True)
                i = int.from_bytes(packed[p+3:p+6], 'big', signed=True)
                iq.extend([i/8388607, q/8388607])
            levels = level(iq)
        elif point == 'Pairs':
            if not n or n % 4 or (m['arrivalTimesRecorded'] and arrivals != n//4):
                raise ValueError('incomplete paired capture')
            if m['returnedIntervalEnd'] - m['firstReturnedPair'] != n//4:
                raise ValueError('incorrect paired interval')
            if times and min(struct.unpack('<'+'Q'*arrivals, times)) != m['sourceOldestNs']:
                raise ValueError('original arrival bound mismatch')
            levels = {'reference': level([v for p in range(0, n, 4) for v in values[p:p+2]]),
                      'rf': level([v for p in range(0, n, 4) for v in values[p+2:p+4]])}
        if levels is not None:
            g['points'].setdefault(point, []).append({'file': path.name, 'levels': levels,
                'sourceOldestNs': m['sourceOldestNs'], 'recordedNs': m['recordedNs'],
                'generation': m['generation'], 'txEpoch': m['txEpoch']})
    required = {'Pcm', 'Wdsp', 'Pairs'} | ({'H2c'} if backend == 'DirectXdma' else set())
    usable = []
    for identity, g in groups.items():
        if not required <= g['points'].keys() or g['rejectedWrites']:
            continue
        pcm_valid = (all(p['levels']['peak'] == 0 for p in g['points']['Pcm'])
                     if g['stimulus'] == 'wdsp-postgen-two-tone-v1'
                     else any(p['levels']['peak'] > .001 for p in g['points']['Pcm']))
        if not (pcm_valid
                and any(p['levels']['complexRms'] > .001 for p in g['points']['Wdsp'])
                and any(p['levels']['reference']['complexRms'] > .001
                        and p['levels']['rf']['complexRms'] > .001 for p in g['points']['Pairs'])):
            continue
        if backend == 'DirectXdma' and not any(p['levels']['complexRms'] > .001 for p in g['points']['H2c']):
            continue
        usable.append({'identity': identity, **g})
    if not usable:
        raise ValueError('no same-arm nonzero PCM/native/returned evidence with complete keyed output boundary')
    return {'format': 'saturn-shared-signal-review-v1', 'sourcePin': context['sourcePin'],
            'backend': backend, 'recordsChecked': len(paths), 'captureChainPresent': True, 'captureSetDrained': complete,
            'groups': usable, 'staleEvents':stale_events, 'sampleAligned': False, 'overloadQualification': 'unknown',
            'recommendedHwPeak': None, 'rfImdQualified': False,
            'limitations': ['Retained windows have different lengths/positions; not a sample-aligned gain calibration.',
                            'P2 socket output is not captured by this Direct-H2C diagnostic.',
                            'Known overload brackets remain in original metadata; no clear interval is manufactured.',
                            'A native/completed-byte capture does not prove physical RF output.']}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--backend', choices=['DirectXdma', 'P2'], default='DirectXdma')
    args = parser.parse_args()
    try:
        print(json.dumps(review(args.directory, args.backend), indent=2))
    except (ValueError, OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.exit(1, f'shared-signal review failed: {error}\n')
