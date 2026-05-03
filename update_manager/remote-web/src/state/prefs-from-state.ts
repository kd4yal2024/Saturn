import type { RadioPrefs, DisplayPrefs } from '../settings/types';
import {
  clampSampleRateHz,
  clampRxAdc,
  clampRxAntenna,
  clampRxVolumeDb,
  normalizeNrMode,
  clampRxNoiseReductionLevel,
  normalizeNbMode,
  clampRxNbThreshold,
  clampDspTapCount,
  clampDspDelay,
  clampDspGain,
  clampDspLeakage,
  normalizeAgcMode,
  clampAgcGain,
  clampFilterLowHz,
  clampFilterHighHz,
  clampTxDriveWatts,
  clampTxMicGainDb,
  clampTwoToneFreqHz,
  clampTwoToneLevelDb,
  clampTwoToneDelayMs,
  sanitizeEqBands,
  sanitizeCfcBands,
  normalizeTxMeterMode,
  clampDisplayDb,
  clampSpectrumAverage,
  clampWaterfallSpeed,
  normalizeWaterfallPalette,
} from '../settings/normalize';
import { clampDemodMode } from '../radio/passband';

export interface RadioPrefsSource {
  sampleRate: number;
  rxAdc: number;
  rxAntenna: number;
  mode: string;
  rxVolumeDb: number;
  rxNoiseReductionMode: string;
  rxNoiseReductionLevel: number;
  rxNbMode: string;
  rxNbThreshold: number;
  rxAnrTaps: number;
  rxAnrDelay: number;
  rxAnrGain: number;
  rxAnrLeakage: number;
  anfEnabled: boolean;
  rxAnfTaps: number;
  rxAnfDelay: number;
  rxAnfGain: number;
  rxAnfLeakage: number;
  agcMode: string;
  agcGain: number;
  filterLow: number;
  filterHigh: number;
  txDrive: number;
  txMicGainDb: number;
  txFilterLow: number;
  txFilterHigh: number;
  rxEqEnabled: boolean;
  rxEqBands: number[];
  txEqEnabled: boolean;
  txEqBands: number[];
  cfcEnabled: boolean;
  cfcPrecomp: number;
  cfcBands: number[];
  txMeterMode: string;
  twoToneEnabled: boolean;
  txTwoToneFreq1: number;
  txTwoToneFreq2: number;
  txTwoToneLevelDb: number;
  txTwoToneInvertLsb: boolean;
  txTwoToneDelayMs: number;
  txNoiseGateEnabled: boolean;
  txNoiseGateThresholdDb: number;
  txTimeoutEnabled: boolean;
  txTimeoutSeconds: number;
}

export interface DisplayPrefsSource {
  spectrumAutoRange: boolean;
  displayFloorDb: number;
  displayCeilingDb: number;
  waterfallAutoRange: boolean;
  waterfallFloorDb: number;
  waterfallCeilingDb: number;
  spectrumAverage: number;
  waterfallSpeed: number;
  waterfallPalette: string;
  spectrumPeakHold: boolean;
  showGrid: boolean;
  showCenterLine: boolean;
  showBandEdges: boolean;
}

export function radioPrefsFromState(s: RadioPrefsSource): RadioPrefs {
  return {
    sampleRate: clampSampleRateHz(s.sampleRate),
    rxAdc: clampRxAdc(s.rxAdc),
    rxAntenna: clampRxAntenna(s.rxAntenna),
    mode: clampDemodMode(s.mode),
    rxVolumeDb: clampRxVolumeDb(s.rxVolumeDb),
    rxNoiseReductionMode: normalizeNrMode(s.rxNoiseReductionMode),
    rxNoiseReductionLevel: clampRxNoiseReductionLevel(s.rxNoiseReductionLevel),
    rxNbMode: normalizeNbMode(s.rxNbMode),
    rxNbThreshold: clampRxNbThreshold(s.rxNbThreshold),
    rxAnrTaps: clampDspTapCount(s.rxAnrTaps),
    rxAnrDelay: clampDspDelay(s.rxAnrDelay),
    rxAnrGain: clampDspGain(s.rxAnrGain, 0.0002),
    rxAnrLeakage: clampDspLeakage(s.rxAnrLeakage, 0.00005),
    anfEnabled: Boolean(s.anfEnabled),
    rxAnfTaps: clampDspTapCount(s.rxAnfTaps),
    rxAnfDelay: clampDspDelay(s.rxAnfDelay),
    rxAnfGain: clampDspGain(s.rxAnfGain, 0.00012),
    rxAnfLeakage: clampDspLeakage(s.rxAnfLeakage, 0.00008),
    agcMode: normalizeAgcMode(s.agcMode),
    agcGain: clampAgcGain(s.agcGain),
    filterLow: clampFilterLowHz(s.filterLow),
    filterHigh: clampFilterHighHz(s.filterHigh),
    txDrive: clampTxDriveWatts(s.txDrive),
    txMicGainDb: clampTxMicGainDb(s.txMicGainDb),
    txFilterLow: clampFilterLowHz(s.txFilterLow),
    txFilterHigh: clampFilterHighHz(s.txFilterHigh),
    rxEqEnabled: Boolean(s.rxEqEnabled),
    rxEqBands: sanitizeEqBands(s.rxEqBands),
    txEqEnabled: Boolean(s.txEqEnabled),
    txEqBands: sanitizeEqBands(s.txEqBands),
    cfcEnabled: Boolean(s.cfcEnabled),
    cfcPrecomp: Number(s.cfcPrecomp) || 0,
    cfcBands: sanitizeCfcBands(s.cfcBands),
    txMeterMode: normalizeTxMeterMode(s.txMeterMode),
    twoToneEnabled: Boolean(s.twoToneEnabled),
    txTwoToneFreq1: clampTwoToneFreqHz(s.txTwoToneFreq1, 700),
    txTwoToneFreq2: clampTwoToneFreqHz(s.txTwoToneFreq2, 1900),
    txTwoToneLevelDb: clampTwoToneLevelDb(s.txTwoToneLevelDb),
    txTwoToneInvertLsb: Boolean(s.txTwoToneInvertLsb),
    txTwoToneDelayMs: clampTwoToneDelayMs(s.txTwoToneDelayMs),
    txNoiseGateEnabled: Boolean(s.txNoiseGateEnabled),
    txNoiseGateThresholdDb: Math.max(-80, Math.min(0, Number(s.txNoiseGateThresholdDb) || -30)),
    txTimeoutEnabled: Boolean(s.txTimeoutEnabled),
    txTimeoutSeconds: Math.max(10, Math.min(600, Math.round(Number(s.txTimeoutSeconds) || 180))),
  };
}

export function displayPrefsFromState(s: DisplayPrefsSource): DisplayPrefs {
  return {
    spectrumAutoRange: Boolean(s.spectrumAutoRange),
    spectrumFloorDb: clampDisplayDb(s.displayFloorDb, -200),
    spectrumCeilingDb: clampDisplayDb(s.displayCeilingDb, -120),
    waterfallAutoRange: Boolean(s.waterfallAutoRange),
    waterfallFloorDb: clampDisplayDb(s.waterfallFloorDb, -200),
    waterfallCeilingDb: clampDisplayDb(s.waterfallCeilingDb, -120),
    spectrumAverage: clampSpectrumAverage(s.spectrumAverage),
    waterfallSpeed: clampWaterfallSpeed(s.waterfallSpeed),
    waterfallPalette: normalizeWaterfallPalette(s.waterfallPalette),
    spectrumPeakHold: Boolean(s.spectrumPeakHold),
    showGrid: Boolean(s.showGrid),
    showCenterLine: Boolean(s.showCenterLine),
    showBandEdges: Boolean(s.showBandEdges),
  };
}
