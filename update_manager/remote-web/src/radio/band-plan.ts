// Display guidance only. These ranges never participate in TX admission or
// band-memory selection. Frequencies are RF Hz; channel dial frequencies are
// identified separately in their labels.
export type BandUse = 'cw' | 'data' | 'phone' | 'beacon' | 'mixed' | 'fm' | 'broadcast';
export type BandSegment = { start: number; end: number; band: string; label: string; use: BandUse };
export const HAM_PLAN_SOURCE = 'https://www.arrl.org/band-plan';
export const HAM_PLAN_REVIEWED = '2026-11-17';
export const HAM_SEGMENTS: readonly BandSegment[] = [
  { start: 1800000, end: 1810000, band: '160m', label: 'CW / data', use: 'data' },
  { start: 1810000, end: 1843000, band: '160m', label: 'CW', use: 'cw' },
  { start: 1843000, end: 2000000, band: '160m', label: 'Phone / mixed', use: 'phone' },
  { start: 3500000, end: 3570000, band: '80m', label: 'CW', use: 'cw' },
  { start: 3570000, end: 3600000, band: '80m', label: 'Data / CW', use: 'data' },
  { start: 3600000, end: 4000000, band: '80m', label: 'Phone / CW', use: 'phone' },
  // ARRL's current US guide: four 2.8 kHz channels and the 15 kHz allocation.
  { start: 5330600, end: 5333400, band: '60m', label: 'USB / data · dial 5330.5', use: 'mixed' },
  { start: 5346600, end: 5349400, band: '60m', label: 'USB / data · dial 5346.5', use: 'mixed' },
  { start: 5351500, end: 5366500, band: '60m', label: 'USB / data · 15 kHz', use: 'mixed' },
  { start: 5371600, end: 5374400, band: '60m', label: 'USB / data · dial 5371.5', use: 'mixed' },
  { start: 5403600, end: 5406400, band: '60m', label: 'USB / data · dial 5403.5', use: 'mixed' },
  { start: 7000000, end: 7080000, band: '40m', label: 'CW / narrow data', use: 'cw' },
  { start: 7080000, end: 7125000, band: '40m', label: 'Data / CW', use: 'data' },
  { start: 7125000, end: 7300000, band: '40m', label: 'Phone / CW', use: 'phone' },
  { start: 10100000, end: 10130000, band: '30m', label: 'CW', use: 'cw' },
  { start: 10130000, end: 10150000, band: '30m', label: 'Data / CW', use: 'data' },
  { start: 14000000, end: 14070000, band: '20m', label: 'CW', use: 'cw' },
  { start: 14070000, end: 14099500, band: '20m', label: 'Data / CW', use: 'data' },
  { start: 14099500, end: 14100500, band: '20m', label: 'Beacons', use: 'beacon' },
  { start: 14100500, end: 14150000, band: '20m', label: 'Data / CW', use: 'data' },
  { start: 14150000, end: 14350000, band: '20m', label: 'Phone / CW', use: 'phone' },
  { start: 18068000, end: 18100000, band: '17m', label: 'CW', use: 'cw' },
  { start: 18100000, end: 18110000, band: '17m', label: 'Data / CW', use: 'data' },
  { start: 18110000, end: 18168000, band: '17m', label: 'Phone / CW', use: 'phone' },
  { start: 21000000, end: 21070000, band: '15m', label: 'CW', use: 'cw' },
  { start: 21070000, end: 21110000, band: '15m', label: 'Data / CW', use: 'data' },
  { start: 21110000, end: 21200000, band: '15m', label: 'CW / data', use: 'mixed' },
  { start: 21200000, end: 21450000, band: '15m', label: 'Phone / CW', use: 'phone' },
  { start: 24890000, end: 24920000, band: '12m', label: 'CW', use: 'cw' },
  { start: 24920000, end: 24930000, band: '12m', label: 'Data / CW', use: 'data' },
  { start: 24930000, end: 24990000, band: '12m', label: 'Phone / CW', use: 'phone' },
  { start: 28000000, end: 28070000, band: '10m', label: 'CW', use: 'cw' },
  { start: 28070000, end: 28150000, band: '10m', label: 'Data / CW', use: 'data' },
  { start: 28150000, end: 28200000, band: '10m', label: 'CW', use: 'cw' },
  { start: 28200000, end: 28300000, band: '10m', label: 'Beacons', use: 'beacon' },
  { start: 28300000, end: 29000000, band: '10m', label: 'Phone', use: 'phone' },
  { start: 29000000, end: 29200000, band: '10m', label: 'AM', use: 'phone' },
  { start: 29200000, end: 29300000, band: '10m', label: 'Phone', use: 'phone' },
  { start: 29300000, end: 29510000, band: '10m', label: 'Satellite', use: 'mixed' },
  { start: 29520000, end: 29590000, band: '10m', label: 'FM repeater in', use: 'fm' },
  { start: 29590000, end: 29610000, band: '10m', label: 'FM simplex', use: 'fm' },
  { start: 29610000, end: 29700000, band: '10m', label: 'FM repeater out', use: 'fm' },
  { start: 50000000, end: 50100000, band: '6m', label: 'CW / beacons', use: 'cw' },
  { start: 50100000, end: 50300000, band: '6m', label: 'SSB / CW', use: 'phone' },
  { start: 50300000, end: 50600000, band: '6m', label: 'All modes', use: 'mixed' },
  { start: 50600000, end: 50800000, band: '6m', label: 'Data', use: 'data' },
  { start: 50800000, end: 51100000, band: '6m', label: 'Control / DX', use: 'mixed' },
  { start: 51120000, end: 51480000, band: '6m', label: 'FM repeater in', use: 'fm' },
  { start: 51500000, end: 51600000, band: '6m', label: 'FM simplex', use: 'fm' },
  { start: 51620000, end: 51980000, band: '6m', label: 'FM repeater out', use: 'fm' },
  { start: 52000000, end: 54000000, band: '6m', label: 'FM / mixed', use: 'fm' },
];

export type VisibleBandSegment = BandSegment & { left: number; width: number };
export function visibleBandSegments(
  segments: readonly BandSegment[], low: number, high: number,
): VisibleBandSegment[] {
  if (!Number.isFinite(low) || !Number.isFinite(high) || high <= low) return [];
  return segments.filter(s => s.end > low && s.start < high).map(s => ({
    ...s,
    left: 100 * (Math.max(low, s.start) - low) / (high - low),
    width: 100 * (Math.min(high, s.end) - Math.max(low, s.start)) / (high - low),
  }));
}
