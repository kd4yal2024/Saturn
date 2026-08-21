import { describe, expect, it } from 'vitest';
import {
  DEFAULT_SPECTRUM_WATERFALL_RATIO,
  MAX_SPECTRUM_WATERFALL_RATIO,
  MIN_SPECTRUM_WATERFALL_RATIO,
  clampSpectrumWaterfallRatio,
  nextPhoneSpectrumMode,
  normalizePhoneSpectrumMode,
  phoneSpectrumRatio,
  spectrumWaterfallRatioFromPointer,
} from '../src/ui/display-layout';

describe('display layout preferences', () => {
  it('normalizes and clamps persisted spectrum ratios', () => {
    expect(clampSpectrumWaterfallRatio('0.7')).toBe(0.7);
    expect(clampSpectrumWaterfallRatio(0)).toBe(MIN_SPECTRUM_WATERFALL_RATIO);
    expect(clampSpectrumWaterfallRatio(2)).toBe(MAX_SPECTRUM_WATERFALL_RATIO);
    expect(clampSpectrumWaterfallRatio('nope')).toBe(DEFAULT_SPECTRUM_WATERFALL_RATIO);
  });

  it('derives a safe ratio from a resize pointer', () => {
    expect(spectrumWaterfallRatioFromPointer(350, 100, 500)).toBe(0.5);
    expect(spectrumWaterfallRatioFromPointer(-10, 100, 500)).toBe(MIN_SPECTRUM_WATERFALL_RATIO);
    expect(spectrumWaterfallRatioFromPointer(100, 0, 0)).toBe(DEFAULT_SPECTRUM_WATERFALL_RATIO);
  });

  it('cycles the three intentional phone display modes', () => {
    expect(normalizePhoneSpectrumMode('unknown')).toBe('balanced');
    expect(nextPhoneSpectrumMode('balanced')).toBe('spectrum');
    expect(nextPhoneSpectrumMode('spectrum')).toBe('waterfall');
    expect(nextPhoneSpectrumMode('waterfall')).toBe('balanced');
    expect(phoneSpectrumRatio('spectrum')).toBeGreaterThan(phoneSpectrumRatio('balanced'));
    expect(phoneSpectrumRatio('waterfall')).toBeLessThan(phoneSpectrumRatio('balanced'));
  });
});
