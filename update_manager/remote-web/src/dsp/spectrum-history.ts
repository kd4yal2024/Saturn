export type SpectrumMapping = {
  receiver: string; centerHz: number; spanHz: number; sampleRate: number;
  sourceBins: number; units: 'relative dB';
};
export type SpectrumFrame = SpectrumMapping & { timestamp: number; sequence: number; bins: Float32Array };
export type HistoryRow = SpectrumMapping & { timestamp: number; lastTimestamp: number; sequence: number; samples: number; epoch: number };
export const MISSING_LEVEL = -1000;
/**
 * Authoritative, owned numerical ring. Left-to-right bins have ALREADY passed
 * visibleBinsForDisplay. Newest is head; age increases backwards in the ring.
 * Each cadence bucket holds sampled per-bin maxima (display only, not power).
 * Empty timestamp buckets remain missing, including pause/resume gaps. No RAF
 * code can append rows. Mapping/cadence changes explicitly start a new segment.
 */
export class SpectrumHistory {
  readonly capacity = 512;
  readonly maxBytes = 32 * 1024 * 1024;
  width = 0; head = -1; count = 0; epoch = 0; revision = 0;
  cadenceMs = 50; accepted = 0; aggregated = 0; missing = 0; rejected = 0;
  boundary = 'Waiting for live spectrum';
  data = new Float32Array(0); latestRaw = new Float32Array(0);
  rows: (HistoryRow | null)[] = Array(this.capacity).fill(null);
  versions = new Float64Array(this.capacity);
  private mapping = ''; private bucket = -Infinity; private lastSequence = -1;
  private lastTimestamp = -Infinity; private firstTimestamp = NaN;
  coalescedSourceUpdates = 0;
  clear(reason: string): void {
    this.head = -1; this.count = 0; this.bucket = -Infinity;
    this.lastSequence = -1; this.lastTimestamp = -Infinity; this.firstTimestamp = NaN;
    this.rows.fill(null); this.data.fill(MISSING_LEVEL); this.latestRaw.fill(MISSING_LEVEL);
    this.epoch++; this.revision++; this.versions.fill(this.revision); this.boundary = reason;
  }
  accept(frame: SpectrumFrame, cadenceMs: number): boolean {
    const { bins, timestamp, sequence } = frame;
    if (!bins.length || bins.byteLength * this.capacity > this.maxBytes ||
        !Number.isFinite(timestamp) || !Number.isFinite(sequence) ||
        !Number.isFinite(frame.centerHz) || !(frame.spanHz > 0) || !Number.isFinite(frame.spanHz) ||
        !(frame.sampleRate > 0) || frame.units !== 'relative dB') { this.rejected++; return false; }
    const cadence = Math.max(16, Math.min(2000, Number.isFinite(cadenceMs) ? cadenceMs : 50));
    const mapping = JSON.stringify([frame.receiver, frame.centerHz, frame.spanHz, frame.sampleRate, frame.sourceBins, bins.length, frame.units, cadence]);
    if (mapping !== this.mapping) {
      this.mapping = mapping; this.cadenceMs = cadence;
      if (bins.length !== this.width) {
        this.width = bins.length;
        this.data = new Float32Array(this.width * this.capacity);
        this.latestRaw = new Float32Array(this.width);
      }
      this.clear('New frequency / receiver / resolution / cadence segment');
    }
    if (sequence <= this.lastSequence || timestamp < this.lastTimestamp) return false;
    if (this.lastSequence >= 0) this.coalescedSourceUpdates += Math.max(0, sequence - this.lastSequence - 1);
    if (!Number.isFinite(this.firstTimestamp)) this.firstTimestamp = timestamp;
    this.lastSequence = sequence; this.lastTimestamp = timestamp;
    this.latestRaw.set(bins); this.accepted++;
    const bucket = Math.floor(timestamp / cadence);
    const steps = this.head < 0 ? 1 : Math.max(0, bucket - this.bucket);
    if (steps === 0) this.aggregated++;
    if (steps > 1) this.missing += steps - 1;
    for (let i = 0; i < Math.min(this.capacity, steps); i++) {
      this.head = (this.head + 1) % this.capacity;
      this.data.fill(MISSING_LEVEL, this.head * this.width, (this.head + 1) * this.width);
      this.rows[this.head] = null; this.versions[this.head] = ++this.revision;
      this.count = Math.min(this.capacity, this.count + 1);
    }
    this.bucket = bucket;
    const offset = this.head * this.width;
    for (let i = 0; i < this.width; i++) {
      const value = bins[i]!;
      if (Number.isFinite(value)) this.data[offset + i] = Math.max(this.data[offset + i]!, value);
    }
    const old = this.rows[this.head];
    this.rows[this.head] = { receiver: frame.receiver, centerHz: frame.centerHz, spanHz: frame.spanHz,
      sampleRate: frame.sampleRate, sourceBins: frame.sourceBins, units: frame.units,
      timestamp: old?.timestamp ?? timestamp, lastTimestamp: timestamp, sequence, samples: (old?.samples ?? 0) + 1, epoch: this.epoch };
    this.versions[this.head] = ++this.revision;
    return true;
  }
  physical(age: number): number { return (this.head - age + this.capacity) % this.capacity; }
  row(age: number): Float32Array | null {
    if (age < 0 || age >= this.count || this.head < 0) return null;
    const p = this.physical(age);
    return this.rows[p] ? this.data.subarray(p * this.width, (p + 1) * this.width) : null;
  }
  /** Sampled maximum for visual reduction; never used as integrated power. */
  peak(age: number, column: number, columns: number, smoothing = 0): number {
    const row = this.row(age);
    if (!row) return MISSING_LEVEL;
    const start = Math.floor(column * this.width / columns);
    const end = Math.max(start + 1, Math.floor((column + 1) * this.width / columns));
    let peak = MISSING_LEVEL;
    for (let i = start; i < Math.min(end, this.width); i++) peak = Math.max(peak, row[i]!);
    if (smoothing > 0 && peak > MISSING_LEVEL) {
      const left = row[Math.max(0, start - 1)]!, right = row[Math.min(this.width - 1, end)]!;
      peak += ((left + peak + right) / 3 - peak) * smoothing;
    }
    return peak;
  }
  get sourceRateHz(): number {
    const rows = this.rows.filter((r): r is HistoryRow => r !== null);
    const samples = rows.reduce((n,r)=>n+r.samples,0);
    const start = rows.reduce((n,r)=>Math.min(n,r.timestamp),Infinity);
    return this.lastTimestamp > start ? (samples-1)*1000/(this.lastTimestamp-start) : 0;
  }
  get retainedMs(): number { return Math.max(0, this.count - 1) * this.cadenceMs; }
  get bytes(): number { return this.data.byteLength + this.latestRaw.byteLength + this.versions.byteLength + this.capacity * 128; }
}

/** On-demand CPU diagnostics; never reads GPU pixels or changes source levels. */
export function levelStatistics(levels: Float32Array, floor: number, ceiling: number) {
  const values = Array.from(levels).filter(v => Number.isFinite(v) && v > -999).sort((a,b) => a-b);
  if (!values.length) return null;
  const q = (fraction: number) => values[Math.floor((values.length-1)*fraction)]!;
  const normalizedMedian = (q(.5)-floor)/(ceiling-floor);
  return { samples: values.length, min: values[0], median: q(.5), p95: q(.95), max: values[values.length-1],
    normalizedMedian, clampedNormalizedMedian: Math.max(0,Math.min(1,normalizedMedian)),
    clippedFloorPercent: 100*values.filter(v=>v<=floor).length/values.length,
    clippedCeilingPercent: 100*values.filter(v=>v>=ceiling).length/values.length };
}
