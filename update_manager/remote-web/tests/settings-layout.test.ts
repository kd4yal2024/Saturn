import { describe, expect, it } from 'vitest';
import {
  LAYOUT_SAFETY_PINNED_IDS,
  LAYOUT_STORAGE_KEYS,
  LAYOUT_TIERS,
  LAYOUT_TOOLBAR_REORDERABLE_IDS,
  defaultLayout,
  layoutSummary,
  loadLayout,
  pinItem,
  reorderToolbar,
  sanitizeLayout,
  saveLayout,
  unpinItem,
} from '../src/settings/layout';
import { SETTINGS_REGISTRY } from '../src/settings/registry';

function memoryStorage(initial: Record<string, string> = {}) {
  const data = { ...initial };
  return {
    data,
    getItem: (key: string) => (key in data ? data[key] as string : null),
    setItem: (key: string, value: string) => { data[key] = value; },
  };
}

describe('layout defaults', () => {
  it('keeps every safety-pinned item on the main screen in all four tiers', () => {
    for (const tier of LAYOUT_TIERS) {
      const layout = defaultLayout(tier);
      for (const safety of LAYOUT_SAFETY_PINNED_IDS) {
        expect(layout.pinned, `${tier} is missing ${safety}`).toContain(safety);
      }
      expect(layout.hidden).toEqual([]);
    }
  });

  it('uses one storage key per tier', () => {
    expect(Object.values(LAYOUT_STORAGE_KEYS)).toEqual([
      'saturn.ui.layout.phone',
      'saturn.ui.layout.tablet',
      'saturn.ui.layout.desktop',
      'saturn.ui.layout.wide',
    ]);
  });

  it('encodes the section 7.4 tier differences', () => {
    const desktop = defaultLayout('desktop').pinned;
    const wide = defaultLayout('wide').pinned;
    const tablet = defaultLayout('tablet').pinned;
    const phone = defaultLayout('phone').pinned;
    // Desktop keeps the display toolbar actions and the radio path on screen.
    expect(desktop).toContain('display.zoom');
    expect(desktop).toContain('display.tunePeakToolbar');
    expect(desktop).toContain('radio.attenuator');
    expect(desktop).toContain('shell.operationsDrawer');
    // Wide additionally carries the radio path and the wider display controls.
    expect(wide).toContain('radio.rxInput');
    expect(wide).toContain('display.spectrumPeakHold');
    // Tablet keeps zoom in its toolbar and pushes tune-peak into overflow.
    expect(tablet).toContain('display.zoom');
    expect(tablet).not.toContain('display.tunePeakToolbar');
    expect(tablet).not.toContain('radio.rxInput');
    expect(tablet).toContain('shell.operationsDrawer');
    // Phone pushes the radio path and the operations bar into the menu.
    expect(phone).not.toContain('radio.attenuator');
    expect(phone).not.toContain('display.zoom');
    expect(phone).toContain('display.span');
    expect(phone).toContain('display.centerAction');
    expect(phone).not.toContain('shell.operationsDrawer');
    expect(phone).not.toContain('meter.details');
    // A tier never gains items the previous tier did not have.
    expect(desktop.length).toBeGreaterThanOrEqual(phone.length);
  });

  it('stays deterministic and safe when the registry is missing an id', () => {
    const sparse = SETTINGS_REGISTRY.filter((entry) => entry.id !== 'display.view');
    const once = defaultLayout('desktop', sparse);
    const twice = defaultLayout('desktop', sparse);
    expect(once).toEqual(twice);
    expect(once.pinned).not.toContain('display.view');
    for (const safety of LAYOUT_SAFETY_PINNED_IDS) expect(once.pinned).toContain(safety);
  });
});

describe('layout sanitising', () => {
  it('replaces corrupted payloads with defaults', () => {
    for (const raw of [null, undefined, 42, 'nonsense', [], { pinned: 'not-an-array' }, {}]) {
      const layout = sanitizeLayout(raw, 'desktop');
      expect(layout.pinned.length).toBeGreaterThan(0);
      for (const safety of LAYOUT_SAFETY_PINNED_IDS) expect(layout.pinned).toContain(safety);
    }
  });

  it('drops unknown ids, duplicates, and a safety item smuggled into hidden', () => {
    const layout = sanitizeLayout({
      pinned: ['radio.vfoA', 'no.such.item', 'radio.vfoA', 'display.view'],
      hidden: ['transmit.ptt', 'radio.attenuator'],
    }, 'phone');
    expect(layout.pinned).toContain('radio.vfoA');
    expect(layout.pinned).toContain('display.view');
    expect(layout.pinned).not.toContain('no.such.item');
    expect(layout.pinned.filter((id) => id === 'radio.vfoA')).toHaveLength(1);
    expect(layout.hidden).toEqual(['radio.attenuator']);
    expect(layout.pinned).toContain('transmit.ptt');
  });
});

describe('layout storage', () => {
  it('round-trips through storage', () => {
    const storage = memoryStorage();
    const pinned = pinItem(defaultLayout('desktop'), 'display.view', 'desktop');
    expect(saveLayout('desktop', pinned, storage)).toBe(true);
    const loaded = loadLayout('desktop', storage);
    expect(loaded.pinned).toEqual(pinned.pinned);
  });

  it('falls back to defaults when storage is unavailable or the payload is invalid', () => {
    const throwing = {
      getItem: () => { throw new Error('denied'); },
      setItem: () => { throw new Error('denied'); },
    };
    expect(loadLayout('phone', throwing).pinned.length).toBeGreaterThan(0);
    expect(saveLayout('phone', defaultLayout('phone'), throwing)).toBe(false);
    expect(loadLayout('phone', undefined).pinned).toEqual(defaultLayout('phone').pinned);
    const corrupt = memoryStorage({ [LAYOUT_STORAGE_KEYS.wide]: '{not json' });
    expect(loadLayout('wide', corrupt).pinned).toEqual(defaultLayout('wide').pinned);
  });
});

describe('pin, unpin and reorder', () => {
  it('refuses to unpin a safety-pinned item', () => {
    const layout = defaultLayout('desktop');
    for (const safety of LAYOUT_SAFETY_PINNED_IDS) {
      const result = unpinItem(layout, safety, 'desktop');
      expect(result.refused, `${safety} should not be unpinnable`).toBe(true);
      expect(result.reason).toBe('safety-pinned');
      expect(result.preferences.pinned).toContain(safety);
      expect(result.preferences.hidden).not.toContain(safety);
    }
  });

  it('unpins and re-pins an ordinary item', () => {
    const layout = defaultLayout('desktop');
    const removed = unpinItem(layout, 'display.view', 'desktop');
    expect(removed.refused).toBe(false);
    expect(removed.preferences.pinned).not.toContain('display.view');
    expect(removed.preferences.hidden).toContain('display.view');
    const restored = pinItem(removed.preferences, 'display.view', 'desktop');
    expect(restored.pinned).toContain('display.view');
    expect(restored.hidden).not.toContain('display.view');
  });

  it('reorders the toolbar without losing or duplicating items', () => {
    const layout = defaultLayout('desktop');
    // Non-toolbar ids in the requested order are ignored: §8.11 offers drag
    // handles for display toolbar items only.
    const reordered = reorderToolbar(layout, ['radio.antenna', 'radio.vfoA', 'ghost.item', 'display.span', 'display.span'], 'desktop');
    expect(reordered.pinned.indexOf('radio.antenna')).toBe(layout.pinned.indexOf('radio.antenna'));
    expect(reordered.pinned.indexOf('radio.vfoA')).toBe(layout.pinned.indexOf('radio.vfoA'));
    expect(reordered.pinned).toHaveLength(layout.pinned.length);
    expect(new Set(reordered.pinned).size).toBe(reordered.pinned.length);
    for (const safety of LAYOUT_SAFETY_PINNED_IDS) expect(reordered.pinned).toContain(safety);
  });

  it('reorders only the display toolbar and leaves every other item in place', () => {
    const layout = defaultLayout('desktop');
    const before = layout.pinned.slice();
    const reordered = reorderToolbar(
      layout,
      ['transmit.ptt', 'display.zoom', 'radio.antenna', 'display.span'],
      'desktop',
    );
    for (const id of ['transmit.ptt', 'transmit.arm', 'radio.vfoA', 'radio.antenna']) {
      if (!before.includes(id)) continue;
      expect(reordered.pinned.indexOf(id), `${id} moved`).toBe(before.indexOf(id));
    }
    const toolbarSlots = before
      .map((id, index) => (LAYOUT_TOOLBAR_REORDERABLE_IDS.includes(id) ? index : -1))
      .filter((index) => index >= 0);
    for (const id of LAYOUT_TOOLBAR_REORDERABLE_IDS) {
      if (!before.includes(id)) continue;
      expect(toolbarSlots).toContain(reordered.pinned.indexOf(id));
    }
    expect(reordered.pinned).toHaveLength(before.length);
    expect(new Set(reordered.pinned).size).toBe(before.length);
  });

  it('summarises which optional items are pinned or hidden', () => {
    const layout = unpinItem(defaultLayout('desktop'), 'display.view', 'desktop').preferences;
    const summary = layoutSummary(layout);
    expect(summary.pinned.some((entry) => entry.id === 'radio.vfoA')).toBe(true);
    expect(summary.hidden.map((entry) => entry.id)).toContain('display.view');
  });

  it('offers unpinned optional items in the pin list even with nothing hidden', () => {
    const summary = layoutSummary(defaultLayout('phone'));
    expect(summary.hidden).toEqual([]);
    expect(summary.available.length).toBeGreaterThan(0);
    // The phone tier overflows the radio path, so it must still be offerable.
    expect(summary.available.map((entry) => entry.id)).toContain('radio.attenuator');
    // Nothing can appear in two lists at once.
    const pinnedIds = summary.pinned.map((entry) => entry.id);
    for (const entry of summary.available) expect(pinnedIds).not.toContain(entry.id);
  });

  it('refuses to pin unknown or settings-only ids', () => {
    const layout = defaultLayout('phone');
    const unknown = pinItem(layout, 'no.such.item', 'phone');
    expect(unknown.pinned).toEqual(layout.pinned);
    const settingsOnly = pinItem(layout, 'transmit.txSetup', 'phone');
    expect(settingsOnly.pinned).not.toContain('transmit.txSetup');
    expect(settingsOnly.pinned).toEqual(layout.pinned);
  });
});
