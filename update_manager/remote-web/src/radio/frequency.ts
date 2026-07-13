export const MIN_FREQUENCY_HZ = 0;
// Saturn's 122.88 MHz ADC clock permits direct sampling through its first
// Nyquist zone. Keep the browser controls aligned with the bridge-side
// frequency clamp so the 88-108 MHz FM broadcast band remains reachable.
export const MAX_FREQUENCY_HZ = 122_880_000;

export function clampFrequencyHz(value: number): number {
  if (!Number.isFinite(value)) return MIN_FREQUENCY_HZ;
  return Math.max(MIN_FREQUENCY_HZ, Math.min(MAX_FREQUENCY_HZ, Math.round(value)));
}

export function formatFrequencyHz(hz: number): string {
  const text = clampFrequencyHz(hz).toString().padStart(7, '0');
  const whole = text.slice(0, -6) || '0';
  const khz = text.slice(-6, -3);
  const rest = text.slice(-3);
  return `${whole}.${khz}.${rest}`;
}

export function formatFrequencyMarkup(hz: number): string {
  const digits = Math.max(0, Math.round(hz)).toString();
  const padded = digits.padStart(Math.max(7, digits.length), '0');
  const parts: string[] = [];
  for (let i = 0; i < padded.length; i += 1) {
    if (i > 0 && (padded.length - i) % 3 === 0) {
      parts.push('<span class="freq-separator">.</span>');
    }
    const step = 10 ** (padded.length - i - 1);
    parts.push(
      `<span class="freq-digit" data-step="${step}" tabindex="0" aria-label="Tune ${step} Hz digit">${padded[i]}</span>`,
    );
  }
  return parts.join('');
}

export function digitStepForIndex(paddedDigitCount: number, digitIndex: number): number {
  if (!Number.isInteger(paddedDigitCount) || paddedDigitCount < 1) return 1;
  if (!Number.isInteger(digitIndex) || digitIndex < 0 || digitIndex >= paddedDigitCount) return 1;
  return 10 ** (paddedDigitCount - digitIndex - 1);
}

export function stepFrequencyDigit(hz: number, paddedDigitCount: number, digitIndex: number, direction: 1 | -1): number {
  return clampFrequencyHz(hz + digitStepForIndex(paddedDigitCount, digitIndex) * direction);
}

export function snapTuneFrequencyHz(hz: number, stepHz = 10): number {
  const step = Math.max(1, stepHz);
  return Math.max(0, Math.round(Number(hz) / step) * step);
}

export function formatFrequencyPlain(hz: number): string {
  const text = Math.round(hz).toString().padStart(7, '0');
  const whole = text.slice(0, -6) || '0';
  const khz = text.slice(-6, -3);
  const rest = text.slice(-3);
  return `${whole}.${khz}.${rest}`;
}

export function safeFiniteNumber(value: unknown, fallback = 0): number {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

export function safeFixed(value: unknown, digits: number, fallbackText: string): string {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric.toFixed(digits) : fallbackText;
}
