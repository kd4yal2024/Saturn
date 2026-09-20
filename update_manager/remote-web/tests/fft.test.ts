import { describe, it, expect } from 'vitest';
import { FftProcessor } from '../src/dsp/fft';

describe('FftProcessor', () => {
  it('creates with given size', () => {
    const fft = new FftProcessor(256);
    expect(fft.size).toBe(256);
  });

  it('transform returns Float32Array of correct size', () => {
    const fft = new FftProcessor(64);
    const iq = new Float32Array(128); // 64 complex pairs
    const result = fft.transform(iq);
    expect(result.length).toBe(64);
  });

  it('transform produces dB values for silence near noise floor', () => {
    const fft = new FftProcessor(64);
    const iq = new Float32Array(128);
    const result = fft.transform(iq);
    for (let i = 0; i < result.length; i += 1) {
      expect(result[i]).toBeCloseTo(-160, 4);
    }
  });

  it('transform produces higher values for a tone', () => {
    const size = 256;
    const fft = new FftProcessor(size);
    const iq = new Float32Array(size * 2);
    for (let i = 0; i < size; i += 1) {
      const phase = (2 * Math.PI * i * 10) / size;
      iq[i * 2] = Math.cos(phase);
      iq[i * 2 + 1] = Math.sin(phase);
    }
    const result = fft.transform(iq);
    const max = Math.max(...result);
    const min = Math.min(...result);
    expect(max).toBeGreaterThan(min + 10);
    expect(result[size / 2 + 10]).toBeCloseTo(20 * Math.log10((size - 1) / (2 * size)), 3);
    expect(fft.transform(iq)).toEqual(result);
  });

  it('follows a tone-to-silence transition immediately without hidden averaging', () => {
    const fft = new FftProcessor(64);
    const iq = new Float32Array(128);
    for (let i = 0; i < 128; i++) iq[i] = 1.0;
    fft.transform(iq);
    const silence = new Float32Array(128);
    const result = fft.transform(silence);
    for (let i = 0; i < result.length; i++) {
      expect(result[i]).toBeCloseTo(-160, 4);
    }
  });
});
