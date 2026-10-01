import { describe, expect, it } from 'vitest';
import {
  RxSpectrumAccumulator,
  compareRxSpectrumCaptures,
  type RxSpectrumFpgaIdentity,
  type RxSpectrumSettings,
} from '../src/dsp/rx-spectrum-measure';

const settings: RxSpectrumSettings = {
  centerHz: 3_905_000,
  sampleRateHz: 48_000,
  fftSize: 1024,
  adc: 0,
  antenna: 1,
  attenuationDb: 10,
  agcMode: 'MEDIUM',
  agcGain: 63,
  noiseReductionMode: 'off',
  noiseBlankerMode: 'off',
  mode: 'USB',
  filterLowHz: 500,
  filterHighHz: 3000,
  filterShiftHz: 0,
  passbandLowHz: 500,
  passbandHighHz: 3000,
  source: 'raw-rx-iq',
};

function frame(noiseDb = -100, passbandDb = -80): Float32Array {
  const bins = new Float32Array(settings.fftSize).fill(noiseDb);
  const binHz = settings.sampleRateHz / bins.length;
  for (let i = 0; i < bins.length; i += 1) {
    const offset = (i - bins.length / 2) * binHz;
    if (offset >= settings.passbandLowHz && offset <= settings.passbandHighHz) bins[i] = passbandDb;
  }
  bins[settings.fftSize / 2] = -20; // DC spur must not contaminate the adjacent floor.
  return bins;
}

function capture(label: string, noiseDb = -100, passbandDb = -80) {
  const recorder = new RxSpectrumAccumulator(settings, identity(label), 0, 3000);
  for (let ms = 0; ms <= 3000; ms += 100) recorder.addFrame(frame(noiseDb, passbandDb), ms, settings);
  return recorder.finish();
}

function identity(rxFilter = '22/Q24'): RxSpectrumFpgaIdentity {
  return { firmware: '1.30.002', subversion: 2, rxFilter, buildIdRaw: 0x53460002,
    buildIdHex: '0x53460002', status: 'identified' };
}

describe('RX spectrum measurement', () => {
  it('measures raw IQ FFT levels before display range, smoothing, or palette', () => {
    const result = capture('22/Q24');
    expect(result.samples).toBe(31);
    expect(result.quality).toBe('good');
    expect(result.summary.adjacentNoiseFloorMedianDb).toBeCloseTo(-100, 1);
    expect(result.summary.passbandLevelMedianDb).toBeCloseTo(-80, 1);
    expect(result.summary.passbandExcessMedianDb).toBeCloseTo(20, 1);
    expect(result.summary.widebandBurstRiseDb).toBeCloseTo(0, 1);
    expect(result.meanSpectrumDb).toHaveLength(settings.fftSize);
  });

  it('samples at most ten frames per second and reveals burst rise', () => {
    const recorder = new RxSpectrumAccumulator(settings, identity(), 0, 3000);
    for (let ms = 0; ms <= 3000; ms += 50) {
      recorder.addFrame(frame(ms === 1500 ? -50 : -100), ms, settings);
    }
    const result = recorder.finish();
    expect(result.samples).toBe(31);
    expect(result.summary.widebandBurstRiseDb).toBeGreaterThan(20);
  });

  it('marks sparse captures limited without inventing percentile values', () => {
    const recorder = new RxSpectrumAccumulator(settings, identity(), 0, 3000);
    recorder.addFrame(frame(), 0, settings);
    const result = recorder.finish();
    expect(result.quality).toBe('limited');
    expect(result.summary.adjacentNoiseFloorMedianDb).toBe(-100);
    expect(result.summary.widebandBurstRiseDb).toBe(0);
  });

  it('records an unidentified build without assigning it a filter', () => {
    const recorder = new RxSpectrumAccumulator(settings, {
      firmware: '1.30 — build unidentified',
      subversion: null,
      rxFilter: null,
      buildIdRaw: 0x53460003,
      buildIdHex: '0x53460003',
      status: 'unidentified',
    }, 0, 3000);
    recorder.addFrame(frame(), 0, settings);
    const result = recorder.finish();
    expect(result.fpgaIdentity.rxFilter).toBeNull();
    expect(result.fpgaIdentity.buildIdHex).toBe('0x53460003');
    expect(result.firmwareLabel).toContain('RX filter unidentified');
  });

  it('rejects changed receive settings and mismatched comparison files', () => {
    const recorder = new RxSpectrumAccumulator(settings, identity(), 0, 3000);
    expect(() => recorder.addFrame(frame(), 0, { ...settings, attenuationDb: 20 })).toThrow(/settings changed/);
    const baseline = capture('18/Q20', -100, -80);
    const candidate = capture('22/Q24', -103, -78);
    const result = compareRxSpectrumCaptures(baseline, candidate);
    expect(result.noiseFloorDeltaDb).toBeCloseTo(-3, 1);
    expect(result.passbandExcessDeltaDb).toBeCloseTo(5, 1);
    expect(() => compareRxSpectrumCaptures(baseline, {
      ...candidate, settings: { ...candidate.settings, centerHz: candidate.settings.centerHz + 100 },
    })).toThrow(/settings\/duration differ/);
    expect(() => compareRxSpectrumCaptures(baseline, {
      ...candidate, durationMs: 120_000,
    })).toThrow(/settings\/duration differ/);
  });
});
