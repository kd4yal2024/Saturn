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

export type BandGroup = 'ham' | 'swl';

// Receive navigation ranges, not a transmit-permission/band-plan policy.
// Prefix memory keys because amateur and broadcast meter-band names overlap.
export const SWL_BANDS = Object.freeze([
  { start: 150000, end: 1710000, label: 'LW/MW', frequency: 1000000 },
  { start: 2300000, end: 2495000, label: '120m', frequency: 2400000 },
  { start: 3200000, end: 3400000, label: '90m', frequency: 3300000 },
  { start: 3900000, end: 4000000, label: '75m', frequency: 3950000 },
  { start: 4750000, end: 5060000, label: '60m', frequency: 4850000 },
  { start: 5900000, end: 6200000, label: '49m', frequency: 6000000 },
  { start: 7200000, end: 7450000, label: '41m', frequency: 7350000 },
  { start: 9400000, end: 9900000, label: '31m', frequency: 9600000 },
  { start: 11600000, end: 12100000, label: '25m', frequency: 11800000 },
  { start: 13570000, end: 13870000, label: '22m', frequency: 13700000 },
  { start: 15100000, end: 15800000, label: '19m', frequency: 15300000 },
  { start: 17480000, end: 17900000, label: '16m', frequency: 17700000 },
  { start: 18900000, end: 19020000, label: '15m', frequency: 18950000 },
  { start: 21450000, end: 21850000, label: '13m', frequency: 21600000 },
  { start: 25670000, end: 26100000, label: '11m', frequency: 25900000 },
]);
export const SWL_BAND_EDGES_HZ: readonly BandEdge[] = Object.freeze(
  SWL_BANDS.map(band => ({ start: band.start, end: band.end, label: `SWL ${band.label}` })),
);
export const BAND_MEMORY_LABELS = new Set([
  ...HAM_BAND_EDGES_HZ.map(band => band.label), ...SWL_BAND_EDGES_HZ.map(band => band.label),
]);

export function normalizeBandGroup(value: unknown): BandGroup {
  return value === 'swl' ? 'swl' : 'ham';
}

export function bandKeyForFrequency(hz: number, group: BandGroup = 'ham'): string {
  const freq = Number(hz);
  const first = group === 'swl' ? SWL_BAND_EDGES_HZ : HAM_BAND_EDGES_HZ;
  const second = group === 'swl' ? HAM_BAND_EDGES_HZ : SWL_BAND_EDGES_HZ;
  const band = first.find((b) => freq >= b.start && freq <= b.end)
    || second.find((b) => freq >= b.start && freq <= b.end);
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
  if (mode === 'NNR') return 'NNR Standard ' + level + '%';
  if (mode === 'NNR_PREMIUM') return 'NNR Premium ' + level + '%';
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
