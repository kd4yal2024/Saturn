import { describe, it, expect } from 'vitest';
import { anrPreset, anfPreset } from '../src/radio/dsp-presets';
import {
  defaultSignedRxPassbandForMode,
  defaultSignedTxPassbandForMode,
  defaultFilterCutsForMode,
} from '../src/radio/passband';
import { formatFrequencyMarkup } from '../src/radio/frequency';

describe('anrPreset', () => {
  it('returns wide preset', () => {
    const p = anrPreset('wide');
    expect(p.rxAnrTaps).toBe(96);
    expect(p.rxAnrDelay).toBe(24);
  });

  it('returns thetis (default) preset', () => {
    const p = anrPreset('thetis');
    expect(p.rxAnrTaps).toBe(64);
    expect(p.rxAnrDelay).toBe(16);
  });
});

describe('anfPreset', () => {
  it('returns sharp preset', () => {
    const p = anfPreset('sharp');
    expect(p.rxAnfTaps).toBe(96);
    expect(p.rxAnfDelay).toBe(12);
  });

  it('returns default preset', () => {
    const p = anfPreset('default');
    expect(p.rxAnfTaps).toBe(64);
  });
});

describe('defaultSignedRxPassbandForMode', () => {
  it('returns positive passband for USB', () => {
    expect(defaultSignedRxPassbandForMode('USB')).toEqual({ lowHz: 50, highHz: 3050 });
  });

  it('returns negative passband for LSB', () => {
    expect(defaultSignedRxPassbandForMode('LSB')).toEqual({ lowHz: -3000, highHz: -300 });
  });

  it('returns narrow CW passband', () => {
    expect(defaultSignedRxPassbandForMode('CWU')).toEqual({ lowHz: 200, highHz: 800 });
  });

  it('returns symmetric passband for AM', () => {
    const pb = defaultSignedRxPassbandForMode('AM');
    expect(pb.lowHz).toBeLessThan(0);
    expect(pb.highHz).toBeGreaterThan(0);
  });
});

describe('defaultSignedTxPassbandForMode', () => {
  it('returns USB tx defaults', () => {
    expect(defaultSignedTxPassbandForMode('USB')).toEqual({ lowHz: 50, highHz: 3050 });
  });

  it('returns DIGL tx defaults', () => {
    expect(defaultSignedTxPassbandForMode('DIGL')).toEqual({ lowHz: -3000, highHz: 0 });
  });
});

describe('defaultFilterCutsForMode', () => {
  it('returns UI cuts for USB', () => {
    const cuts = defaultFilterCutsForMode('USB');
    expect(cuts.rxLow).toBe(50);
    expect(cuts.rxHigh).toBe(3050);
    expect(cuts.txLow).toBe(50);
    expect(cuts.txHigh).toBe(3050);
  });

  it('returns UI cuts for LSB', () => {
    const cuts = defaultFilterCutsForMode('LSB');
    expect(cuts.rxLow).toBeGreaterThanOrEqual(0);
    expect(cuts.rxHigh).toBeGreaterThan(cuts.rxLow);
  });
});

describe('formatFrequencyMarkup', () => {
  it('produces digit spans with separators', () => {
    const html = formatFrequencyMarkup(14200000);
    expect(html).toContain('freq-digit');
    expect(html).toContain('freq-separator');
    expect(html).toContain('data-step=');
  });

  it('handles zero', () => {
    const html = formatFrequencyMarkup(0);
    expect(html).toContain('0');
  });
});
