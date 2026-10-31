"""Listen to a retained pre-WDSP PCM capture. Never flatten complex IQ."""
import argparse
from pathlib import Path
import struct
import math
import wave


def pcm(data):
    if len(data) < 120 or data[:8] != b'SATDIAG1' or data[8] != 0 or data[10] != 0:
        raise ValueError('a pre-WDSP real PCM sample capture is required, not IQ/events')
    rate, count, scale = struct.unpack_from('<IId', data, 104)
    if not 1 <= rate <= 384000 or not 1 <= count <= 16384 or len(data) != 120 + 8 * count:
        raise ValueError('invalid PCM rate/count')
    values = struct.unpack_from(f'<{count}d', data, 120)
    if not math.isfinite(scale) or scale <= 0 or any(not math.isfinite(v) for v in values):
        raise ValueError('nonfinite or invalid PCM scaling; retain original evidence')
    normalized = [v / scale for v in values]
    if any(abs(v) > 1 for v in normalized):
        raise ValueError('clipped listening export refused; original PCM remains available')
    encoded = b''.join(struct.pack('<h', round(v * 32767)) for v in normalized)
    return rate, encoded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    parser.add_argument('new_wav', type=Path)
    args = parser.parse_args()
    if not args.capture.is_file() or args.capture.stat().st_size > 120 + 16384 * 8:
        parser.error('bounded ordinary capture required')
    rate, data = pcm(args.capture.read_bytes())
    with args.new_wav.open('xb') as output, wave.open(output, 'wb') as wav:
        wav.setnchannels(1); wav.setsampwidth(2); wav.setframerate(rate); wav.writeframes(data)
    print('PCM16 listening copy only; original f64 capture and scaling unchanged')


if __name__ == '__main__':
    main()
