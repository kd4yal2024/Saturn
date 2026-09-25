import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  SPECTRUM_DISPLAY_IQ_COMMAND,
  SPECTRUM_FFT_MAX,
  SPECTRUM_ROW_STREAM_TYPE,
  clampSpectrumFftSize,
  clampSpectrumIntervalMs,
  dequantizeSpectrumRow,
  parseDisplayEcho,
  parseSpectrumRowHeader,
  spectrumCapsAdvertised,
  spectrumDisplayAckCommand,
  spectrumDisplayCommand,
  spectrumRowSequence,
} from '../src/transport/spectrum-row';

type RowOptions = {
  binCount?: number;
  fftSize?: number;
  centerHz?: number;
  sequence?: number;
  spanHz?: number;
  format?: number;
  streamType?: number;
  flags?: number;
  channels?: number;
  dbOffset?: number;
  dbStep?: number;
  codes?: number[];
};

function buildRow(options: RowOptions = {}): ArrayBuffer {
  const binCount = options.binCount ?? 256;
  const centerHz = options.centerHz ?? 14_200_000;
  const buffer = new ArrayBuffer(64 + binCount);
  const view = new DataView(buffer);
  view.setUint32(0, 0, true);
  view.setUint32(4, options.spanHz ?? 384_000, true);
  view.setUint32(8, options.format ?? 0x5301, true);
  view.setUint32(12, options.fftSize ?? binCount, true);
  view.setUint32(16, options.flags ?? 1, true);
  view.setUint32(20, binCount, true);
  view.setUint32(24, options.streamType ?? SPECTRUM_ROW_STREAM_TYPE, true);
  view.setUint32(28, options.channels ?? 1, true);
  view.setUint32(32, options.sequence ?? 7, true);
  view.setUint32(36, centerHz >>> 0, true);
  view.setUint32(40, Math.floor(centerHz / 0x1_0000_0000), true);
  view.setFloat32(44, options.dbOffset ?? -160, true);
  view.setFloat32(48, options.dbStep ?? 0.625, true);
  view.setUint32(52, 0, true);
  view.setUint32(56, 0, true);
  view.setUint32(60, 0, true);
  if (options.codes) new Uint8Array(buffer, 64, binCount).set(options.codes);
  return buffer;
}

describe('spectrum display negotiation helpers', () => {
  it('rounds down to a power of two and clamps to the contract', () => {
    expect(clampSpectrumFftSize(2048)).toBe(2048);
    expect(clampSpectrumFftSize(3000)).toBe(2048);
    expect(clampSpectrumFftSize(300)).toBe(256);
    expect(clampSpectrumFftSize(100)).toBe(256);
    expect(clampSpectrumFftSize(16384)).toBe(SPECTRUM_FFT_MAX);
    expect(clampSpectrumFftSize(Number.NaN)).toBe(256);
  });

  it('clamps the interval to 33..250 ms', () => {
    expect(clampSpectrumIntervalMs(50)).toBe(50);
    expect(clampSpectrumIntervalMs(1)).toBe(33);
    expect(clampSpectrumIntervalMs(1000)).toBe(250);
    expect(clampSpectrumIntervalMs(Number.NaN)).toBe(250);
  });

  it('builds the frozen command strings', () => {
    expect(spectrumDisplayCommand(2048, 50)).toBe('saturn_display:spectrum,2048,50;');
    expect(spectrumDisplayCommand(16384, 1000)).toBe('saturn_display:spectrum,4096,250;');
    expect(SPECTRUM_DISPLAY_IQ_COMMAND).toBe('saturn_display:iq;');
    expect(spectrumDisplayAckCommand(42)).toBe('saturn_display_ack:42;');
    expect(spectrumDisplayAckCommand(-3)).toBe('saturn_display_ack:0;');
  });

  it('detects the spectrum_u8 capability token', () => {
    expect(spectrumCapsAdvertised('spectrum_u8')).toBe(true);
    expect(spectrumCapsAdvertised('pcm,spectrum_u8')).toBe(true);
    expect(spectrumCapsAdvertised(' spectrum_u8 ')).toBe(true);
    expect(spectrumCapsAdvertised('')).toBe(false);
    expect(spectrumCapsAdvertised(null)).toBe(false);
    expect(spectrumCapsAdvertised('spectrum_i16')).toBe(false);
  });

  it('parses both echo forms and rejects malformed ones', () => {
    expect(parseDisplayEcho(['0', 'iq'])).toEqual({ mode: 'iq' });
    expect(parseDisplayEcho(['0', 'spectrum', '2048', '50']))
      .toEqual({ mode: 'spectrum', fftSize: 2048, intervalMs: 50 });
    // Effective values are re-clamped, so a bad echo cannot widen the contract.
    expect(parseDisplayEcho(['0', 'spectrum', '99999', '5']))
      .toEqual({ mode: 'spectrum', fftSize: 4096, intervalMs: 33 });
    expect(parseDisplayEcho(['0'])).toBeNull();
    expect(parseDisplayEcho(['0', 'spectrum'])).toBeNull();
    expect(parseDisplayEcho([])).toBeNull();
  });
});

describe('spectrum row decoding', () => {
  it('parses the frozen header layout', () => {
    const header = parseSpectrumRowHeader(buildRow({ binCount: 256, sequence: 9, centerHz: 14_200_000 }));
    expect(header).not.toBeNull();
    expect(header?.streamType).toBe(SPECTRUM_ROW_STREAM_TYPE);
    expect(header?.fftSize).toBe(256);
    expect(header?.binCount).toBe(256);
    expect(header?.sequence).toBe(9);
    expect(header?.centerHz).toBe(14_200_000);
    expect(header?.spanHz).toBe(384_000);
    expect(header?.dbOffset).toBe(-160);
    expect(header?.dbStep).toBeCloseTo(0.625, 6);
  });

  it('dequantizes db = db_offset + code * db_step into a reused target', () => {
    const codes = new Array<number>(256).fill(0);
    codes[0] = 0;
    codes[1] = 1;
    codes[128] = 128;
    codes[255] = 255;
    const buffer = buildRow({ binCount: 256, codes });
    const header = parseSpectrumRowHeader(buffer);
    expect(header).not.toBeNull();
    const target = new Float32Array(256);
    const bins = dequantizeSpectrumRow(buffer, header!, target);
    expect(bins).toBe(target);
    expect(bins[0]).toBeCloseTo(-160, 5);
    expect(bins[1]).toBeCloseTo(-159.375, 5);
    expect(bins[128]).toBeCloseTo(-80, 5);
    expect(bins[255]).toBeCloseTo(-0.625, 5);
  });

  it('reads the sequence the credit window uses', () => {
    expect(spectrumRowSequence(buildRow({ sequence: 1234 }))).toBe(1234);
    expect(spectrumRowSequence(new ArrayBuffer(8))).toBeNull();
    const wrongType = buildRow({ streamType: 3 });
    expect(spectrumRowSequence(wrongType)).toBeNull();
  });

  it('rejects malformed frames instead of rendering them', () => {
    const cases: Array<[string, ArrayBuffer]> = [
      ['too short', new ArrayBuffer(32)],
      ['wrong stream type', buildRow({ streamType: 0 })],
      ['wrong format', buildRow({ format: 0x5300 })],
      ['non-power-of-two fft', buildRow({ fftSize: 300 })],
      ['fft out of range', buildRow({ fftSize: 8192, binCount: 256 })],
      ['bin count mismatch', buildRow({ fftSize: 256, binCount: 512 })],
      ['not fftshifted', buildRow({ flags: 0 })],
      ['non-positive db step', buildRow({ dbStep: 0 })],
    ];
    for (const [label, buffer] of cases) {
      expect(parseSpectrumRowHeader(buffer), label).toBeNull();
    }
  });

  it('rejects a truncated payload', () => {
    const full = buildRow({ binCount: 256 });
    const truncated = full.slice(0, 64 + 100);
    expect(parseSpectrumRowHeader(truncated)).toBeNull();
  });
});

describe('wire round trip with the bridge quantizer', () => {
  it('decodes a bridge-shaped 2048-bin row within half a quantization step', () => {
    // The bridge's Rust FFT reproduces fft.ts (see the parity fixture) and then
    // quantizes db with round((db + 160) / 0.625) clamped to u8. Rebuilding a
    // frame from the reference dB values exercises the exact bytes the browser
    // will receive for a real 384 kHz capture.
    const fixture = JSON.parse(
      readFileSync(
        new URL('../../saturn-bridge/src/testdata/fft_parity_2048.json', import.meta.url),
        'utf8',
      ),
    ) as { expectedDb: number[] };
    const codes = fixture.expectedDb.map((db) =>
      Math.max(0, Math.min(255, Math.round((db + 160) / 0.625))),
    );
    const frame = buildRow({ binCount: 2048, codes });
    const header = parseSpectrumRowHeader(frame);
    expect(header).not.toBeNull();
    const bins = dequantizeSpectrumRow(frame, header!, new Float32Array(2048));
    for (let i = 0; i < bins.length; i += 1) {
      expect(Math.abs(bins[i]! - fixture.expectedDb[i]!)).toBeLessThanOrEqual(0.625 / 2 + 1e-4);
    }
    const peak = Math.max(...bins);
    // The tone peak must survive quantization: within half a step of −15.15 dB.
    expect(peak).toBeGreaterThan(-16);
  });
});
