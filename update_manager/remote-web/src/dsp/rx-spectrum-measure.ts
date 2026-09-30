/** Relative RX IQ spectrum measurements. Levels are FFT dB, not calibrated dBm. */
export type RxSpectrumSettings = {
  centerHz: number;
  sampleRateHz: number;
  fftSize: number;
  adc: number;
  antenna: number;
  attenuationDb: number;
  agcMode: string;
  agcGain: number;
  noiseReductionMode: string;
  noiseBlankerMode: string;
  mode: string;
  filterLowHz: number;
  filterHighHz: number;
  filterShiftHz: number;
  passbandLowHz: number;
  passbandHighHz: number;
  source: 'raw-rx-iq';
};

export type RxSpectrumCapture = {
  format: 'saturn-rx-spectrum-v1';
  firmwareLabel: string;
  startedAtIso: string;
  endedAtIso: string;
  durationMs: number;
  sampleIntervalMs: number;
  samples: number;
  quality: 'good' | 'limited';
  settings: RxSpectrumSettings;
  units: 'relative FFT dB';
  summary: {
    adjacentNoiseFloorMedianDb: number;
    adjacentNoiseFloorP90Db: number;
    passbandLevelMedianDb: number;
    passbandExcessMedianDb: number;
    widebandPowerMedianDb: number;
    widebandPowerP99Db: number;
    widebandBurstRiseDb: number;
  };
  meanSpectrumDb: number[];
};

const SAMPLE_INTERVAL_MS = 100;
const CENTER_GUARD_HZ = 250;
const NOISE_HALF_WIDTH_HZ = 10_000;
const PASSBAND_GUARD_HZ = 250;

function roundDb(value: number): number {
  return Math.round(value * 100) / 100;
}

function percentile(values: number[], fraction: number): number {
  if (!values.length) return NaN;
  const sorted = values.slice().sort((a, b) => a - b);
  const position = (sorted.length - 1) * fraction;
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  if (lower === upper) return sorted[lower] ?? NaN;
  return (sorted[lower] ?? 0) * (upper - position) +
    (sorted[upper] ?? 0) * (position - lower);
}

function dbFromPower(power: number): number {
  return 10 * Math.log10(Math.max(1e-16, power));
}

/** Exact RX conditions that must match for a meaningful paired comparison. */
export function rxSpectrumSettingsMatch(a: RxSpectrumSettings, b: RxSpectrumSettings): boolean {
  return (Object.keys(a) as Array<keyof RxSpectrumSettings>).every((key) => a[key] === b[key]);
}

export class RxSpectrumAccumulator {
  readonly settings: RxSpectrumSettings;
  readonly firmwareLabel: string;
  readonly durationMs: number;
  readonly startedAtIso: string;
  readonly startedAtMs: number;
  private readonly sumPower: Float64Array;
  private readonly noiseDb: number[] = [];
  private readonly passbandDb: number[] = [];
  private readonly passbandExcessDb: number[] = [];
  private readonly widebandDb: number[] = [];
  private nextSampleAtMs: number;
  private lastSampleAtMs: number;

  constructor(settings: RxSpectrumSettings, firmwareLabel: string, startedAtMs: number, durationMs = 30_000) {
    if (!Number.isInteger(settings.fftSize) || settings.fftSize < 64 ||
        !Number.isFinite(settings.sampleRateHz) || settings.sampleRateHz <= 0 ||
        !Number.isFinite(startedAtMs) || !Number.isFinite(durationMs) || durationMs < 1000) {
      throw new Error('Invalid RX spectrum capture settings');
    }
    this.settings = { ...settings };
    this.firmwareLabel = firmwareLabel.trim().slice(0, 80) || 'unlabeled';
    this.durationMs = durationMs;
    this.startedAtMs = startedAtMs;
    this.startedAtIso = new Date().toISOString();
    this.sumPower = new Float64Array(settings.fftSize);
    this.nextSampleAtMs = startedAtMs;
    this.lastSampleAtMs = startedAtMs;
  }

  get samples(): number { return this.widebandDb.length; }
  get elapsedMs(): number { return Math.max(0, this.lastSampleAtMs - this.startedAtMs); }
  shouldSample(atMs: number): boolean {
    return Number.isFinite(atMs) && atMs >= this.nextSampleAtMs &&
      atMs <= this.startedAtMs + this.durationMs;
  }

  /** Input bins must be full-span, low-to-high frequency, before display processing. */
  addFrame(bins: Float32Array, atMs: number, currentSettings: RxSpectrumSettings): boolean {
    if (!rxSpectrumSettingsMatch(this.settings, currentSettings)) {
      throw new Error('RX settings changed during capture');
    }
    if (bins.length !== this.sumPower.length) {
      throw new Error('FFT size changed during capture');
    }
    if (!this.shouldSample(atMs)) {
      return false;
    }

    const noiseCandidates: number[] = [];
    let passbandPower = 0;
    let passbandCount = 0;
    let widebandPower = 0;
    let widebandCount = 0;
    const { sampleRateHz, passbandLowHz, passbandHighHz } = this.settings;
    const binHz = sampleRateHz / bins.length;
    for (let i = 0; i < bins.length; i += 1) {
      const db = bins[i];
      if (db === undefined || !Number.isFinite(db)) {
        throw new Error('Invalid FFT bin during capture');
      }
      const power = 10 ** (Math.max(-160, Math.min(80, db)) / 10);
      this.sumPower[i] = (this.sumPower[i] ?? 0) + power;
      const offsetHz = (i - bins.length / 2) * binHz;
      if (Math.abs(offsetHz) >= CENTER_GUARD_HZ) {
        widebandPower += power;
        widebandCount += 1;
      }
      if (offsetHz >= passbandLowHz && offsetHz <= passbandHighHz) {
        passbandPower += power;
        passbandCount += 1;
      }
      if (Math.abs(offsetHz) <= NOISE_HALF_WIDTH_HZ &&
          Math.abs(offsetHz) >= CENTER_GUARD_HZ &&
          (offsetHz < passbandLowHz - PASSBAND_GUARD_HZ ||
           offsetHz > passbandHighHz + PASSBAND_GUARD_HZ)) {
        noiseCandidates.push(db);
      }
    }
    if (!noiseCandidates.length || !passbandCount || !widebandCount) {
      throw new Error('RX passband or adjacent noise region is outside the captured spectrum');
    }
    const noise = percentile(noiseCandidates, 0.2);
    const passband = dbFromPower(passbandPower / passbandCount);
    this.noiseDb.push(noise);
    this.passbandDb.push(passband);
    this.passbandExcessDb.push(passband - noise);
    this.widebandDb.push(dbFromPower(widebandPower / widebandCount));
    this.lastSampleAtMs = atMs;
    this.nextSampleAtMs = atMs + SAMPLE_INTERVAL_MS;
    return true;
  }

  finish(endedAtIso = new Date().toISOString()): RxSpectrumCapture {
    if (this.samples === 0) throw new Error('No RX IQ frames were sampled');
    const widebandMedian = percentile(this.widebandDb, 0.5);
    const widebandP99 = percentile(this.widebandDb, 0.99);
    const expectedSamples = this.durationMs / SAMPLE_INTERVAL_MS;
    return {
      format: 'saturn-rx-spectrum-v1',
      firmwareLabel: this.firmwareLabel,
      startedAtIso: this.startedAtIso,
      endedAtIso,
      durationMs: this.durationMs,
      sampleIntervalMs: SAMPLE_INTERVAL_MS,
      samples: this.samples,
      quality: this.samples >= expectedSamples * 0.6 && this.elapsedMs >= this.durationMs * 0.8
        ? 'good' : 'limited',
      settings: { ...this.settings },
      units: 'relative FFT dB',
      summary: {
        adjacentNoiseFloorMedianDb: roundDb(percentile(this.noiseDb, 0.5)),
        adjacentNoiseFloorP90Db: roundDb(percentile(this.noiseDb, 0.9)),
        passbandLevelMedianDb: roundDb(percentile(this.passbandDb, 0.5)),
        passbandExcessMedianDb: roundDb(percentile(this.passbandExcessDb, 0.5)),
        widebandPowerMedianDb: roundDb(widebandMedian),
        widebandPowerP99Db: roundDb(widebandP99),
        widebandBurstRiseDb: roundDb(widebandP99 - widebandMedian),
      },
      meanSpectrumDb: Array.from(this.sumPower, (power) => roundDb(dbFromPower(power / this.samples))),
    };
  }
}

export function compareRxSpectrumCaptures(baseline: RxSpectrumCapture, candidate: RxSpectrumCapture) {
  if (baseline?.format !== 'saturn-rx-spectrum-v1' || candidate?.format !== 'saturn-rx-spectrum-v1' ||
      !baseline.settings || !candidate.settings || !baseline.summary || !candidate.summary ||
      !Array.isArray(baseline.meanSpectrumDb) || !Array.isArray(candidate.meanSpectrumDb) ||
      !rxSpectrumSettingsMatch(baseline.settings, candidate.settings) ||
      baseline.durationMs !== candidate.durationMs ||
      baseline.sampleIntervalMs !== candidate.sampleIntervalMs ||
      baseline.meanSpectrumDb.length !== baseline.settings.fftSize ||
      candidate.meanSpectrumDb.length !== candidate.settings.fftSize) {
    throw new Error('Captures are invalid or settings/duration differ; use the same frequency, rate, antenna, attenuation, gain, mode, filter, and duration');
  }
  const finite = [
    baseline.summary.adjacentNoiseFloorMedianDb, candidate.summary.adjacentNoiseFloorMedianDb,
    baseline.summary.passbandExcessMedianDb, candidate.summary.passbandExcessMedianDb,
    baseline.summary.widebandBurstRiseDb, candidate.summary.widebandBurstRiseDb,
  ];
  if (!finite.every(Number.isFinite)) throw new Error('Capture metrics are invalid');
  return {
    baselineLabel: baseline.firmwareLabel,
    candidateLabel: candidate.firmwareLabel,
    baselineQuality: baseline.quality,
    candidateQuality: candidate.quality,
    baselineSamples: baseline.samples,
    candidateSamples: candidate.samples,
    settings: candidate.settings,
    noiseFloorDeltaDb: roundDb(candidate.summary.adjacentNoiseFloorMedianDb - baseline.summary.adjacentNoiseFloorMedianDb),
    passbandExcessDeltaDb: roundDb(candidate.summary.passbandExcessMedianDb - baseline.summary.passbandExcessMedianDb),
    burstRiseDeltaDb: roundDb(candidate.summary.widebandBurstRiseDb - baseline.summary.widebandBurstRiseDb),
    caveat: 'Relative FFT levels only. On-air changes in propagation, static, and signals can dominate firmware differences.',
  };
}
