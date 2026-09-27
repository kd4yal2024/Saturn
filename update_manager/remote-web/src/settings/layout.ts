import { SETTINGS_REGISTRY, type SettingsEntry } from './registry';

/**
 * Per-tier main-screen layout preferences (build sheet §7 defaults, §8.11
 * "Show on main screen", §15 persistence).
 *
 * Guarantees this module provides:
 * - reads never throw: unavailable, unparsable or hostile storage yields defaults;
 * - writes never throw and report success;
 * - the safety-pinned set is immutable: VPFO/transmit-critical items cannot be
 *   unpinned or dropped by a corrupted payload;
 * - unknown ids are discarded, duplicates are collapsed, and ordering is stable.
 */

export type LayoutTier = 'phone' | 'tablet' | 'desktop' | 'wide';

export type LayoutPreferences = {
  /** Items shown on the main screen, in toolbar order. */
  pinned: string[];
  /** Items deliberately kept off the main screen. */
  hidden: string[];
};

export const LAYOUT_TIERS: readonly LayoutTier[] = ['phone', 'tablet', 'desktop', 'wide'];

export const LAYOUT_STORAGE_KEYS: Readonly<Record<LayoutTier, string>> = {
  phone: 'saturn.ui.layout.phone',
  tablet: 'saturn.ui.layout.tablet',
  desktop: 'saturn.ui.layout.desktop',
  wide: 'saturn.ui.layout.wide',
};

/** Items that may never be unpinned, per §8.11. */
export const LAYOUT_SAFETY_PINNED_IDS: readonly string[] = [
  'radio.vfoA',
  'transmit.appBarBadge',
  'transmit.arm',
  'transmit.ptt',
  'shell.onAirBar',
  'shell.settings',
];

export type LayoutStorage = Pick<Storage, 'getItem' | 'setItem'>;

/** §7.4: items a tier adds to the registry's always-on baseline. */
const TIER_EXTRA_PINNED: Readonly<Record<LayoutTier, readonly string[]>> = {
  desktop: [
    'display.span', 'display.zoom', 'display.viewTraditional', 'display.centerAction',
    'display.pause', 'display.waterfallToggle', 'display.averageToolbar', 'display.peakToolbar',
    'display.tunePeakToolbar', 'display.wakeLockAction', 'display.freqLockAction',
    'radio.attenuator', 'radio.antenna', 'receive.voiceSquelch', 'receive.agcGain',
    'meter.type', 'meter.average', 'meter.details', 'shell.operationsDrawer',
  ],
  wide: [
    'display.span', 'display.zoom', 'display.viewTraditional', 'display.centerAction',
    'display.pause', 'display.waterfallToggle', 'display.averageToolbar', 'display.peakToolbar',
    'display.tunePeakToolbar', 'display.wakeLockAction', 'display.freqLockAction',
    'radio.attenuator', 'radio.antenna', 'radio.rxInput', 'display.spectrumPeakHold',
    'receive.voiceSquelch', 'receive.agcGain', 'receive.filterLow', 'receive.filterHigh',
    'meter.type', 'meter.average', 'meter.details', 'shell.operationsDrawer',
  ],
  tablet: ['display.span', 'display.zoom', 'display.viewTraditional', 'display.centerAction',
    'display.waterfallToggle', 'display.peakToolbar', 'meter.type', 'meter.average', 'meter.details', 'shell.operationsDrawer'],
  phone: ['display.span', 'display.viewTraditional', 'display.centerAction', 'display.waterfallToggle'],
};

/** §7.4: what a tier pushes into its overflow menu instead of the main screen. */
const TIER_OVERFLOW: Readonly<Record<LayoutTier, readonly string[]>> = {
  desktop: [],
  wide: [],
  tablet: ['display.tunePeakToolbar', 'display.averageToolbar', 'radio.rxInput'],
  phone: [
    'display.zoom', 'display.tunePeakToolbar', 'display.averageToolbar', 'display.peakToolbar',
    'display.wakeLockAction', 'display.freqLockAction', 'display.spectrumPeakHold',
    'radio.rxInput', 'radio.attenuator', 'radio.antenna', 'radio.keypad',
    'receive.voiceSquelch', 'receive.agcGain', 'receive.filterLow', 'receive.filterHigh',
    'meter.details', 'shell.operationsDrawer',
  ],
};

function sanitizeList(value: unknown, known: Set<string>, seen: Set<string>): string[] {
  if (!Array.isArray(value)) return [];
  const result: string[] = [];
  for (const candidate of value) {
    if (typeof candidate !== 'string') continue;
    if (!known.has(candidate) || seen.has(candidate)) continue;
    seen.add(candidate);
    result.push(candidate);
  }
  return result;
}

export function defaultLayout(
  tier: LayoutTier,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): LayoutPreferences {
  const known = new Set(registry.map((item) => item.id));
  const pinned: string[] = [];
  const push = (id: string) => {
    if (!known.has(id) || pinned.includes(id)) return;
    pinned.push(id);
  };
  // Baseline: everything the registry marks always-on, in registry order.
  for (const item of registry) {
    if (item.mainScreen === 'always') push(item.id);
  }
  for (const id of TIER_EXTRA_PINNED[tier]) push(id);
  const overflow = new Set(TIER_OVERFLOW[tier]);
  const result = pinned.filter((id) => !overflow.has(id));
  // Safety-pinned items are never subject to overflow, and are present even when
  // the registry does not (yet) carry them, so a missing id cannot silently drop
  // Arm, Hold PTT or the TX state badge from the main screen.
  for (const safety of LAYOUT_SAFETY_PINNED_IDS) {
    if (!result.includes(safety)) result.unshift(safety);
  }
  return { pinned: result, hidden: [] };
}

/** Coerce anything that came out of storage into a usable preference set. */
export function sanitizeLayout(
  raw: unknown,
  tier: LayoutTier,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): LayoutPreferences {
  const known = new Set(registry.map((item) => item.id));
  for (const safety of LAYOUT_SAFETY_PINNED_IDS) known.add(safety);
  const defaults = defaultLayout(tier, registry);
  if (!raw || typeof raw !== 'object') return defaults;
  const source = raw as Partial<LayoutPreferences>;
  const seen = new Set<string>();
  const pinned = sanitizeList(source.pinned, known, seen);
  const hidden = sanitizeList(source.hidden, known, seen);
  // A safety item can never end up hidden, however the payload was written.
  const safeHidden = hidden.filter((id) => !LAYOUT_SAFETY_PINNED_IDS.includes(id));
  for (const safety of LAYOUT_SAFETY_PINNED_IDS) {
    if (!pinned.includes(safety)) pinned.unshift(safety);
  }
  if (pinned.length === 0) return defaults;
  return { pinned, hidden: safeHidden };
}

export function loadLayout(
  tier: LayoutTier,
  storage: LayoutStorage | undefined = typeof localStorage === 'undefined' ? undefined : localStorage,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): LayoutPreferences {
  if (!storage) return defaultLayout(tier, registry);
  try {
    const raw = storage.getItem(LAYOUT_STORAGE_KEYS[tier]);
    if (!raw) return defaultLayout(tier, registry);
    return sanitizeLayout(JSON.parse(raw), tier, registry);
  } catch {
    return defaultLayout(tier, registry);
  }
}

export function saveLayout(
  tier: LayoutTier,
  preferences: LayoutPreferences,
  storage: LayoutStorage | undefined = typeof localStorage === 'undefined' ? undefined : localStorage,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): boolean {
  if (!storage) return false;
  try {
    storage.setItem(LAYOUT_STORAGE_KEYS[tier], JSON.stringify(sanitizeLayout(preferences, tier, registry)));
    return true;
  } catch {
    return false;
  }
}

export function pinItem(
  preferences: LayoutPreferences,
  id: string,
  tier: LayoutTier,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): LayoutPreferences {
  const current = sanitizeLayout(preferences, tier, registry);
  // Only real, main-screen-capable entries may be pinned: unknown ids and
  // settings-only rows are refused rather than silently added.
  const entry = registry.find((item) => item.id === id);
  if (!entry || entry.mainScreen === 'settings-only') return current;
  if (!current.pinned.includes(id)) current.pinned.push(id);
  return { pinned: current.pinned, hidden: current.hidden.filter((item) => item !== id) };
}

export type UnpinResult = { preferences: LayoutPreferences; refused: boolean; reason?: string };

export function unpinItem(
  preferences: LayoutPreferences,
  id: string,
  tier: LayoutTier,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): UnpinResult {
  const current = sanitizeLayout(preferences, tier, registry);
  if (LAYOUT_SAFETY_PINNED_IDS.includes(id)) {
    return { preferences: current, refused: true, reason: 'safety-pinned' };
  }
  return {
    preferences: {
      pinned: current.pinned.filter((item) => item !== id),
      hidden: current.hidden.includes(id) ? current.hidden : [...current.hidden, id],
    },
    refused: false,
  };
}

/** Apply a new toolbar order; unknown ids are dropped, omitted ids keep their
 *  previous relative order at the end, and safety items stay pinned. */
/** §8.11 offers drag handles only for display toolbar items; nothing else moves. */
export const LAYOUT_TOOLBAR_REORDERABLE_IDS: readonly string[] = [
  'display.span',
  'display.zoom',
  'display.averageToolbar',
  'display.peakToolbar',
  'display.tunePeakToolbar',
  'display.pause',
  'display.centerAction',
  'display.waterfallToggle',
  'display.viewTraditional',
  'display.wakeLockAction',
  'display.freqLockAction',
  'display.spectrumPeakHold',
];

export function reorderToolbar(
  preferences: LayoutPreferences,
  order: readonly string[],
  tier: LayoutTier,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): LayoutPreferences {
  const current = sanitizeLayout(preferences, tier, registry);
  const reorderable = new Set(LAYOUT_TOOLBAR_REORDERABLE_IDS);
  const slots: number[] = [];
  const previous: string[] = [];
  current.pinned.forEach((id, index) => {
    if (!reorderable.has(id)) return;
    slots.push(index);
    previous.push(id);
  });
  if (slots.length === 0) return current;
  const requested: string[] = [];
  for (const id of order) {
    if (!reorderable.has(id) || requested.includes(id) || !current.pinned.includes(id)) continue;
    requested.push(id);
  }
  // Only the toolbar slots are permuted; every other pinned item, safety items
  // included, keeps its exact position.
  const sequence = [...requested, ...previous.filter((id) => !requested.includes(id))];
  const next = [...current.pinned];
  slots.forEach((slot, index) => { next[slot] = sequence[index] as string; });
  return { pinned: next, hidden: current.hidden };
}

/** Which optional items a tier currently shows, for the Settings pin list. */
export function layoutSummary(
  preferences: LayoutPreferences,
  registry: readonly SettingsEntry[] = SETTINGS_REGISTRY,
): { pinned: SettingsEntry[]; hidden: SettingsEntry[]; available: SettingsEntry[] } {
  const byId = new Map(registry.map((item) => [item.id, item]));
  const pinnedIds = new Set(preferences.pinned);
  const hiddenIds = new Set(preferences.hidden);
  return {
    pinned: preferences.pinned.map((id) => byId.get(id)).filter((item): item is SettingsEntry => Boolean(item)),
    hidden: preferences.hidden.map((id) => byId.get(id)).filter((item): item is SettingsEntry => Boolean(item)),
    // Optional items that are neither pinned nor explicitly hidden are still
    // offered in the pin list; they are simply off the main screen by default.
    available: registry.filter((item) => item.mainScreen !== 'settings-only'
      && !pinnedIds.has(item.id) && !hiddenIds.has(item.id)),
  };
}
