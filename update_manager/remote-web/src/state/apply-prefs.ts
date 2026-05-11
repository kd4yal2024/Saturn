/**
 * Pure state-mutation helpers for applying and normalizing prefs on AppState.
 *
 * These replace inline normalizeUiState, applyRadioPrefsObject, and
 * applyDisplayPrefsObject — each clamps/sanitizes fields in place.
 */

import {
  clampSampleRateHz,
  clampRxAdc,
  clampRxAntenna,
  clampRxVolumeDb,
  normalizeNrMode,
  clampRxNoiseReductionLevel,
  normalizeNbMode,
  clampRxNbThreshold,
  normalizeAgcMode,
  clampAgcGain,
  clampDspTapCount,
  clampDspDelay,
  clampDspGain,
  clampDspLeakage,
  clampTxDriveWatts,
  clampTxMicGainDb,
  normalizeTxMeterMode,
  clampTwoToneFreqHz,
  clampTwoToneLevelDb,
  clampTwoToneDelayMs,
  sanitizeEqBands,
  sanitizeCfcBands,
  clampDisplayDb,
  clampSpectrumAverage,
  clampWaterfallSpeed,
  normalizeWaterfallPalette,
  safeFiniteNumber,
} from '../settings/normalize';
import { clampDemodMode, clampFilterLowHz, clampFilterHighHz } from '../radio/passband';

/** Minimal state shape for radio prefs application. */
export interface RadioPrefsTarget {
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
  rxFilterShiftHz?: number;
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

/** Minimal state shape for display prefs application. */
export interface DisplayPrefsTarget {
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

/** Full state shape for normalizeAppStateInPlace. */
export interface NormalizableState extends RadioPrefsTarget, DisplayPrefsTarget {
  displayZoom: number;
  frameRate: number;
  fftWidth: number;
  audioBackpressureDrops: number;
}

function stringPref(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback;
}

function scalarPref(value: unknown, fallback: string | number): string | number {
  return typeof value === 'string' || typeof value === 'number' ? value : fallback;
}

/**
 * Merge incoming radio prefs onto state, using existing state values as
 * fallbacks for missing fields. Each field is clamped/normalized.
 */
export function applyRadioPrefsToState(
  prefs: Record<string, unknown> | null | undefined,
  state: RadioPrefsTarget,
): void {
  if (!prefs || typeof prefs !== 'object') return;

  state.sampleRate = clampSampleRateHz(prefs.sampleRate ?? state.sampleRate);
  state.rxAdc = clampRxAdc(prefs.rxAdc ?? state.rxAdc);
  state.rxAntenna = clampRxAntenna(prefs.rxAntenna ?? state.rxAntenna);
  state.mode = clampDemodMode(stringPref(prefs.mode, state.mode));
  state.rxVolumeDb = clampRxVolumeDb(prefs.rxVolumeDb ?? state.rxVolumeDb);
  state.rxNoiseReductionMode = normalizeNrMode(prefs.rxNoiseReductionMode ?? state.rxNoiseReductionMode);
  state.rxNoiseReductionLevel = clampRxNoiseReductionLevel(prefs.rxNoiseReductionLevel ?? state.rxNoiseReductionLevel);
  state.rxNbMode = normalizeNbMode(prefs.rxNbMode ?? state.rxNbMode);
  state.rxNbThreshold = clampRxNbThreshold(prefs.rxNbThreshold ?? state.rxNbThreshold);
  state.rxAnrTaps = clampDspTapCount(prefs.rxAnrTaps ?? state.rxAnrTaps);
  state.rxAnrDelay = clampDspDelay(prefs.rxAnrDelay ?? state.rxAnrDelay);
  state.rxAnrGain = clampDspGain(prefs.rxAnrGain ?? state.rxAnrGain, 0.0002);
  state.rxAnrLeakage = clampDspLeakage(prefs.rxAnrLeakage ?? state.rxAnrLeakage, 0.00005);
  state.anfEnabled = Boolean(prefs.anfEnabled ?? state.anfEnabled);
  state.rxAnfTaps = clampDspTapCount(prefs.rxAnfTaps ?? state.rxAnfTaps);
  state.rxAnfDelay = clampDspDelay(prefs.rxAnfDelay ?? state.rxAnfDelay);
  state.rxAnfGain = clampDspGain(prefs.rxAnfGain ?? state.rxAnfGain, 0.00012);
  state.rxAnfLeakage = clampDspLeakage(prefs.rxAnfLeakage ?? state.rxAnfLeakage, 0.00008);
  state.agcMode = normalizeAgcMode(prefs.agcMode ?? state.agcMode);
  state.agcGain = clampAgcGain(prefs.agcGain ?? state.agcGain);
  state.filterLow = clampFilterLowHz(scalarPref(prefs.filterLow, state.filterLow));
  state.filterHigh = clampFilterHighHz(scalarPref(prefs.filterHigh, state.filterHigh));
  if ('rxFilterShiftHz' in state) state.rxFilterShiftHz = 0;
  state.txDrive = clampTxDriveWatts(prefs.txDrive ?? state.txDrive);
  state.txMicGainDb = clampTxMicGainDb(prefs.txMicGainDb ?? state.txMicGainDb);
  state.txFilterLow = clampFilterLowHz(scalarPref(prefs.txFilterLow, state.txFilterLow));
  state.txFilterHigh = clampFilterHighHz(scalarPref(prefs.txFilterHigh, state.txFilterHigh));
  state.rxEqEnabled = Boolean(prefs.rxEqEnabled ?? state.rxEqEnabled);
  state.rxEqBands = sanitizeEqBands(prefs.rxEqBands as number[] | undefined, state.rxEqBands);
  state.txEqEnabled = Boolean(prefs.txEqEnabled ?? state.txEqEnabled);
  state.txEqBands = sanitizeEqBands(prefs.txEqBands as number[] | undefined, state.txEqBands);
  state.cfcEnabled = Boolean(prefs.cfcEnabled ?? state.cfcEnabled);
  state.cfcPrecomp = Number(prefs.cfcPrecomp ?? state.cfcPrecomp) || 0;
  state.cfcBands = sanitizeCfcBands(prefs.cfcBands as number[] | undefined, state.cfcBands);
  state.txMeterMode = normalizeTxMeterMode(prefs.txMeterMode ?? state.txMeterMode);
  state.twoToneEnabled = Boolean(prefs.twoToneEnabled ?? state.twoToneEnabled);
  state.txTwoToneFreq1 = clampTwoToneFreqHz(prefs.txTwoToneFreq1 ?? state.txTwoToneFreq1, 700);
  state.txTwoToneFreq2 = clampTwoToneFreqHz(prefs.txTwoToneFreq2 ?? state.txTwoToneFreq2, 1900);
  state.txTwoToneLevelDb = clampTwoToneLevelDb(prefs.txTwoToneLevelDb ?? state.txTwoToneLevelDb);
  state.txTwoToneInvertLsb = Boolean(prefs.txTwoToneInvertLsb ?? state.txTwoToneInvertLsb);
  state.txTwoToneDelayMs = clampTwoToneDelayMs(prefs.txTwoToneDelayMs ?? state.txTwoToneDelayMs);
  state.txNoiseGateEnabled = Boolean(prefs.txNoiseGateEnabled ?? state.txNoiseGateEnabled);
  state.txNoiseGateThresholdDb = Math.max(-80, Math.min(0,
    Number(prefs.txNoiseGateThresholdDb ?? state.txNoiseGateThresholdDb) || -30));
  state.txTimeoutEnabled = Boolean(prefs.txTimeoutEnabled ?? state.txTimeoutEnabled);
  state.txTimeoutSeconds = Math.max(10, Math.min(600, Math.round(Number(prefs.txTimeoutSeconds ?? state.txTimeoutSeconds) || 180)));
}

/**
 * Merge incoming display prefs onto state, using existing state values as
 * fallbacks for missing fields. Each field is clamped/normalized.
 *
 * Returns whether spectrumPeakHold was turned off (caller may need to reset peak bins).
 */
export function applyDisplayPrefsToState(
  prefs: Record<string, unknown> | null | undefined,
  state: DisplayPrefsTarget,
): { peakHoldCleared: boolean } {
  if (!prefs || typeof prefs !== 'object') return { peakHoldCleared: false };

  const wasPeakHold = state.spectrumPeakHold;

  state.spectrumAutoRange = Boolean(prefs.spectrumAutoRange ?? state.spectrumAutoRange);
  state.displayFloorDb = clampDisplayDb(prefs.spectrumFloorDb ?? state.displayFloorDb, -200);
  state.displayCeilingDb = clampDisplayDb(prefs.spectrumCeilingDb ?? state.displayCeilingDb, -120);
  state.waterfallAutoRange = Boolean(prefs.waterfallAutoRange ?? state.waterfallAutoRange);
  state.waterfallFloorDb = clampDisplayDb(prefs.waterfallFloorDb ?? state.waterfallFloorDb, -200);
  state.waterfallCeilingDb = clampDisplayDb(prefs.waterfallCeilingDb ?? state.waterfallCeilingDb, -120);
  state.spectrumAverage = clampSpectrumAverage(prefs.spectrumAverage ?? state.spectrumAverage);
  state.waterfallSpeed = clampWaterfallSpeed(prefs.waterfallSpeed ?? state.waterfallSpeed);
  state.waterfallPalette = normalizeWaterfallPalette(prefs.waterfallPalette ?? state.waterfallPalette);
  state.spectrumPeakHold = Boolean(prefs.spectrumPeakHold ?? state.spectrumPeakHold);
  state.showGrid = Boolean(prefs.showGrid ?? state.showGrid);
  state.showCenterLine = Boolean(prefs.showCenterLine ?? state.showCenterLine);
  state.showBandEdges = Boolean(prefs.showBandEdges ?? state.showBandEdges);

  return { peakHoldCleared: wasPeakHold && !state.spectrumPeakHold };
}

/**
 * Clamp/normalize all mutable state fields in place.
 * Called before every UI render cycle.
 */
export function normalizeAppStateInPlace(state: NormalizableState): void {
  // Radio prefs subset
  state.sampleRate = clampSampleRateHz(state.sampleRate);
  state.rxAdc = clampRxAdc(state.rxAdc);
  state.rxAntenna = clampRxAntenna(state.rxAntenna);
  state.rxVolumeDb = clampRxVolumeDb(state.rxVolumeDb);
  state.rxNoiseReductionMode = normalizeNrMode(state.rxNoiseReductionMode);
  state.rxNoiseReductionLevel = clampRxNoiseReductionLevel(state.rxNoiseReductionLevel);
  state.rxNbMode = normalizeNbMode(state.rxNbMode);
  state.rxNbThreshold = clampRxNbThreshold(state.rxNbThreshold);
  state.rxAnrTaps = clampDspTapCount(state.rxAnrTaps);
  state.rxAnrDelay = clampDspDelay(state.rxAnrDelay);
  state.rxAnrGain = clampDspGain(state.rxAnrGain, 0.0002);
  state.rxAnrLeakage = clampDspLeakage(state.rxAnrLeakage, 0.00005);
  state.anfEnabled = Boolean(state.anfEnabled);
  state.rxAnfTaps = clampDspTapCount(state.rxAnfTaps);
  state.rxAnfDelay = clampDspDelay(state.rxAnfDelay);
  state.rxAnfGain = clampDspGain(state.rxAnfGain, 0.00012);
  state.rxAnfLeakage = clampDspLeakage(state.rxAnfLeakage, 0.00008);
  state.agcMode = normalizeAgcMode(state.agcMode);
  state.agcGain = clampAgcGain(state.agcGain);
  state.displayZoom = Math.max(1, Math.min(32, safeFiniteNumber(state.displayZoom, 1)));
  state.filterLow = clampFilterLowHz(state.filterLow);
  state.filterHigh = clampFilterHighHz(state.filterHigh);
  if ('rxFilterShiftHz' in state) {
    state.rxFilterShiftHz = Math.round(safeFiniteNumber(state.rxFilterShiftHz, 0));
  }
  state.txDrive = clampTxDriveWatts(state.txDrive);
  state.txMicGainDb = clampTxMicGainDb(state.txMicGainDb);
  state.txFilterLow = clampFilterLowHz(state.txFilterLow);
  state.txFilterHigh = clampFilterHighHz(state.txFilterHigh);
  state.rxEqBands = sanitizeEqBands(state.rxEqBands);
  state.txEqBands = sanitizeEqBands(state.txEqBands);
  state.cfcEnabled = Boolean(state.cfcEnabled);
  state.cfcPrecomp = safeFiniteNumber(state.cfcPrecomp, 0);
  state.cfcBands = sanitizeCfcBands(state.cfcBands);
  state.txMeterMode = normalizeTxMeterMode(state.txMeterMode);
  state.twoToneEnabled = Boolean(state.twoToneEnabled);
  state.txTwoToneFreq1 = clampTwoToneFreqHz(state.txTwoToneFreq1, 700);
  state.txTwoToneFreq2 = clampTwoToneFreqHz(state.txTwoToneFreq2, 1900);
  state.txTwoToneLevelDb = clampTwoToneLevelDb(state.txTwoToneLevelDb);
  state.txTwoToneInvertLsb = Boolean(state.txTwoToneInvertLsb);
  state.txTwoToneDelayMs = clampTwoToneDelayMs(state.txTwoToneDelayMs);
  state.txNoiseGateEnabled = Boolean(state.txNoiseGateEnabled);
  state.txNoiseGateThresholdDb = Math.max(-80, Math.min(0,
    Number(state.txNoiseGateThresholdDb) || -30));
  state.txTimeoutEnabled = Boolean(state.txTimeoutEnabled);
  state.txTimeoutSeconds = Math.max(10, Math.min(600, Math.round(safeFiniteNumber(state.txTimeoutSeconds, 180))));

  // Telemetry / display
  state.frameRate = Math.max(0, safeFiniteNumber(state.frameRate, 0));
  state.fftWidth = Math.max(0, Math.round(safeFiniteNumber(state.fftWidth, 0)));
  state.audioBackpressureDrops = Math.max(0, Math.round(safeFiniteNumber(state.audioBackpressureDrops, 0)));
  state.spectrumAutoRange = Boolean(state.spectrumAutoRange);
  state.displayFloorDb = clampDisplayDb(state.displayFloorDb, -200);
  state.displayCeilingDb = clampDisplayDb(state.displayCeilingDb, -120);
  state.waterfallAutoRange = Boolean(state.waterfallAutoRange);
  state.waterfallFloorDb = clampDisplayDb(state.waterfallFloorDb, -200);
  state.waterfallCeilingDb = clampDisplayDb(state.waterfallCeilingDb, -120);
  state.spectrumAverage = clampSpectrumAverage(state.spectrumAverage);
  state.waterfallSpeed = clampWaterfallSpeed(state.waterfallSpeed);
  state.waterfallPalette = normalizeWaterfallPalette(state.waterfallPalette);
  state.spectrumPeakHold = Boolean(state.spectrumPeakHold);
  state.showGrid = Boolean(state.showGrid);
  state.showCenterLine = Boolean(state.showCenterLine);
  state.showBandEdges = Boolean(state.showBandEdges);
}
