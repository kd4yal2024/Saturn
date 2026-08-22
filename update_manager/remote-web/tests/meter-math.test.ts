import { describe, expect, it } from 'vitest';
import {
  lerp, moveToward, smoothToward, svgArcPath, dbmToSmDeg, formatSignalStrength, linearLevelToDbfs,
  instrumentOperatingState, auxDeg, txPowerToAuxDeg, swrToAuxDeg,
  SM_START, SM_END, AX_START, AX_END,
} from '../src/ui/meter-math';

describe('lerp', () => {
  it('returns start at t=0', () => expect(lerp(10, 20, 0)).toBe(10));
  it('returns end at t=1', () => expect(lerp(10, 20, 1)).toBe(20));
  it('returns midpoint at t=0.5', () => expect(lerp(10, 20, 0.5)).toBe(15));
});

describe('moveToward', () => {
  it('rises toward target', () => {
    expect(moveToward(0, 100, 200, 50, 0.1)).toBe(20);
  });
  it('falls toward target', () => {
    expect(moveToward(100, 0, 200, 50, 0.1)).toBe(95);
  });
  it('returns target when current is null', () => {
    expect(moveToward(null, 42, 100, 100, 0.1)).toBe(42);
  });
  it('clamps to target on overshoot', () => {
    expect(moveToward(99, 100, 200, 50, 1)).toBe(100);
  });
});

describe('smoothToward', () => {
  it('uses a fast exponential attack without overshooting', () => {
    const value = smoothToward(-120, -60, 65, 360, 0.065);
    expect(value).toBeGreaterThan(-100);
    expect(value).toBeLessThan(-60);
  });

  it('uses a slower release than attack', () => {
    const attack = smoothToward(-120, -60, 65, 360, 0.065);
    const release = smoothToward(-60, -120, 65, 360, 0.065);
    expect(attack - (-120)).toBeGreaterThan((-60) - release);
  });

  it('initializes from the target and remains stable at zero elapsed time', () => {
    expect(smoothToward(null, -73, 65, 360, 0.016)).toBe(-73);
    expect(smoothToward(-90, -60, 65, 360, 0)).toBe(-90);
  });
});

describe('svgArcPath', () => {
  it('returns M...A path string', () => {
    const path = svgArcPath(100, 100, 50, 0, 90);
    expect(path).toMatch(/^M .+ A 50 50 0 \d 1 .+$/);
  });
});

describe('dbmToSmDeg', () => {
  it('returns SM_START for very low dBm', () => {
    expect(dbmToSmDeg(-130)).toBe(SM_START);
  });
  it('returns SM_END for very high dBm', () => {
    expect(dbmToSmDeg(0)).toBe(SM_END);
  });
  it('returns midrange for S5 (-97 dBm)', () => {
    const deg = dbmToSmDeg(-97);
    expect(deg).toBeGreaterThan(SM_START);
    expect(deg).toBeLessThan(SM_END);
  });
});

describe('formatSignalStrength', () => {
  it('does not represent missing telemetry as a signal', () => {
    expect(formatSignalStrength(null)).toBe('--');
    expect(formatSignalStrength(Number.NaN)).toBe('--');
  });

  it('formats the calibrated S-unit thresholds', () => {
    expect(formatSignalStrength(-121)).toBe('S1');
    expect(formatSignalStrength(-97)).toBe('S5');
    expect(formatSignalStrength(-73)).toBe('S9');
  });

  it('formats readings above S9 as decibels over S9', () => {
    expect(formatSignalStrength(-63)).toBe('S9 +10');
    expect(formatSignalStrength(-13)).toBe('S9 +60');
  });
});

describe('instrumentOperatingState', () => {
  it('keeps receive quiet until TX is requested', () => {
    expect(instrumentOperatingState('rx', false, false)).toBe('rx');
  });

  it('distinguishes armed from keyed operation', () => {
    expect(instrumentOperatingState('armed', false, false)).toBe('armed');
    expect(instrumentOperatingState('rx', true, false)).toBe('armed');
    expect(instrumentOperatingState('rx', false, false, true)).toBe('armed');
    expect(instrumentOperatingState('keyed', true, true)).toBe('tx');
  });
});

describe('linearLevelToDbfs', () => {
  it('formats normalized audio peaks on a dBFS scale', () => {
    expect(linearLevelToDbfs(1)).toBe(0);
    expect(linearLevelToDbfs(0.5)).toBeCloseTo(-6.02, 2);
    expect(linearLevelToDbfs(0.1)).toBeCloseTo(-20, 4);
  });

  it('uses the configured floor for silence and rejects missing telemetry', () => {
    expect(linearLevelToDbfs(0, -90)).toBe(-90);
    expect(linearLevelToDbfs(null)).toBeNull();
  });
});

describe('auxDeg', () => {
  it('returns AX_START for min value', () => {
    expect(auxDeg(0, 0, 100)).toBe(AX_START);
  });
  it('returns AX_END for max value', () => {
    expect(auxDeg(100, 0, 100)).toBe(AX_END);
  });
});

describe('txPowerToAuxDeg', () => {
  it('returns AX_START for 0W', () => {
    expect(txPowerToAuxDeg(0)).toBe(AX_START);
  });
  it('increases with power', () => {
    expect(txPowerToAuxDeg(50)).toBeGreaterThan(txPowerToAuxDeg(10));
    expect(txPowerToAuxDeg(100)).toBeGreaterThan(txPowerToAuxDeg(50));
  });
});

describe('swrToAuxDeg', () => {
  it('returns AX_START for SWR 1.0', () => {
    expect(swrToAuxDeg(1)).toBe(AX_START);
  });
  it('returns AX_END for SWR 5.0', () => {
    expect(swrToAuxDeg(5)).toBe(AX_END);
  });
  it('increases with SWR', () => {
    expect(swrToAuxDeg(2)).toBeGreaterThan(swrToAuxDeg(1));
    expect(swrToAuxDeg(3)).toBeGreaterThan(swrToAuxDeg(2));
  });
});
