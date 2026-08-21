export const SETUP_PANEL_IDS = [
  'profiles',
  'display',
  'dsp',
  'tx',
  'network',
  'audio',
  'advanced',
] as const;

export type SetupPanelId = (typeof SETUP_PANEL_IDS)[number];

export function normalizeSetupPanelId(
  value: unknown,
  fallback: SetupPanelId = 'profiles',
): SetupPanelId {
  const normalized = `${value ?? ''}`.trim().toLowerCase();
  return (SETUP_PANEL_IDS as readonly string[]).includes(normalized)
    ? normalized as SetupPanelId
    : fallback;
}

export function adjacentSetupPanelId(
  current: unknown,
  direction: -1 | 1,
): SetupPanelId {
  const panel = normalizeSetupPanelId(current);
  const index = SETUP_PANEL_IDS.indexOf(panel);
  return SETUP_PANEL_IDS[(index + direction + SETUP_PANEL_IDS.length) % SETUP_PANEL_IDS.length]!;
}
