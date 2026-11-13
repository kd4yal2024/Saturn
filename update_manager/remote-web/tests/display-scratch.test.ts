import { describe, expect, it } from 'vitest';
import { visibleBinsForDisplay, smoothSpectrumTrace, smoothWaterfallBins } from '../src/dsp/display';
import { SpectrumHistory } from '../src/dsp/spectrum-history';

describe('display-owned scratch keeps numerical and retention contracts', () => {
  it('matches the owned path across FFT/zoom sizes without changing source or history', () => {
    const history = new SpectrumHistory();
    for (const size of [32, 4096, 16384]) for (const zoom of [1, 5, 1000]) {
      const input = Float32Array.from({ length: size }, (_, i) => -120 + 30 * Math.sin(i * .07));
      const saved = input.slice();
      const expected = visibleBinsForDisplay(input, zoom)!;
      const scratch = new Float32Array(expected.length);
      const output = visibleBinsForDisplay(input, zoom, scratch)!;
      expect(output).toBe(scratch);
      expect(output).toEqual(expected);
      expect(input).toEqual(saved);
      history.clear('test-size');
      history.accept({ bins: output, timestamp: 0, sequence: 1, receiver: 'rx:0', centerHz: 14200000,
        spanHz: 384000 / zoom, sampleRate: 384000, sourceBins: size, units: 'relative dB' }, 33);
      const retained = history.row(0)!.slice();
      visibleBinsForDisplay(new Float32Array(size).fill(-80), zoom, scratch);
      expect(history.row(0)).toEqual(retained);
      // Default callers, including RX Measure, still receive an owned result.
      expect(expected).toEqual(visibleBinsForDisplay(input, zoom));
    }
  });

  it('preserves both spatial smoothing and temporal waterfall history', () => {
    const input = new Float32Array([-130, -125, -80, -123, -120]);
    const previous = new Float32Array([-131, -124, -100, -121, -119]);
    const saved = previous.slice();
    for (const amount of [1, 50, 100]) {
      const scratch = new Float32Array(input.length);
      expect(smoothSpectrumTrace(input, amount, scratch)).toBe(scratch);
      expect(scratch).toEqual(smoothSpectrumTrace(input, amount));
      expect(smoothWaterfallBins(input, previous, amount, scratch)).toBe(scratch);
      expect(scratch).toEqual(smoothWaterfallBins(input, previous, amount));
      expect(previous).toEqual(saved);
    }
  });

  it('rejects overlapping storage and wrong-size output rather than corrupting inputs', () => {
    const input = new Float32Array([-130, -80, -120, -110]);
    const original = input.slice();
    const view = new Float32Array(input.buffer);
    expect(visibleBinsForDisplay(input, 1, view)).toEqual(visibleBinsForDisplay(input, 1));
    expect(smoothSpectrumTrace(input, 100, view)).toEqual(smoothSpectrumTrace(input, 100));
    const prev = new Float32Array([-129, -79, -119, -109]);
    const saved = prev.slice();
    expect(smoothWaterfallBins(input, prev, 100, prev)).toEqual(smoothWaterfallBins(input, prev, 100));
    expect(smoothSpectrumTrace(input, 100, new Float32Array(1))).toHaveLength(input.length);
    expect(prev).toEqual(saved);
    expect(input).toEqual(original);
  });
});
