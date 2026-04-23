import { describe, expect, it } from 'vitest';
import {
  normalizeDemodMode,
  signedPassbandFromUiCuts,
  uiCutsFromSignedPassband,
} from '../src/radio/passband';

describe('passband helpers', () => {
  it('normalizes unsupported modes to USB', () => {
    expect(normalizeDemodMode('lsb')).toBe('LSB');
    expect(normalizeDemodMode('bad')).toBe('USB');
  });

  it('uses positive signed passbands for USB-like modes', () => {
    expect(signedPassbandFromUiCuts(100, 2900, 'USB')).toEqual({ lowHz: 100, highHz: 2900 });
  });

  it('uses negative signed passbands for LSB-like modes', () => {
    expect(signedPassbandFromUiCuts(100, 2900, 'LSB')).toEqual({ lowHz: -2900, highHz: -100 });
    expect(uiCutsFromSignedPassband(-2900, -100, 'LSB')).toEqual({ lowHz: 100, highHz: 2900 });
  });

  it('uses symmetric passbands for AM-like modes', () => {
    expect(signedPassbandFromUiCuts(50, 5000, 'AM')).toEqual({ lowHz: -5000, highHz: 5000 });
    expect(uiCutsFromSignedPassband(-5000, 5000, 'AM')).toEqual({ lowHz: 0, highHz: 5000 });
  });
});
