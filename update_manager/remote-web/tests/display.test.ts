import { describe, expect, it } from 'vitest';
import {
  displaySpanHz,
  displayPercentForOffsetHz,
  frequencyScaleTicks,
  visibleBinsForDisplay,
  shiftBinsHorizontally,
  smoothSpectrumTrace,
  smoothWaterfallBins,
  bandEdgesInView,
  autoRangeFromBins,
} from '../src/dsp/display';

describe('displaySpanHz', () => {
  it('returns sampleRate at zoom 1', () => {
    expect(displaySpanHz(192000, 1)).toBe(192000);
  });
  it('divides by zoom', () => {
    expect(displaySpanHz(192000, 4)).toBe(48000);
  });
  it('clamps to minimum 1000', () => {
    expect(displaySpanHz(100, 1)).toBe(1000);
  });
});

describe('displayPercentForOffsetHz', () => {
  it('returns 50 for center', () => {
    expect(displayPercentForOffsetHz(0, 192000)).toBe(50);
  });
  it('returns 100 for right edge', () => {
    expect(displayPercentForOffsetHz(96000, 192000)).toBe(100);
  });
  it('returns 0 for left edge', () => {
    expect(displayPercentForOffsetHz(-96000, 192000)).toBe(0);
  });
});

describe('frequencyScaleTicks', () => {
  it('returns absolute frequencies across the visible span', () => {
    const ticks = frequencyScaleTicks(14_200_000, 192_000, 5);
    expect(ticks.map((tick) => tick.hz)).toEqual([
      14_104_000,
      14_152_000,
      14_200_000,
      14_248_000,
      14_296_000,
    ]);
    expect(ticks.map((tick) => tick.percent)).toEqual([0, 25, 50, 75, 100]);
  });
});

describe('visibleBinsForDisplay', () => {
  it('returns null for null input', () => {
    expect(visibleBinsForDisplay(null, 1)).toBeNull();
  });
  it('returns all bins at zoom 1', () => {
    const bins = new Float32Array([1, 2, 3, 4]);
    const result = visibleBinsForDisplay(bins, 1)!;
    expect(result.length).toBe(4);
    // reversed: [4, 3, 2, 1]
    expect(result[0]).toBe(4);
    expect(result[3]).toBe(1);
  });
  it('returns fewer bins at higher zoom', () => {
    const bins = new Float32Array(1024);
    const result = visibleBinsForDisplay(bins, 4)!;
    expect(result.length).toBe(256);
  });
});

describe('shiftBinsHorizontally', () => {
  it('returns same bins for 0 shift', () => {
    const bins = new Float32Array([1, 2, 3]);
    expect(shiftBinsHorizontally(bins, 0, -200)).toBe(bins);
  });
  it('shifts right', () => {
    const bins = new Float32Array([1, 2, 3, 4]);
    const result = shiftBinsHorizontally(bins, 2, -200)!;
    expect(result[0]).toBe(-200);
    expect(result[1]).toBe(-200);
    expect(result[2]).toBe(1);
    expect(result[3]).toBe(2);
  });
  it('shifts left', () => {
    const bins = new Float32Array([1, 2, 3, 4]);
    const result = shiftBinsHorizontally(bins, -2, -200)!;
    expect(result[0]).toBe(3);
    expect(result[1]).toBe(4);
    expect(result[2]).toBe(-200);
    expect(result[3]).toBe(-200);
  });
  it('fills entirely for shift >= length', () => {
    const bins = new Float32Array([1, 2, 3]);
    const result = shiftBinsHorizontally(bins, 5, -200)!;
    expect(Array.from(result)).toEqual([-200, -200, -200]);
  });
});

describe('smoothSpectrumTrace', () => {
  it('returns the original bins when smoothing is off', () => {
    const bins = new Float32Array([0, 10, 0]);
    expect(smoothSpectrumTrace(bins, 0)).toBe(bins);
  });

  it('softens isolated peaks without changing the bin count', () => {
    const bins = new Float32Array([0, 0, 12, 0, 0]);
    const result = smoothSpectrumTrace(bins, 100);
    expect(result).toHaveLength(bins.length);
    expect(result[2]).toBeLessThan(12);
    expect(result[1]).toBeGreaterThan(0);
    expect(result[3]).toBeGreaterThan(0);
  });
});

describe('smoothWaterfallBins', () => {
  it('leaves disabled cleanup unchanged', () => {
    const bins = new Float32Array([-120, -100]);
    expect(smoothWaterfallBins(bins, new Float32Array([-140, -140]), 0)).toBe(bins);
  });

  it('reduces frame-to-frame speckle and suppresses the measured background', () => {
    const previous = new Float32Array([-120, -120, -120, -120]);
    const current = new Float32Array([-100, -120, -140, -120]);
    const result = smoothWaterfallBins(current, previous, 100);
    expect(result).toHaveLength(current.length);
    expect(result[0]).toBeLessThan(-100);
    expect(result[1]).toBeLessThan(-120);
    expect(result[2]).toBeGreaterThan(-158);
  });

  it('protects persistent signals well above the estimated noise floor', () => {
    const bins = new Float32Array([-120, -121, -119, -120, -100]);
    const result = smoothWaterfallBins(bins, bins, 100);
    expect(result[0]).toBeLessThan(-130);
    expect(result[4]).toBeCloseTo(-100, 4);
  });
});

describe('bandEdgesInView', () => {
  const edges = [
    { start: 7000000, end: 7300000, label: '40m' },
    { start: 14000000, end: 14350000, label: '20m' },
  ];
  it('returns edges within span', () => {
    const markers = bandEdgesInView(7150000, 500000, edges);
    expect(markers.length).toBe(2); // 40m lo and 40m hi
    expect(markers[0]?.label).toBe('40m lo');
    expect(markers[1]?.label).toBe('40m hi');
  });
  it('returns empty when no edges in view', () => {
    const markers = bandEdgesInView(3500000, 100000, edges);
    expect(markers.length).toBe(0);
  });
});

describe('autoRangeFromBins', () => {
  it('smooths toward target range', () => {
    const bins = new Float32Array([-100, -80, -90, -85]);
    const result = autoRangeFromBins(bins, -200, -120, 0.15);
    expect(result.floor).toBeGreaterThan(-200);
    expect(result.ceiling).toBeGreaterThan(-120);
  });
  it('preserves current range for empty-like bins', () => {
    const bins = new Float32Array([Infinity, -Infinity]);
    const result = autoRangeFromBins(bins, -200, -120, 0.15);
    expect(result.floor).toBe(-200);
    expect(result.ceiling).toBe(-120);
  });
});
