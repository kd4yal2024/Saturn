"""Offline conversion of existing paired captures; no devices, sockets or RF.

Keeps the complete original JSON beside bit-exact converted windows. Does not
invent pre-WDSP/output/key observations that the source does not contain.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct

UNKNOWN = (1 << 64) - 1


def integer(value, bits):
    if isinstance(value, bool) or not re.fullmatch(r'\d+', str(value)):
        raise ValueError('invalid unsigned metadata integer')
    number = int(value)
    if not 0 <= number < (1 << bits):
        raise ValueError('metadata integer out of range')
    return number


def windows(document):
    if document.get('format') != 'saturn-feedback-measurement-v1':
        raise ValueError('paired raw measurement required; summary-only is not raw evidence')
    captures = document.get('captures')
    if not isinstance(captures, list) or not 1 <= len(captures) <= 4097:
        raise ValueError('expected nonempty bounded paired captures')
    result = []
    for index, capture in enumerate(captures):
        backend = {'P2': 0, 'DirectXdma': 1}[capture['backend']]
        fence = capture['fence']
        boot = integer(fence['boot'], 128)
        generation = integer(fence['generation'], 64)
        config = integer(fence['config'], 128)
        epoch = integer(fence['txEpoch'], 64)
        count = integer(capture['pairCount'], 32)
        if not 1 <= count <= 1024 or capture['scaling'].get('converter') != 'signed24/8388608':
            raise ValueError('unsupported count or scaling')
        if capture['scaling'].get('normalizationApplied') is not False:
            raise ValueError('unknown normalization history')
        rate = integer(capture['rateHz'], 32)
        if rate == 0:
            raise ValueError('missing sample rate')
        first = integer(capture['oldestObservedNs'], 64)
        recorded = integer(capture['recordedNs'], 64)
        arrivals = capture.get('arrivalNs')
        if arrivals is not None:
            if len(arrivals) != count:
                raise ValueError('arrival coverage mismatch')
            arrivals = [integer(v, 64) for v in arrivals]
            if min(arrivals) != first or max(arrivals) > recorded:
                raise ValueError('invalid original arrival interval')
        # Missing individual arrivals do NOT certify a complete interval.
        last = UNKNOWN if arrivals is None else max(arrivals)
        attenuation = capture.get('feedbackAttenuationCommandDb')
        attenuation = 255 if attenuation is None else integer(attenuation, 8)
        if attenuation != 255 and attenuation > 31:
            raise ValueError('invalid attenuation command')
        # Converter intentionally cannot upgrade bracket validity. Keep unknown
        # even if an unverified JSON producer claimed clear/clipped evidence;
        # source bracket evidence/reasons remain in original-capture.json.
        for point, field in [(3, 'referenceF64Bits'), (4, 'rfF64Bits')]:
            bits = capture[field]
            if len(bits) != count * 2 or any(not re.fullmatch('[0-9a-fA-F]{16}', v) for v in bits):
                raise ValueError('invalid exact IQ bits')
            # The adapter exports are already divided by 8388608. Preserve
            # those exact values; do not divide them a second time.
            header = b'SATDIAG1' + bytes([point, backend, 0, 0, 0, 0, 2, attenuation])
            header += boot.to_bytes(16, 'little') + generation.to_bytes(8, 'little') + config.to_bytes(16, 'little')
            header += struct.pack('<6Q', epoch, first, last, recorded, UNKNOWN, UNKNOWN)
            header += struct.pack('<IId', rate, len(bits), 1.0)
            payload = b''.join(int(v, 16).to_bytes(8, 'little') for v in bits)
            result.append((f'capture-{index:04d}-{field}.satdiag', header + payload))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('new_directory', type=Path)
    args = parser.parse_args()
    if not args.source.is_file() or args.source.stat().st_size > 100_000_000:
        parser.error('bounded ordinary JSON input required')
    original = args.source.read_bytes()
    converted = windows(json.loads(original))
    args.new_directory.mkdir(mode=0o700)  # never overwrite another run
    (args.new_directory / 'original-capture.json').write_bytes(original)
    for name, data in converted:
        (args.new_directory / name).write_bytes(data)
    manifest = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(args.new_directory.iterdir())}
    (args.new_directory / 'sha256.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({'pairedBlocksConverted': len(converted) // 2,
                      'originalEvidencePreserved': True, 'overloadQualification': 'unknown',
                      'rfImdQualified': False}))


if __name__ == '__main__':
    main()
