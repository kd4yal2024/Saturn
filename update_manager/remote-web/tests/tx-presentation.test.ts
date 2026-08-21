import { describe, expect, it } from 'vitest';
import {
  txActionAvailability,
  txControlPresentationState,
} from '../src/ui/tx-presentation';

const base = {
  connected: true,
  blocked: false,
  receiveOnly: false,
  faulted: false,
  ready: false,
  txPhase: 'rx',
  requested: false,
  enabled: false,
};

describe('txControlPresentationState', () => {
  it('distinguishes unavailable, locked, and deliberately armed states', () => {
    expect(txControlPresentationState({ ...base, connected: false })).toBe('disabled');
    expect(txControlPresentationState(base)).toBe('locked');
    expect(txControlPresentationState({ ...base, ready: true })).toBe('armed');
  });

  it('distinguishes keying from confirmed transmission', () => {
    expect(txControlPresentationState({ ...base, ready: true, requested: true })).toBe('engaging');
    expect(txControlPresentationState({ ...base, ready: true, txPhase: 'keyed' })).toBe('transmitting');
  });

  it('keeps receive-only modes and blocked RF disabled', () => {
    expect(txControlPresentationState({ ...base, receiveOnly: true, ready: true })).toBe('disabled');
    expect(txControlPresentationState({ ...base, blocked: true, ready: true })).toBe('disabled');
  });

  it('exposes a latched TX fault only when the radio is otherwise available', () => {
    expect(txControlPresentationState({ ...base, faulted: true })).toBe('fault');
    expect(txControlPresentationState({ ...base, faulted: true, connected: false })).toBe('disabled');
  });

  it('keeps an active owner releasable while a new block is being reported', () => {
    expect(txControlPresentationState({ ...base, blocked: true, requested: true })).toBe('engaging');
    expect(txControlPresentationState({ ...base, receiveOnly: true, txPhase: 'keyed' })).toBe('transmitting');
  });
});

describe('txActionAvailability', () => {
  it('requires a separate arm action before key controls become available', () => {
    expect(txActionAvailability('locked', '')).toEqual({ arm: true, ptt: false, mox: false, lock: true });
    expect(txActionAvailability('armed', '')).toEqual({ arm: false, ptt: true, mox: true, lock: true });
  });

  it('permits deliberate fault recovery without enabling a key control', () => {
    expect(txActionAvailability('fault', '')).toEqual({ arm: true, ptt: false, mox: false, lock: true });
  });

  it('keeps only the owning active control enabled so it can release', () => {
    expect(txActionAvailability('transmitting', 'ptt').ptt).toBe(true);
    expect(txActionAvailability('transmitting', 'ptt').mox).toBe(false);
    expect(txActionAvailability('engaging', 'mox').mox).toBe(true);
  });
});
