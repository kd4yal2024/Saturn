import { describe, expect, it } from 'vitest';
import {
  normalizeOperationsDrawerTarget,
  restoreOperationsDrawerSelection,
  selectOperationsDrawerTarget,
} from '../src/ui/operations-drawer';

describe('operations drawer model', () => {
  it('normalizes known targets and rejects stale values', () => {
    expect(normalizeOperationsDrawerTarget(' NETWORK ')).toBe('network');
    expect(normalizeOperationsDrawerTarget('engineering')).toBe('memory');
  });

  it('opens a requested view and collapses an already-selected view', () => {
    expect(selectOperationsDrawerTarget({ open: false, target: 'memory' }, 'audio'))
      .toEqual({ open: true, target: 'audio' });
    expect(selectOperationsDrawerTarget({ open: true, target: 'audio' }, 'audio'))
      .toEqual({ open: false, target: 'audio' });
  });

  it('restores only safe persisted state', () => {
    expect(restoreOperationsDrawerSelection({ open: true, target: 'radio' }))
      .toEqual({ open: true, target: 'radio' });
    expect(restoreOperationsDrawerSelection({ open: 'yes', target: 'bad' }))
      .toEqual({ open: false, target: 'memory' });
  });
});
