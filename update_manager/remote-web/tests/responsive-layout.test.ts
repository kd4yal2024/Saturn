import { describe, expect, it } from 'vitest';

import { preferredResponsiveLayout } from '../src/ui/responsive-layout';

describe('preferredResponsiveLayout', () => {
  it('preserves an explicit operator layout choice', () => {
    expect(preferredResponsiveLayout('desktop', { width: 390, height: 844, coarsePointer: true })).toBe('desktop');
    expect(preferredResponsiveLayout('phone', { width: 1920, height: 1080, coarsePointer: false })).toBe('phone');
  });

  it('selects phone for portrait and landscape phones', () => {
    expect(preferredResponsiveLayout(null, { width: 390, height: 844, coarsePointer: true })).toBe('phone');
    expect(preferredResponsiveLayout(null, { width: 844, height: 390, coarsePointer: true })).toBe('phone');
  });

  it('keeps tablets and desktops in the shared full-console layout', () => {
    expect(preferredResponsiveLayout(null, { width: 768, height: 1024, coarsePointer: true })).toBe('desktop');
    expect(preferredResponsiveLayout(null, { width: 1024, height: 768, coarsePointer: true })).toBe('desktop');
    expect(preferredResponsiveLayout(null, { width: 1920, height: 1080, coarsePointer: false })).toBe('desktop');
  });
});
