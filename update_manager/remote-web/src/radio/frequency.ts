export const MIN_FREQUENCY_HZ = 0;
export const MAX_FREQUENCY_HZ = 64_000_000;

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

export function digitStepForIndex(paddedDigitCount: number, digitIndex: number): number {
  if (!Number.isInteger(paddedDigitCount) || paddedDigitCount < 1) return 1;
  if (!Number.isInteger(digitIndex) || digitIndex < 0 || digitIndex >= paddedDigitCount) return 1;
  return 10 ** (paddedDigitCount - digitIndex - 1);
}

export function stepFrequencyDigit(hz: number, paddedDigitCount: number, digitIndex: number, direction: 1 | -1): number {
  return clampFrequencyHz(hz + digitStepForIndex(paddedDigitCount, digitIndex) * direction);
}
