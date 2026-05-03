import { describe, it, expect } from 'vitest';
import { bandKeyForFrequency, effectiveIqSampleRate } from '../src/radio/band';

describe('bandKeyForFrequency', () => {
  it('returns band label for frequency within a band', () => {
    expect(bandKeyForFrequency(7150000)).toBe('40m');
    expect(bandKeyForFrequency(14200000)).toBe('20m');
    expect(bandKeyForFrequency(1900000)).toBe('160m');
  });

  it('returns empty string for out-of-band frequency', () => {
    expect(bandKeyForFrequency(5000000)).toBe('');
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
