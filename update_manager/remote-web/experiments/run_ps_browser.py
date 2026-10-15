#!/usr/bin/env python3
"""Loopback-only real Chrome experiment. No hardware/service/deployment access.

Dependencies (install into an isolated tool directory): playwright, numpy, scipy.
Requires an installed Chrome and a local recorded speech WAV; never uploads it.
Writes only to a new explicitly supplied result directory.
"""
import argparse
import base64
import ctypes
import datetime
import functools
import hashlib
import http.server
import json
import pathlib
import struct
import subprocess
import threading
import time

import numpy as np
from scipy import signal
from scipy.io import wavfile
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[1]
RATE = 48000


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def make_input(source, output):
    rate, speech = wavfile.read(source)
    if np.issubdtype(speech.dtype, np.integer):
        speech = speech.astype(np.float64) / (np.iinfo(speech.dtype).max + 1)
    if speech.ndim == 2:
        speech = speech.mean(axis=1)
    if rate != RATE:
        from math import gcd
        divisor = gcd(rate, RATE)
        speech = signal.resample_poly(speech, RATE // divisor, rate // divisor)
    if len(speech) < RATE * 9.6 or not np.isfinite(speech).all():
        raise ValueError('at least 9.6 seconds finite recorded speech required')
    speech = speech[:int(RATE * 9.6)]
    speech *= 0.3 / max(float(np.max(np.abs(speech))), 1e-8)
    t = np.arange(int(RATE * 4.8)) / RATE
    tones = 0.15 * (np.sin(2 * np.pi * 700 * t) + np.sin(2 * np.pi * 1900 * t))
    samples = np.concatenate([speech, tones]).astype('<f4')
    assert len(samples) % 512 == len(samples) % 960 == 0
    samples.tofile(output / 'input.f32')
    wavfile.write(output / 'input.wav', RATE, samples)
    write_json(output / 'input-provenance.json', {
        'source': str(source), 'sourceSha256': hashlib.sha256(source.read_bytes()).hexdigest(),
        'scope': 'local user recording, no external upload; no redistribution license asserted',
        'transformation': 'first 9.6 s, mono, resample if necessary, peak normalized to 0.3; then 4.8 s equal 700/1900 Hz tones',
        'sampleRateHz': RATE, 'samples': len(samples),
        'inputSha256': hashlib.sha256(samples.tobytes()).hexdigest(),
    })
    return samples


class Opus:
    def __init__(self):
        self.lib = ctypes.CDLL('libopus.so.0')
        self.lib.opus_decoder_create.argtypes = [ctypes.c_int32, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
        self.lib.opus_decoder_create.restype = ctypes.c_void_p
        self.lib.opus_decoder_destroy.argtypes = [ctypes.c_void_p]
        self.lib.opus_decode_float.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int32,
                                             ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
        self.lib.opus_decode_float.restype = ctypes.c_int
        self.lib.opus_get_version_string.restype = ctypes.c_char_p

    def decode(self, packets):
        result = []
        bank = None
        decoder = None
        try:
            for p in packets:
                if p['bank'] != bank:
                    if decoder:
                        self.lib.opus_decoder_destroy(decoder)
                    error = ctypes.c_int()
                    decoder = self.lib.opus_decoder_create(RATE, 1, ctypes.byref(error))
                    if not decoder or error.value:
                        raise ValueError('decoder creation failed')
                    bank = p['bank']
                payload = base64.b64decode(p['bytes'])
                buffer = ctypes.create_string_buffer(payload)
                audio = np.empty(5760, dtype=np.float32)
                count = self.lib.opus_decode_float(decoder, buffer, len(payload), audio.ctypes.data, 5760, 0)
                if count < 0:
                    raise ValueError(f'opus decode error {count}')
                p['decodedSamples'] = count
                result.append(audio[:count].copy())
        finally:
            if decoder:
                self.lib.opus_decoder_destroy(decoder)
        return result


def stats(values):
    values = [v for v in values if v is not None]
    return ({'count': len(values), 'p50': float(np.percentile(values, 50)),
             'p95': float(np.percentile(values, 95)), 'max': float(max(values))} if values else None)


def analyze(result, source, output):
    mode = result['mode']
    captured = np.frombuffer(base64.b64decode(result['captured']), dtype='<f4')
    wavfile.write(output / f'{mode}-captured.wav', RATE, captured)
    packets = result['packets']
    if mode == 'pcm':
        chunks = [np.frombuffer(base64.b64decode(p['bytes'])[64:], dtype='<i2').astype(np.float32) / 32768 for p in packets]
    else:
        codec = Opus()
        chunks = codec.decode(packets)
        result['decoderIdentity'] = codec.lib.opus_get_version_string().decode()
    raw = np.concatenate(chunks) if chunks else np.array([], dtype=np.float32)
    wavfile.write(output / f'{mode}-all-decoded.wav', RATE, raw)
    raw.astype('<f4').tofile(output / f'{mode}-all-decoded.f32')
    # Frozen actual browser payload corpus for the Bridge's own decoder replay.
    with (output / f'{mode}-bridge-corpus.bin').open('xb') as stream:
        for index, p in enumerate(packets):
            payload = base64.b64decode(p['bytes'])
            if mode == 'pcm':
                frame = payload
            else:
                header = bytearray(64)
                for offset, value in [(0, 0), (4, RATE), (8, 1), (20, 960),
                                      (24, 2), (28, 1), (32, index), (36, 2), (40, len(payload))]:
                    struct.pack_into('<I', header, offset, value)
                frame = bytes(header) + payload
            stream.write(struct.pack('<II', p['bank'], len(frame)) + frame)
    # Diagnostic only: compare exactly four nominal packets per bank. Preserve
    # ALL flush output above. This is NOT a proposed silent production fix.
    kept = [audio for p, audio in zip(packets, chunks) if mode != 'banked' or p['submittedSequence'] is not None]
    decoded = np.concatenate(kept) if kept else np.array([], dtype=np.float32)
    wavfile.write(output / f'{mode}-nominal-decoded.wav', RATE, decoded)
    # Delay estimated by cross correlation. Report without hiding count errors.
    n = min(len(source), len(decoded))
    reference, observed = captured[:n], decoded[:n]
    corr = signal.correlate(observed, reference, mode='full', method='fft')
    lags = signal.correlation_lags(n, n)
    valid = np.abs(lags) <= 960
    lag = int(lags[valid][np.argmax(corr[valid])])
    if lag >= 0:
        ref, obs = reference[:n-lag], observed[lag:]
    else:
        ref, obs = reference[-lag:], observed[:n+lag]
    rms = lambda x: float(np.sqrt(np.mean(np.square(x))))
    residual = rms(obs - ref)
    boundary = np.arange(3840, len(decoded), 3840)
    jumps = np.abs(decoded[boundary] - decoded[boundary-1])
    # Tone-only interval avoids treating speech as two fundamentals or IMD.
    tone = decoded[int(10 * RATE):min(len(decoded), int(14 * RATE))]
    freqs, psd = signal.welch(tone, RATE, nperseg=4800)
    np.savetxt(output / f'{mode}-tone-spectrum.csv', np.column_stack([freqs, psd]),
               delimiter=',', header='frequency_hz,power_fs_squared_per_hz', comments='')
    main = ((np.abs(freqs-700) <= 30) | (np.abs(freqs-1900) <= 30))
    band = (freqs >= 100) & (freqs <= 4000)
    leak = float(10 * np.log10(max(float(psd[band & ~main].sum()), 1e-30) / max(float(psd[main].sum()), 1e-30)))
    summary = {
        'inputSamples': len(source), 'captureSamples': result['capture']['blocks'] * (512 if mode == 'pcm' else 960),
        'packets': len(packets), 'rawDecodedSamples': len(raw), 'nominalDecodedSamples': len(decoded),
        'flushExtraPackets': sum(p.get('submittedSequence') is None for p in packets),
        'durationUs': stats(p['durationUs'] for p in packets),
        'encodeCallbackMs': stats(p['encodeMs'] for p in packets),
        'workletOldestToMainMs': stats(result['capture']['workletToMainMs']),
        'globalCorrelationDelaySamples': lag, 'globalCorrelationDelayMs': lag / 48,
        'alignedErrorRmsFs': residual, 'alignedSnrDb': 20 * np.log10(max(rms(ref), 1e-30) / max(residual, 1e-30)),
        '80msBoundaryAbsoluteJumpFs': stats(jumps),
        '80msInputBoundaryAbsoluteJumpFs': stats(np.abs(captured[boundary[boundary < len(captured)]] - captured[boundary[boundary < len(captured)]-1])),
        'toneOutOfFundamentalBandDb': leak,
        'cpu': result['cpu'], 'errors': result['errors'], 'unmatchedInput': result['unmatchedInput'],
        'nominalWarning': 'banked nominal audio omits measured extra flush packets for diagnosis only, never managed transport fallback',
    }
    write_json(output / f'{mode}-summary.json', summary)
    return summary


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--speech', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--bridge', help='explicit offline loopback replay Bridge only')
    parser.add_argument('--lifecycle-pcm-only', action='store_true',
                        help='cancellation correction: PCM release/rearm, active disconnect, reconnect; no Opus experiment')
    args = parser.parse_args()
    if args.lifecycle_pcm_only and not args.bridge:
        parser.error('--lifecycle-pcm-only requires an explicit loopback replay Bridge')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    sources = [ROOT / 'experiments/ps-browser.ts', pathlib.Path(__file__).resolve(),
               ROOT / 'src/puresignal/browser-capture-review.ts',
               ROOT / 'src/transport/tx-generation-review.ts',
               ROOT / 'src/audio/tx-opus-encoder.ts', ROOT / 'src/transport/tx-uplink.ts']
    write_json(output / 'run-metadata.json', {
        'startedAtIso': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'sourceSha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources},
        'argv': vars(args) | {'speech': str(args.speech), 'output': str(args.output)},
        'experiment': 'offline browser transport only, no radio discovery or RF',
    })
    source = make_input(args.speech.resolve(), output)
    subprocess.run(['node', '-e', '''require('esbuild').buildSync({entryPoints:[process.argv[1]],bundle:true,format:'iife',outfile:process.argv[2]})''',
                    str(ROOT / 'experiments/ps-browser.ts'), str(output / 'experiment.js')], cwd=ROOT, check=True)
    (output / 'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Offline PS — no RF</title><script src="experiment.js"></script>')
    httpd = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(QuietHandler, directory=str(output)))
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    summaries = {}
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(executable_path='/usr/bin/google-chrome', headless=True,
                args=['--no-sandbox', '--autoplay-policy=no-user-gesture-required', '--disable-background-timer-throttling'])
            root_session = browser.new_browser_cdp_session()
            def cpu():
                return sum(v['cpuTime'] for v in root_session.send('SystemInfo.getProcessInfo')['processInfo'])
            write_json(output / 'browser-identity.json', {'version': browser.version,
                'system': root_session.send('SystemInfo.getInfo'),
                'execution': 'Linux headless real Chrome, real AudioWorklet and WebCodecs; recorded MediaStream, no physical microphone or RF'})
            if args.bridge:
                page = browser.new_page()
                page.goto(f'http://127.0.0.1:{httpd.server_port}/')
                results = []
                cases = [('pcm', 0)] * 3 if args.lifecycle_pcm_only else [('pcm', 0), ('opus_wb', 0), ('opus_wb', 500)]
                for index, (codec, delay) in enumerate(cases):
                    # Entire new control/media connection pair each case:
                    # reconnect is actual WS transport, not a parser shim.
                    old = results[0]['sent'][0] if results else None
                    try:
                        active_disconnect = args.lifecycle_pcm_only and index == 1
                        results.append(page.evaluate('args => psExperiment.managed(...args)', [args.bridge, '/input.f32', codec, delay, old, active_disconnect]))
                    except Exception as error:
                        write_json(output / 'managed-browser-failure.json', {'codec': codec,
                            'delayMs': delay, 'error': str(error), 'completedCases': len(results)})
                        raise
                    finally:
                        write_json(output / 'managed-browser.json', results)
                page.close()
            else:
                for mode in ['pcm', 'continuous', 'banked']:
                    page = browser.new_page()
                    page.goto(f'http://127.0.0.1:{httpd.server_port}/')
                    session = browser.new_browser_cdp_session()
                    start_cpu, started = cpu(), time.monotonic()
                    result = page.evaluate('args => psExperiment.compare(...args)', [mode, '/input.f32'])
                    wall = time.monotonic() - started
                    result['cpu'] = {'chromeAggregateCpuSeconds': cpu() - start_cpu,
                        'wallSeconds': wall, 'scope': 'all Chrome processes, includes audio graph/encoder; not isolated encoder CPU'}
                    result['cpu']['oneCorePercent'] = result['cpu']['chromeAggregateCpuSeconds'] / wall * 100
                    write_json(output / f'{mode}-browser.json', result)
                    summaries[mode] = analyze(result, source, output)
                    page.close(); session.detach()
                write_json(output / 'comparison.json', summaries)
            browser.close()
    finally:
        httpd.shutdown(); httpd.server_close(); thread.join()
    print(f'Browser experiment retained: {output}', flush=True)


if __name__ == '__main__':
    main()
