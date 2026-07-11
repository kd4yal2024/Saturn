export type BandEdge = {
  start: number;
  end: number;
  label: string;
};

export const HAM_BAND_EDGES_HZ: readonly BandEdge[] = Object.freeze([
  { start: 1800000, end: 2000000, label: '160m' },
  { start: 3500000, end: 4000000, label: '80m' },
  { start: 5330500, end: 5403500, label: '60m' },
  { start: 7000000, end: 7300000, label: '40m' },
  { start: 10100000, end: 10150000, label: '30m' },
  { start: 14000000, end: 14350000, label: '20m' },
  { start: 18068000, end: 18168000, label: '17m' },
  { start: 21000000, end: 21450000, label: '15m' },
  { start: 24890000, end: 24990000, label: '12m' },
  { start: 28000000, end: 29700000, label: '10m' },
  { start: 50000000, end: 54000000, label: '6m' },
  { start: 88000000, end: 108000000, label: 'FM' },
]);

export function bandKeyForFrequency(hz: number): string {
  const freq = Number(hz);
  const band = HAM_BAND_EDGES_HZ.find((b) => freq >= b.start && freq <= b.end);
  return band ? band.label : '';
}

export function adcLabel(value: number): string {
  const adc = Math.max(0, Math.min(2, Math.round(Number(value) || 0)));
  if (adc === 2) return 'ADC3';
  return adc === 1 ? 'ADC2' : 'ADC1';
}

export function antennaLabel(value: number): string {
  const ant = Math.max(1, Math.min(3, Math.round(Number(value) || 1)));
  return `ANT${ant}`;
}

export function normalizeStreamMode(value: unknown): 'lan' | 'wan' {
  return `${value || ''}`.trim().toLowerCase() === 'wan' ? 'wan' : 'lan';
}

/** Human-readable noise reduction label. */
export function noiseReductionLabel(mode: string, level: number): string {
  if (mode === 'OFF') return 'Off';
  if (mode === 'NR3') return 'NR3 fixed';
  return `${mode} ${level}%`;
}

/** Human-readable description of a mic capture error. */
export function describeMicCaptureError(
  err: unknown,
  isSecureContext: boolean,
): string {
  const e = err as { name?: string; message?: string } | null;
  const name = String(e?.name ?? '').trim();
  if (name === 'MicUnavailableError') return e?.message ?? 'Microphone unavailable';
  if (name === 'NotAllowedError' || name === 'PermissionDeniedError') {
    return isSecureContext
      ? 'Microphone access was denied by the browser'
      : 'Microphone access is blocked on insecure HTTP pages; open the remote over HTTPS to use the phone microphone';
  }
  if (name === 'NotFoundError' || name === 'DevicesNotFoundError') return 'No microphone was found on this device';
  if (name === 'NotReadableError' || name === 'TrackStartError') return 'The browser could not start the microphone';
  if (e?.message) return e.message;
  return 'Unknown microphone capture failure';
}

export function effectiveIqSampleRate(
  sampleRate: number,
  streamMode: 'lan' | 'wan',
  wanCap: number = 96000,
): number {
  return streamMode === 'wan' ? Math.min(sampleRate, wanCap) : sampleRate;
}
