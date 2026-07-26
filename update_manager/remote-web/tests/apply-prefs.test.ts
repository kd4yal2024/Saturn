import { describe, expect, it } from 'vitest';
import {
  applyRadioPrefsToState,
  applyDisplayPrefsToState,
  normalizeAppStateInPlace,
} from '../src/state/apply-prefs';
import { createAppState } from '../src/state/app-state';

describe('applyRadioPrefsToState', () => {
  it('does nothing for null input', () => {
    const state = createAppState();
    const before = { ...state };
    applyRadioPrefsToState(null, state);
    expect(state.sampleRate).toBe(before.sampleRate);
    expect(state.mode).toBe(before.mode);
  });

  it('merges partial prefs, keeping existing as fallback', () => {
    const state = createAppState();
    state.agcGain = 50;
    applyRadioPrefsToState({ mode: 'LSB', txDrive: 75 }, state);
    expect(state.mode).toBe('LSB');
    expect(state.txDrive).toBe(75);
    expect(state.agcGain).toBe(50); // unchanged
  });

  it('clamps out-of-range values', () => {
    const state = createAppState();
    applyRadioPrefsToState({ agcGain: 999, rxVolumeDb: 50, txDrive: -5 }, state);
    expect(state.agcGain).toBeLessThanOrEqual(120);
    expect(state.rxVolumeDb).toBeLessThanOrEqual(12);
    expect(state.txDrive).toBeGreaterThanOrEqual(1);
  });

  it('normalizes invalid mode to USB', () => {
    const state = createAppState();
    applyRadioPrefsToState({ mode: 'garbage' }, state);
    expect(state.mode).toBe('USB');
  });

  it('handles eq bands', () => {
    const state = createAppState();
    const bands = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    applyRadioPrefsToState({ rxEqBands: bands }, state);
    expect(state.rxEqBands).toHaveLength(11);
  });

  it('applies WDSP 2.00 NR2 and phase-rotator preferences', () => {
    const state = createAppState();
    applyRadioPrefsToState({
      rxNr2GainMethod: 'trained',
      rxNr2NpeMethod: 'nstat',
      rxNr2PostFilterEnabled: false,
      rxWbfmDeemphasis: '50us',
      txPhaseRotatorEnabled: true,
      txPhaseRotatorAuto: true,
      txPhaseRotatorCornerHz: 2500,
    }, state);
    expect(state.rxNr2GainMethod).toBe('TRAINED');
    expect(state.rxNr2NpeMethod).toBe('NSTAT');
    expect(state.rxNr2PostFilterEnabled).toBe(false);
    expect(state.rxWbfmDeemphasis).toBe('EU_50US');
    expect(state.txPhaseRotatorEnabled).toBe(true);
    expect(state.txPhaseRotatorAuto).toBe(true);
    expect(state.txPhaseRotatorCornerHz).toBe(2000);
  });
});

describe('applyDisplayPrefsToState', () => {
  it('does nothing for null input', () => {
    const state = createAppState();
    const result = applyDisplayPrefsToState(null, state);
    expect(result.peakHoldCleared).toBe(false);
  });

  it('merges partial display prefs', () => {
    const state = createAppState();
    applyDisplayPrefsToState({
      waterfallPalette: 'ember',
      spectrumTraceColor: '#ff8800',
      spectrumTraceSmoothing: 35,
      spectrumTraceFill: 40,
      spectrumPeakGlow: 45,
      spectrumGlassSheen: 55,
      showGrid: false,
    }, state);
    expect(state.waterfallPalette).toBe('ember');
    expect(state.spectrumTraceColor).toBe('#ff8800');
    expect(state.spectrumTraceSmoothing).toBe(35);
    expect(state.spectrumTraceFill).toBe(40);
    expect(state.spectrumPeakGlow).toBe(45);
    expect(state.spectrumGlassSheen).toBe(55);
    expect(state.showGrid).toBe(false);
    expect(state.showCenterLine).toBe(true); // unchanged
  });

  it('normalizes invalid palette', () => {
    const state = createAppState();
    applyDisplayPrefsToState({ waterfallPalette: 'nope' }, state);
    expect(state.waterfallPalette).toBe('classic');
  });

  it('reports peakHoldCleared when peak hold turned off', () => {
    const state = createAppState();
    state.spectrumPeakHold = true;
    const result = applyDisplayPrefsToState({ spectrumPeakHold: false }, state);
    expect(result.peakHoldCleared).toBe(true);
  });

  it('does not report peakHoldCleared when peak hold stays on', () => {
    const state = createAppState();
    state.spectrumPeakHold = true;
    const result = applyDisplayPrefsToState({ showGrid: false }, state);
    expect(result.peakHoldCleared).toBe(false);
  });
});

describe('normalizeAppStateInPlace', () => {
  it('clamps default state without errors', () => {
    const state = createAppState();
    normalizeAppStateInPlace(state);
    expect(state.sampleRate).toBe(192000);
    expect(state.mode).toBe('USB');
    expect(state.displayZoom).toBe(1);
  });

  it('clamps out-of-range values', () => {
    const state = createAppState();
    state.agcGain = 999;
    state.displayZoom = 100;
    state.rxVolumeDb = 50;
    state.frameRate = -5;
    state.fftWidth = -1;
    normalizeAppStateInPlace(state);
    expect(state.agcGain).toBeLessThanOrEqual(120);
    expect(state.displayZoom).toBeLessThanOrEqual(32);
    expect(state.rxVolumeDb).toBeLessThanOrEqual(12);
    expect(state.frameRate).toBeGreaterThanOrEqual(0);
    expect(state.fftWidth).toBeGreaterThanOrEqual(0);
  });

  it('normalizes display prefs fields', () => {
    const state = createAppState();
    state.waterfallPalette = 'invalid';
    state.spectrumAverage = -5;
    state.spectrumTraceColor = 'invalid';
    state.spectrumTraceSmoothing = 999;
    state.spectrumTraceFill = 250;
    state.spectrumPeakGlow = -1;
    normalizeAppStateInPlace(state);
    expect(state.waterfallPalette).toBe('classic');
    expect(state.spectrumAverage).toBeGreaterThanOrEqual(1);
    expect(state.spectrumTraceColor).toBe('#62d0ff');
    expect(state.spectrumTraceSmoothing).toBe(100);
    expect(state.spectrumTraceFill).toBe(100);
    expect(state.spectrumPeakGlow).toBe(0);
  });
});
