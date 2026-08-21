import { describe, expect, it } from 'vitest';
import {
  SETUP_PANEL_IDS,
  adjacentSetupPanelId,
  normalizeSetupPanelId,
} from '../src/ui/setup-navigation';

describe('setup navigation', () => {
  it('defines the seven operator-facing setup destinations', () => {
    expect(SETUP_PANEL_IDS).toEqual([
      'profiles', 'display', 'dsp', 'tx', 'network', 'audio', 'advanced',
    ]);
  });

  it('normalizes stale persisted values safely', () => {
    expect(normalizeSetupPanelId(' Network ')).toBe('network');
    expect(normalizeSetupPanelId('routing')).toBe('profiles');
  });

  it('wraps keyboard navigation in both directions', () => {
    expect(adjacentSetupPanelId('profiles', -1)).toBe('advanced');
    expect(adjacentSetupPanelId('advanced', 1)).toBe('profiles');
    expect(adjacentSetupPanelId('dsp', 1)).toBe('tx');
  });
});
