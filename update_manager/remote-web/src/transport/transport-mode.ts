const SPLIT_TRUE_VALUES = new Set(['1', 'true', 'on', 'yes', 'split']);
const SPLIT_FALSE_VALUES = new Set(['0', 'false', 'off', 'no', 'legacy', 'single']);

export const SPLIT_TRANSPORT_QUERY_PARAM = 'transport';
export const SPLIT_TRANSPORT_LEGACY_QUERY_PARAM = 'phase42_split';
export const SPLIT_TRANSPORT_STORAGE_KEY = 'saturn.remote.splitTransport';
export const SPLIT_TRANSPORT_LEGACY_STORAGE_KEY = 'saturn.phase42.splitTransport';

function splitTruthy(value: string | null | undefined): boolean {
  return SPLIT_TRUE_VALUES.has(String(value ?? '').trim().toLowerCase());
}

function splitExplicitlyDisabled(value: string | null | undefined): boolean {
  return SPLIT_FALSE_VALUES.has(String(value ?? '').trim().toLowerCase());
}

export function splitTransportEnabled(
  search: string,
  storedValue: string | null | undefined = undefined,
): boolean {
  const params = new URLSearchParams(search);
  const queryValue = params.has(SPLIT_TRANSPORT_QUERY_PARAM)
    ? params.get(SPLIT_TRANSPORT_QUERY_PARAM)
    : params.get(SPLIT_TRANSPORT_LEGACY_QUERY_PARAM);
  if (queryValue !== null) {
    return !splitExplicitlyDisabled(queryValue);
  }
  if (storedValue !== undefined && storedValue !== null) {
    return splitTruthy(storedValue) || !splitExplicitlyDisabled(storedValue);
  }
  return true;
}

export function createSplitSessionId(
  nowMs = Date.now(),
  randomValue = Math.random(),
): string {
  const timePart = Math.max(0, Math.floor(Number.isFinite(nowMs) ? nowMs : 0)).toString(36);
  const entropySource = Math.max(0, Math.min(0.999999999, Number.isFinite(randomValue) ? randomValue : 0));
  const entropyPart = Math.floor(entropySource * 0x1_0000_0000).toString(36).padStart(7, '0');
  return `split-${timePart}-${entropyPart}`;
}
