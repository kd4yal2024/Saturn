export const RADIO_CONTROL_CONTEXTS = ['rx', 'dsp', 'tx'] as const;

export type RadioControlContext = (typeof RADIO_CONTROL_CONTEXTS)[number];

export function normalizeRadioControlContext(
  value: unknown,
  fallback: RadioControlContext = 'rx',
): RadioControlContext {
  const normalized = `${value ?? ''}`.trim().toLowerCase();
  return RADIO_CONTROL_CONTEXTS.includes(normalized as RadioControlContext)
    ? normalized as RadioControlContext
    : fallback;
}

export function radioControlContextForTarget(
  target: unknown,
): RadioControlContext | null {
  const normalized = `${target ?? ''}`.trim().toLowerCase();
  return RADIO_CONTROL_CONTEXTS.includes(normalized as RadioControlContext)
    ? normalized as RadioControlContext
    : null;
}
