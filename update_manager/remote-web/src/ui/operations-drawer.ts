export const OPERATIONS_DRAWER_TARGETS = [
  'memory',
  'audio',
  'network',
  'dsp',
  'radio',
  'log',
] as const;

export type OperationsDrawerTarget = (typeof OPERATIONS_DRAWER_TARGETS)[number];

export interface OperationsDrawerSelection {
  open: boolean;
  target: OperationsDrawerTarget;
}

export function normalizeOperationsDrawerTarget(
  value: unknown,
  fallback: OperationsDrawerTarget = 'memory',
): OperationsDrawerTarget {
  const normalized = `${value ?? ''}`.trim().toLowerCase();
  return (OPERATIONS_DRAWER_TARGETS as readonly string[]).includes(normalized)
    ? normalized as OperationsDrawerTarget
    : fallback;
}

/** Clicking the selected tab collapses the drawer; another tab opens its view. */
export function selectOperationsDrawerTarget(
  current: OperationsDrawerSelection,
  requested: unknown,
): OperationsDrawerSelection {
  const target = normalizeOperationsDrawerTarget(requested, current.target);
  return {
    open: !(current.open && current.target === target),
    target,
  };
}

export function restoreOperationsDrawerSelection(value: unknown): OperationsDrawerSelection {
  if (!value || typeof value !== 'object') {
    return { open: false, target: 'memory' };
  }
  const record = value as Record<string, unknown>;
  return {
    open: record.open === true,
    target: normalizeOperationsDrawerTarget(record.target),
  };
}
