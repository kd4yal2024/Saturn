import { describe, expect, it } from 'vitest';
import { radioPrefsFromState, displayPrefsFromState } from '../src/state/prefs-from-state';
import { createAppState } from '../src/state/app-state';

describe('radioPrefsFromState', () => {
  it('returns clamped radio prefs from default state', () => {
    const state = createAppState();
    const prefs = radioPrefsFromState(state);
    expect(prefs.sampleRate).toBe(192000);
    expect(prefs.mode).toBe('USB');
    expect(prefs.rxVolumeDb).toBe(-10);
    expect(prefs.agcMode).toBe('MEDIUM');
    expect(prefs.agcGain).toBe(80);
    expect(prefs.filterLow).toBe(50);
    expect(prefs.filterHigh).toBe(3050);
    expect(prefs.txDrive).toBe(10);
    expect(prefs.rxEqBands).toHaveLength(11);
    expect(prefs.txEqBands).toHaveLength(11);
    expect(prefs.cfcBands).toHaveLength(11);
  });

  it('clamps out-of-range values', () => {
    const state = createAppState();
    state.agcGain = 999;
    state.rxVolumeDb = 50;
    state.txDrive = -5;
    const prefs = radioPrefsFromState(state);
    expect(prefs.agcGain).toBeLessThanOrEqual(120);
    expect(prefs.rxVolumeDb).toBeLessThanOrEqual(12);
    expect(prefs.txDrive).toBeGreaterThanOrEqual(1);
  });

  it('normalizes mode strings', () => {
    const state = createAppState();
    state.mode = 'garbage';
    const prefs = radioPrefsFromState(state);
    expect(prefs.mode).toBe('USB');
  });
});

describe('displayPrefsFromState', () => {
  it('returns clamped display prefs from default state', () => {
    const state = createAppState();
    const prefs = displayPrefsFromState(state);
    expect(prefs.spectrumAutoRange).toBe(true);
    expect(prefs.spectrumFloorDb).toBe(-200);
    expect(prefs.spectrumCeilingDb).toBe(-120);
    expect(prefs.waterfallPalette).toBe('classic');
    expect(prefs.spectrumAverage).toBe(1);
    expect(prefs.waterfallSpeed).toBe(1);
    expect(prefs.showGrid).toBe(true);
    expect(prefs.showCenterLine).toBe(true);
    expect(prefs.showBandEdges).toBe(true);
  });

  it('normalizes invalid palette', () => {
    const state = createAppState();
    state.waterfallPalette = 'nope';
    const prefs = displayPrefsFromState(state);
    expect(prefs.waterfallPalette).toBe('classic');
  });
});
