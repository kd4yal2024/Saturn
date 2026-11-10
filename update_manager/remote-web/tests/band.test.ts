import { describe, it, expect } from 'vitest';
import { bandKeyForFrequency, effectiveIqSampleRate, SWL_BANDS } from '../src/radio/band';

describe('bandKeyForFrequency', () => {
  it('returns band label for frequency within a band', () => {
    expect(bandKeyForFrequency(7150000)).toBe('40m');
    expect(bandKeyForFrequency(14200000)).toBe('20m');
    expect(bandKeyForFrequency(1900000)).toBe('160m');
    expect(bandKeyForFrequency(99_500_000)).toBe('FM');
  });

  it('returns empty string for out-of-band frequency', () => {
    expect(bandKeyForFrequency(5500000)).toBe('');
    expect(bandKeyForFrequency(0)).toBe('');
  });

  it('returns band label at band edges', () => {
    expect(bandKeyForFrequency(7000000)).toBe('40m');
    expect(bandKeyForFrequency(7300000)).toBe('40m');
  });
});

describe('effectiveIqSampleRate', () => {
  it('returns full rate on LAN', () => {
    expect(effectiveIqSampleRate(192000, 'lan')).toBe(192000);
  });

  it('caps rate on WAN', () => {
    expect(effectiveIqSampleRate(192000, 'wan')).toBe(96000);
    expect(effectiveIqSampleRate(48000, 'wan')).toBe(48000);
  });
});

// Actual broadcast and Ham bands with the same meter names must not collide.
describe('SWL band navigation', () => {
  it('recognizes every preset under a separate memory key', () => {
    for (const b of SWL_BANDS) {
      expect(b.frequency).toBeGreaterThanOrEqual(b.start);
      expect(b.frequency).toBeLessThanOrEqual(b.end);
      expect(bandKeyForFrequency(b.frequency, 'swl')).toBe(`SWL ${b.label}`);
    }
  });
  it('keeps overlapping ranges and reused names distinct', () => {
    expect(bandKeyForFrequency(3_950_000, 'ham')).toBe('80m');
    expect(bandKeyForFrequency(3_950_000, 'swl')).toBe('SWL 75m');
    expect(bandKeyForFrequency(7_250_000, 'ham')).toBe('40m');
    expect(bandKeyForFrequency(7_250_000, 'swl')).toBe('SWL 41m');
    expect(bandKeyForFrequency(5_357_500)).toBe('60m');
    expect(bandKeyForFrequency(4_850_000)).toBe('SWL 60m');
    expect(bandKeyForFrequency(21_300_000)).toBe('15m');
    expect(bandKeyForFrequency(18_950_000)).toBe('SWL 15m');
  });
});
