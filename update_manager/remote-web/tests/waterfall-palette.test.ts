import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';
import { clampWaterfallContrast, normalizeDisplayPrefs, normalizeWaterfallPalette } from '../src/settings/normalize';
import { smoothWaterfallBins } from '../src/dsp/display';

const template = readFileSync('../templates/saturn-remote-next.html', 'utf8');
const start = template.indexOf('    class WaterfallRenderer {');
const end = template.indexOf('    const spectrumRenderer =', start);
const renderer = runInNewContext(`${template.slice(start, end)}\nWaterfallRenderer`, {
  normalizeWaterfallPalette, clampWaterfallContrast,
});
const color = (level: number, floor = 0, ceiling = 1, contrast = 100) =>
  renderer.prototype.colorForDb(level, floor, ceiling, 'enhanced', contrast);

describe('Enhanced waterfall palette', () => {
  it('keeps colors unchanged after repeated cleanup of a steady waterfall', () => {
    const bins = new Float32Array([-130, -125, -120, -115, -110, -100, -90, -80]);
    const expected = Array.from(bins, level => color(level, -130, -80));
    let history: Float32Array = bins;
    for (let frame = 0; frame < 1000; frame++) history = smoothWaterfallBins(bins, history, 100);
    expect(Array.from(history, level => color(level, -130, -80))).toEqual(expected);
  });
  const stops = [
    [0, [0, 0, 0]], [2 / 9, [0, 0, 255]], [3 / 9, [0, 255, 255]],
    [4 / 9, [0, 255, 0]], [5 / 9, [255, 255, 0]], [7 / 9, [255, 0, 0]],
    [8 / 9, [255, 0, 255]], [1, [192, 124, 255]],
  ] as const;

  it.each(stops)('maps level %s to its reference color', (level, expected) => {
    expect(color(level)).toEqual(expected);
  });

  it('is continuous at all breakpoints, including the saturation endpoint', () => {
    for (const [level] of stops) {
      const before = color(level - 1e-7);
      const after = color(level + 1e-7);
      expect(before.every((channel: number, i: number) => Math.abs(channel - after[i]) <= 1)).toBe(true);
    }
  });

  it('interpolates weak signals and clamps out-of-range samples', () => {
    expect(color(1 / 9)).toEqual([0, 0, 128]);
    expect(color(-10)).toEqual([0, 0, 0]);
    expect(color(10)).toEqual([192, 124, 255]);
    expect(color(NaN)).toEqual([0, 0, 0]);
    expect(color(0, 0, 0)).toEqual([0, 0, 0]);
  });

  it('uses existing floor, ceiling and contrast rather than hard-coded RF levels', () => {
    expect(color(-120, -130, -85)).toEqual([0, 0, 255]);
    expect(color(0.25, 0, 1, 200)).toEqual([0, 0, 0]);
    expect(color(0.75, 0, 1, 200)).toEqual([192, 124, 255]);
  });

  it('survives settings serialization without changing existing defaults', () => {
    const prefs = normalizeDisplayPrefs({ waterfallPalette: 'enhanced' });
    expect(normalizeDisplayPrefs(JSON.parse(JSON.stringify(prefs))).waterfallPalette).toBe('enhanced');
    expect(normalizeWaterfallPalette(' ENHANCED ')).toBe('enhanced');
    expect(normalizeDisplayPrefs({}).waterfallPalette).toBe('classic');
    for (const palette of ['classic', 'ice', 'ember', 'forest']) {
      expect(normalizeWaterfallPalette(palette)).toBe(palette);
    }
  });
});
