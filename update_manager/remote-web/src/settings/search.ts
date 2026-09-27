import {
  SETTINGS_REGISTRY,
  type SettingsEntry,
  type SettingsSectionId,
} from './registry';

export type SettingsSearchOptions = {
  section?: SettingsSectionId;
  limit?: number;
};

export function normalizeSettingsQuery(query: string): string {
  return String(query ?? '').trim().toLowerCase();
}

/**
 * Search the settings index by new label, plain-language description, or the
 * pre-redesign names kept in `synonyms` (build sheet §8.11 requires that the old
 * labels — "MON", "SQL", "ATT", "SATP" — still find their control).
 */
export function searchSettings(query: string, options: SettingsSearchOptions = {}): SettingsEntry[] {
  const pool = options.section
    ? SETTINGS_REGISTRY.filter((item) => item.section === options.section)
    : [...SETTINGS_REGISTRY];
  const needle = normalizeSettingsQuery(query);
  if (!needle) {
    return typeof options.limit === 'number' ? pool.slice(0, options.limit) : pool;
  }
  const scored: { entry: SettingsEntry; score: number }[] = [];
  for (const entry of pool) {
    const label = entry.label.toLowerCase();
    const description = entry.description.toLowerCase();
    const synonyms = entry.synonyms.map((value) => value.toLowerCase());
    let score = 0;
    if (label === needle) score += 100;
    else if (label.startsWith(needle)) score += 60;
    else if (label.includes(needle)) score += 40;
    if (synonyms.includes(needle)) score += 80;
    else if (synonyms.some((value) => value.includes(needle))) score += 30;
    if (description.includes(needle)) score += 15;
    if (entry.id.toLowerCase().includes(needle)) score += 10;
    if (entry.targetId.toLowerCase().includes(needle)) score += 5;
    if (score > 0) scored.push({ entry, score });
  }
  scored.sort((left, right) => right.score - left.score || left.entry.label.localeCompare(right.entry.label));
  const limit = typeof options.limit === 'number' ? options.limit : scored.length;
  return scored.slice(0, limit).map((item) => item.entry);
}
