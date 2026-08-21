export type PhoneSpectrumMode = 'balanced' | 'spectrum' | 'waterfall';

export const DEFAULT_SPECTRUM_WATERFALL_RATIO = 0.62;
export const MIN_SPECTRUM_WATERFALL_RATIO = 0.34;
export const MAX_SPECTRUM_WATERFALL_RATIO = 0.78;

const PHONE_SPECTRUM_MODES: readonly PhoneSpectrumMode[] = [
  'balanced',
  'spectrum',
  'waterfall',
];

export function clampSpectrumWaterfallRatio(
  value: unknown,
  fallback = DEFAULT_SPECTRUM_WATERFALL_RATIO,
): number {
  const fallbackValue = Number.isFinite(Number(fallback))
    ? Number(fallback)
    : DEFAULT_SPECTRUM_WATERFALL_RATIO;
  const numeric = Number(value);
  const candidate = Number.isFinite(numeric) ? numeric : fallbackValue;
  return Math.max(
    MIN_SPECTRUM_WATERFALL_RATIO,
    Math.min(MAX_SPECTRUM_WATERFALL_RATIO, candidate),
  );
}

export function spectrumWaterfallRatioFromPointer(
  clientY: unknown,
  top: unknown,
  height: unknown,
  fallback = DEFAULT_SPECTRUM_WATERFALL_RATIO,
): number {
  const numericHeight = Number(height);
  if (!Number.isFinite(numericHeight) || numericHeight <= 0) {
    return clampSpectrumWaterfallRatio(fallback);
  }
  return clampSpectrumWaterfallRatio(
    (Number(clientY) - Number(top)) / numericHeight,
    fallback,
  );
}

export function normalizePhoneSpectrumMode(value: unknown): PhoneSpectrumMode {
  const normalized = String(value ?? '').trim().toLowerCase();
  return PHONE_SPECTRUM_MODES.includes(normalized as PhoneSpectrumMode)
    ? normalized as PhoneSpectrumMode
    : 'balanced';
}

export function nextPhoneSpectrumMode(value: unknown): PhoneSpectrumMode {
  const current = normalizePhoneSpectrumMode(value);
  const index = PHONE_SPECTRUM_MODES.indexOf(current);
  return PHONE_SPECTRUM_MODES[(index + 1) % PHONE_SPECTRUM_MODES.length] ?? 'balanced';
}

export function phoneSpectrumRatio(value: unknown): number {
  switch (normalizePhoneSpectrumMode(value)) {
    case 'spectrum':
      return 0.76;
    case 'waterfall':
      return 0.42;
    default:
      return DEFAULT_SPECTRUM_WATERFALL_RATIO;
  }
}
