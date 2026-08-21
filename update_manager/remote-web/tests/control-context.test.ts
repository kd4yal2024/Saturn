import { describe, expect, it } from 'vitest';
import {
  normalizeRadioControlContext,
  radioControlContextForTarget,
} from '../src/ui/control-context';

describe('normalizeRadioControlContext', () => {
  it('accepts the three operator contexts case-insensitively', () => {
    expect(normalizeRadioControlContext('RX')).toBe('rx');
    expect(normalizeRadioControlContext('dsp')).toBe('dsp');
    expect(normalizeRadioControlContext('Tx')).toBe('tx');
  });

  it('uses a safe receive fallback for stale preferences', () => {
    expect(normalizeRadioControlContext('network')).toBe('rx');
    expect(normalizeRadioControlContext(null)).toBe('rx');
  });
});

describe('radioControlContextForTarget', () => {
  it('maps matching shell targets to a context', () => {
    expect(radioControlContextForTarget('rx')).toBe('rx');
    expect(radioControlContextForTarget('dsp')).toBe('dsp');
    expect(radioControlContextForTarget('tx')).toBe('tx');
  });

  it('leaves non-context navigation alone', () => {
    expect(radioControlContextForTarget('radio')).toBeNull();
    expect(radioControlContextForTarget('more')).toBeNull();
  });
});
