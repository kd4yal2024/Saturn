import { describe, expect, it } from 'vitest';
import { clampFrequencyHz, digitStepForIndex, formatFrequencyHz, stepFrequencyDigit } from '../src/radio/frequency';

describe('frequency helpers', () => {
  it('formats Saturn readout frequencies', () => {
    expect(formatFrequencyHz(14_200_000)).toBe('14.200.000');
    expect(formatFrequencyHz(7050000)).toBe('7.050.000');
  });

  it('clamps frequencies to the supported range', () => {
    expect(clampFrequencyHz(-1)).toBe(0);
    expect(clampFrequencyHz(80_000_000)).toBe(64_000_000);
  });

  it('computes digit steps from padded positions', () => {
    expect(digitStepForIndex(8, 0)).toBe(10_000_000);
    expect(digitStepForIndex(8, 7)).toBe(1);
  });

  it('steps a digit and clamps the result', () => {
    expect(stepFrequencyDigit(14_200_000, 8, 4, 1)).toBe(14_201_000);
    expect(stepFrequencyDigit(0, 8, 0, -1)).toBe(0);
  });
});
