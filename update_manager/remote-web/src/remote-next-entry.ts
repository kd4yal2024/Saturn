


import {
  normalizeBandMemory,
  clampSampleRateHz,
  clampRxAdc,
  clampRxAntenna,
  clampRxVolumeDb,
  normalizeNrMode,
  clampRxNoiseReductionLevel,
  normalizeNr2GainMethod,
  normalizeNr2NpeMethod,
  normalizeWbfmDeemphasis,
  clampTxPhaseRotatorCornerHz,
  clampPureSignalAttenuationDb,
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
  clampEqGainDb,
  sanitizeEqBands,
  sanitizeCfcBands,
  clampDisplayDb,
  clampSpectrumAverage,
  clampWaterfallSpeed,
  clampWaterfallContrast,
  normalizeWaterfallPalette,
  normalizeSpectrumTraceColor,
  clampSpectrumVisualEffect,
  sanitizePhonePanels,
} from './settings/normalize';
import { parseTciText } from './tci/parser';
import { applyTciText } from './tci/apply';
import { decodeAudioFrame } from './transport/rx-frame';


import {
  buildTxMicPcmS16Frame,
  decideTxMicSend,
  detectTxCodecCapabilities,
  txMicByteRateBytesPerSecond,
} from './transport/tx-uplink';
import {
  TX_OPUS_OVERRIDE_LEGACY_STORAGE_KEY,
  TX_OPUS_OVERRIDE_STORAGE_KEY,
  createTxOpusAudioDataFromFloat32,
  createTxOpusFrameProducer,
  txOpusCodecForAccepted,
  txOpusProfileForCodec,
  txOpusRuntimeOverrideEnabled,
} from './audio/tx-opus-encoder';

import { createLegacySocketAdapter } from './transport/legacy-socket-adapter';
import { createReconnectSupervisor } from './transport/reconnect-supervisor';
import {
  SPLIT_TRANSPORT_LEGACY_STORAGE_KEY,
  SPLIT_TRANSPORT_STORAGE_KEY,
  createSplitSessionId,
  splitTransportEnabled,
} from './transport/transport-mode';
import {
  bandKeyForFrequency,
  effectiveIqSampleRate,
  adcLabel,
  antennaLabel,
  normalizeStreamMode,
  noiseReductionLabel,
  describeMicCaptureError,
} from './radio/band';
import {
  normalizeDemodMode,
  clampDemodMode,
  isNegativePassbandMode,
  isSymmetricPassbandMode,
  clampFilterLowHz,
  clampFilterHighHz,
  signedPassbandFromUiCuts,
  shiftedSignedPassbandFromUiCuts,
  decomposeSignedPassbandWithShift,
  defaultFilterCutsForMode,
  uiCutsFromSignedPassband,
  clickTuneCarrierOffsetHz,
} from './radio/passband';
import {
  formatFrequencyMarkup,
  snapTuneFrequencyHz,
  formatFrequencyPlain,
  safeFiniteNumber,
  safeFixed,
} from './radio/frequency';
import { anrPreset, anfPreset } from './radio/dsp-presets';
import {
  buildRxFilterBandCommand,
  buildTxFilterBandCommand,
  buildAnrCommands,
  buildAnfCommands,
  buildTwoToneCommands,
  buildAllRadioPrefsCommands,
  buildTxCodecCapsCommand,
} from './tci/commands';
import { prepareAudioForPlayback } from './audio/resample';
import {
  audioFramesToMilliseconds,
  formatRxLatencyDiagnostic,
  rxAudioArrivalJitterMs,
  summarizeRxAudioJitter,
} from './audio/rx-telemetry';
import {
  buildRxAudioStartCommand,
  rxAudioTransportProfile,
} from './audio/rx-profile';
import {
  buildTxAudioProfileCommands,
  voodoo38kTxAudioProfile,
} from './audio/tx-audio-profile';
import { volumeAmplitudeFromDb } from './audio/constants';
import { FftProcessor } from './dsp/fft';
import {
  lerp,
  moveToward,
  smoothToward,
  svgArcPath,
  dbmToSmDeg,
  formatSignalStrength,
  linearLevelToDbfs,
  instrumentOperatingState,
  auxDeg,
  txPowerToAuxDeg,
  swrToAuxDeg,
  SM_CX,
  SM_CY,
  SM_R,
  SM_START,
  SM_END,
  SM_S9_DEG,
  AX_CX,
  AX_CY,
  AX_R,
  AX_START,
  AX_END,
  AX_SPAN,
  SMETER_ATTACK_MS,
  SMETER_RELEASE_MS,
  SMETER_PEAK_HOLD_MS,
  SMETER_PEAK_DROP_PER_SEC,
  TX_POWER_ATTACK_PER_SEC,
  TX_POWER_RELEASE_PER_SEC,
  TX_POWER_PEAK_HOLD_MS,
  TX_POWER_PEAK_DROP_PER_SEC,
  SWR_ATTACK_PER_SEC,
  SWR_RELEASE_PER_SEC,
} from './ui/meter-math';
import {
  displaySpanHz,
  displayPercentForOffsetHz,
  frequencyScaleTicks,
  visibleBinsForDisplay,
  shiftBinsHorizontally,
  smoothSpectrumTrace,
  smoothWaterfallBins,
  bandEdgesInView,
  autoRangeFromBins,
} from './dsp/display';
import { createAppState } from './state/app-state';
import {
  normalizeRadioControlContext,
  radioControlContextForTarget,
} from './ui/control-context';
import {
  txActionAvailability,
  txControlPresentationState,
} from './ui/tx-presentation';
import {
  normalizeOperationsDrawerTarget,
  restoreOperationsDrawerSelection,
  selectOperationsDrawerTarget,
} from './ui/operations-drawer';
import {
  adjacentSetupPanelId,
  normalizeSetupPanelId,
} from './ui/setup-navigation';
import {
  clampSpectrumWaterfallRatio,
  nextPhoneSpectrumMode,
  normalizePhoneSpectrumMode,
  phoneSpectrumRatio,
  spectrumWaterfallRatioFromPointer,
} from './ui/display-layout';
import { radioPrefsFromState, displayPrefsFromState } from './state/prefs-from-state';
import { buildPerfSnapshot, buildPerfSummary } from './state/perf-snapshot';
import {
  applyRadioPrefsToState,
  applyDisplayPrefsToState,
  normalizeAppStateInPlace,
} from './state/apply-prefs';
import { preferredResponsiveLayout } from './ui/responsive-layout';

const api = {
  // Controller / runtime

  // TCI parsing
  parseTciText,
  applyTciText,

  // TCI command builders
  buildRxFilterBandCommand,
  buildTxFilterBandCommand,
  buildAnrCommands,
  buildAnfCommands,
  buildTwoToneCommands,
  buildAllRadioPrefsCommands,
  buildTxCodecCapsCommand,

  // Transport
  decodeAudioFrame,
  buildTxMicPcmS16Frame,
  decideTxMicSend,
  detectTxCodecCapabilities,
  createTxOpusAudioDataFromFloat32,
  createTxOpusFrameProducer,
  txOpusCodecForAccepted,
  txOpusProfileForCodec,
  txOpusRuntimeOverrideEnabled,
  txMicByteRateBytesPerSecond,
  TX_OPUS_OVERRIDE_LEGACY_STORAGE_KEY,
  TX_OPUS_OVERRIDE_STORAGE_KEY,
  createReconnectSupervisor,
  createLegacySocketAdapter,
  SPLIT_TRANSPORT_LEGACY_STORAGE_KEY,
  SPLIT_TRANSPORT_STORAGE_KEY,
  createSplitSessionId,
  splitTransportEnabled,

  // Radio
  bandKeyForFrequency,
  effectiveIqSampleRate,
  adcLabel,
  antennaLabel,
  normalizeStreamMode,
  noiseReductionLabel,
  describeMicCaptureError,
  normalizeDemodMode,
  clampDemodMode,
  isNegativePassbandMode,
  isSymmetricPassbandMode,
  clampFilterLowHz,
  clampFilterHighHz,
  signedPassbandFromUiCuts,
  shiftedSignedPassbandFromUiCuts,
  decomposeSignedPassbandWithShift,
  uiCutsFromSignedPassband,
  clickTuneCarrierOffsetHz,
  defaultFilterCutsForMode,
  formatFrequencyMarkup,
  formatFrequencyPlain,
  snapTuneFrequencyHz,
  safeFiniteNumber,
  safeFixed,
  anrPreset,
  anfPreset,

  // Settings clamp/normalize
  clampSampleRateHz,
  clampRxAdc,
  clampRxAntenna,
  clampRxVolumeDb,
  normalizeNrMode,
  clampRxNoiseReductionLevel,
  normalizeNr2GainMethod,
  normalizeNr2NpeMethod,
  normalizeWbfmDeemphasis,
  clampTxPhaseRotatorCornerHz,
  clampPureSignalAttenuationDb,
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
  clampEqGainDb,
  sanitizeEqBands,
  sanitizeCfcBands,
  clampDisplayDb,
  clampSpectrumAverage,
  clampWaterfallSpeed,
  clampWaterfallContrast,
  normalizeWaterfallPalette,
  normalizeSpectrumTraceColor,
  clampSpectrumVisualEffect,
  sanitizePhonePanels,
  normalizeBandMemory,

  // Audio
  volumeAmplitudeFromDb,
  prepareAudioForPlayback,
  audioFramesToMilliseconds,
  formatRxLatencyDiagnostic,
  rxAudioArrivalJitterMs,
  summarizeRxAudioJitter,
  buildRxAudioStartCommand,
  rxAudioTransportProfile,
  buildTxAudioProfileCommands,
  voodoo38kTxAudioProfile,

  // DSP
  FftProcessor,
  displaySpanHz,
  displayPercentForOffsetHz,
  frequencyScaleTicks,
  visibleBinsForDisplay,
  shiftBinsHorizontally,
  smoothSpectrumTrace,
  smoothWaterfallBins,
  bandEdgesInView,
  autoRangeFromBins,

  // State
  createAppState,
  radioPrefsFromState,
  displayPrefsFromState,
  buildPerfSnapshot,
  buildPerfSummary,
  applyRadioPrefsToState,
  applyDisplayPrefsToState,
  normalizeAppStateInPlace,
  preferredResponsiveLayout,
  normalizeRadioControlContext,
  radioControlContextForTarget,
  txActionAvailability,
  txControlPresentationState,
  normalizeOperationsDrawerTarget,
  restoreOperationsDrawerSelection,
  selectOperationsDrawerTarget,
  adjacentSetupPanelId,
  normalizeSetupPanelId,
  clampSpectrumWaterfallRatio,
  nextPhoneSpectrumMode,
  normalizePhoneSpectrumMode,
  phoneSpectrumRatio,
  spectrumWaterfallRatioFromPointer,

  // Meter math
  lerp, moveToward, smoothToward, svgArcPath, dbmToSmDeg, formatSignalStrength, linearLevelToDbfs, instrumentOperatingState,
  auxDeg, txPowerToAuxDeg, swrToAuxDeg,
  SM_CX, SM_CY, SM_R, SM_START, SM_END, SM_S9_DEG,
  AX_CX, AX_CY, AX_R, AX_START, AX_END, AX_SPAN,
  SMETER_ATTACK_MS, SMETER_RELEASE_MS, SMETER_PEAK_HOLD_MS, SMETER_PEAK_DROP_PER_SEC,
  TX_POWER_ATTACK_PER_SEC, TX_POWER_RELEASE_PER_SEC, TX_POWER_PEAK_HOLD_MS, TX_POWER_PEAK_DROP_PER_SEC,
  SWR_ATTACK_PER_SEC, SWR_RELEASE_PER_SEC,
};

export const SaturnRemoteNext = api;

(globalThis as typeof globalThis & { SaturnRemoteNext?: typeof api }).SaturnRemoteNext = api;
