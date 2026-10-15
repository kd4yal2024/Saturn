"""Validate preserved SYNTHETIC replay capture JSON. No RF quality inference."""
import argparse
import json
from pathlib import Path
import struct


def verify(directory):
    expected_cases = {'expiry', 'release', 'disconnect', 'missing', 'queue', 'config', 'native', 'gap'}
    count = 0
    captures = 0
    for backend in ['P2', 'DirectXdma']:
        for case in sorted(expected_cases):
            path = directory / f'{backend}-{case}.json'
            record = json.loads(path.read_text())
            assert record['format'] == 'saturn-feedback-measurement-v1'
            assert record['rfImdQualified'] is False
            assert record['captureFinished'] is True
            assert isinstance(record['cleanupCompletedNs'], int)
            expected_fault = {'disconnect': 'TxAuthority', 'missing': 'Stale', 'queue': 'QueueOverrun',
                'native': 'NativeFailure', 'gap': 'SequenceGap' if backend == 'P2' else 'DmaFraming'}.get(case)
            assert record['terminalFault'] == expected_fault
            assert record['recordingComplete'] is (case in {'expiry', 'release', 'config'})
            assert record['retentionComplete'] is (case != 'queue')
            # This scenario intentionally never supplies feedback. Every other
            # scenario establishes paired acquisition before its stop/fault.
            # Overflow retains the first block AND the overflow-triggering one.
            blocks = record['captures']
            if not isinstance(blocks, list):
                raise ValueError(f'{path.name}: captures must be an array')
            if case == 'missing':
                if blocks:
                    raise ValueError(f'{path.name}: missing-feedback scenario requires zero captures')
            elif case == 'queue':
                if len(blocks) != 2:
                    raise ValueError(f'{path.name}: queue scenario requires exactly two captures')
            elif not blocks:
                raise ValueError(f'{path.name}: scenario requires at least one capture')
            for capture in record['captures']:
                assert capture['backend'] == backend
                assert capture['sourcePin'] == 'synthetic-offline-fixture-not-hardware'
                assert capture['nativeIdentity'].startswith('WDSP210 archive=8b75784e')
                assert capture['feedbackAttenuationCommandDb'] == 9
                assert capture['commandedDriveCode'] == 5
                assert capture['overloadQualification'] == 'Unknown'
                assert capture['overloadProvenUnclipped'] is False
                assert capture['lp700Measurement'] is None
                assert capture['native'] == {'fittingEnabled': False, 'automaticEnabled': False,
                                             'correctionApplied': False, 'verified': True}
                pairs = capture['pairCount']
                assert len(capture['referenceF64Bits']) == len(capture['rfF64Bits']) == 2*pairs
                assert len(capture['arrivalNs']) == pairs
                assert min(capture['arrivalNs']) == capture['oldestObservedNs']
                assert max(capture['arrivalNs']) <= capture['recordedNs']
                assert capture['interval']['endExclusive'] - capture['interval']['first'] == pairs
                assert capture['interval']['fence'] == capture['fence']
                assert capture['scaling']['converter'] == 'signed24/8388608'
                for bits in capture['referenceF64Bits'] + capture['rfF64Bits']:
                    value = struct.unpack('>d', bytes.fromhex(bits))[0]
                    assert -1 <= value < 1
                    assert value*8388608 == int(value*8388608)
                captures += 1
            count += 1
    assert len(list(directory.glob('*.json'))) == count
    return {'syntheticCaptureFilesVerified': count, 'pairedBlocksVerified': captures,
            'rfImdQualified': False, 'hardwareExecuted': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.directory), indent=2))


if __name__ == '__main__':
    main()
