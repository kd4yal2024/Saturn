import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { FftProcessor } from '../src/dsp/fft';

type ParityFixture = {
  generator: string;
  fftSize: number;
  sampleRateHz: number;
  dbOffset: number;
  inputIqFloat32: number[];
  expectedDb: number[];
};

const fixture = JSON.parse(
  readFileSync(
    new URL('../../saturn-bridge/src/testdata/fft_parity_2048.json', import.meta.url),
    'utf8',
  ),
) as ParityFixture;

describe('Rust/browser FFT parity fixture', () => {
  it('holds exactly one fft_size of contiguous input pairs', () => {
    expect(fixture.fftSize).toBe(2048);
    expect(fixture.sampleRateHz).toBe(384_000);
    expect(fixture.inputIqFloat32).toHaveLength(fixture.fftSize * 2);
    expect(fixture.expectedDb).toHaveLength(fixture.fftSize);
  });

  it('matches the real fft.ts output, so the bridge test is meaningful', () => {
    const iq = Float32Array.from(fixture.inputIqFloat32);
    const bins = new FftProcessor(fixture.fftSize).transform(iq);
    let worst = 0;
    for (let i = 0; i < bins.length; i += 1) {
      worst = Math.max(worst, Math.abs(bins[i]! - fixture.expectedDb[i]!));
    }
    expect(worst).toBeLessThanOrEqual(0.01);
  });

  it('is byte-stable: regenerate after any fft.ts change', () => {
    const iq = Float32Array.from(fixture.inputIqFloat32);
    const bins = new FftProcessor(fixture.fftSize).transform(iq);
    const regenerated = Array.from(bins, (value) => Number(value.toFixed(6)));
    expect(regenerated).toEqual(fixture.expectedDb);
  });
});
