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
      expect(result[i]).toBeLessThan(0);
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
    // Run a few times to let smoothing settle
    let result = fft.transform(iq);
    result = fft.transform(iq);
    result = fft.transform(iq);
    const max = Math.max(...result);
    const min = Math.min(...result);
    expect(max).toBeGreaterThan(min + 10);
  });

  it('resetSmoothing clears previous bins', () => {
    const fft = new FftProcessor(64);
    const iq = new Float32Array(128);
    for (let i = 0; i < 128; i++) iq[i] = 1.0;
    fft.transform(iq);
    fft.resetSmoothing();
    const silence = new Float32Array(128);
    const result = fft.transform(silence);
    // After reset, should not carry previous high values
    for (let i = 0; i < result.length; i++) {
      expect(result[i]).toBeLessThan(-20);
    }
  });
});
