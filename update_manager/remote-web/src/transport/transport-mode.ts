const PHASE42_TRUE_VALUES = new Set(['1', 'true', 'on', 'yes']);
const PHASE42_FALSE_VALUES = new Set(['0', 'false', 'off', 'no', 'legacy']);

export const PHASE42_SPLIT_QUERY_PARAM = 'phase42_split';
export const PHASE42_SPLIT_STORAGE_KEY = 'saturn.phase42.splitTransport';

function phase42Truthy(value: string | null | undefined): boolean {
  return PHASE42_TRUE_VALUES.has(String(value ?? '').trim().toLowerCase());
}

function phase42ExplicitlyDisabled(value: string | null | undefined): boolean {
  return PHASE42_FALSE_VALUES.has(String(value ?? '').trim().toLowerCase());
}

export function phase42SplitTransportEnabled(
  search: string,
  storedValue: string | null | undefined = undefined,
): boolean {
  const params = new URLSearchParams(search);
  const queryValue = params.get(PHASE42_SPLIT_QUERY_PARAM);
  if (queryValue !== null) {
    return !phase42ExplicitlyDisabled(queryValue);
  }
  if (storedValue !== undefined && storedValue !== null) {
    return phase42Truthy(storedValue) || !phase42ExplicitlyDisabled(storedValue);
  }
  return true;
}

export function createPhase42SessionId(
  nowMs = Date.now(),
  randomValue = Math.random(),
): string {
  const timePart = Math.max(0, Math.floor(Number.isFinite(nowMs) ? nowMs : 0)).toString(36);
  const entropySource = Math.max(0, Math.min(0.999999999, Number.isFinite(randomValue) ? randomValue : 0));
  const entropyPart = Math.floor(entropySource * 0x1_0000_0000).toString(36).padStart(7, '0');
  return `phase42-${timePart}-${entropyPart}`;
}
