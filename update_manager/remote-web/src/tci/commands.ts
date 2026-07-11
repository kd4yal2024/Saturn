import type { RadioPrefs } from '../settings/types';
import {
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
  clampDspTapCount,
  clampDspDelay,
  clampDspGain,
  clampDspLeakage,
  normalizeAgcMode,
  clampAgcGain,
  clampTxDriveWatts,
  clampTxMicGainDb,
  clampTwoToneFreqHz,
  clampTwoToneLevelDb,
  clampTwoToneDelayMs,
} from '../settings/normalize';
import { clampDemodMode, shiftedSignedPassbandFromUiCuts, signedPassbandFromUiCuts } from '../radio/passband';
import type { TxCodecCapability } from '../transport/tx-uplink';

export function buildRxFilterBandCommand(
  filterLow: number,
  filterHigh: number,
  mode: string,
  filterShiftHz = 0,
): string {
  const pb = shiftedSignedPassbandFromUiCuts(filterLow, filterHigh, mode, filterShiftHz);
  return `rx_filter_band:0,${pb.lowHz},${pb.highHz};`;
}

export function buildTxFilterBandCommand(
  txFilterLow: number,
  txFilterHigh: number,
  mode: string,
): string {
  const pb = signedPassbandFromUiCuts(txFilterLow, txFilterHigh, mode);
  return `tx_filter_band:0,${pb.lowHz},${pb.highHz};`;
}

export function buildTxCodecCapsCommand(codecs: readonly TxCodecCapability[] = ['pcm']): string {
  const allowed = new Set<TxCodecCapability>(['pcm', 'opus_nb', 'opus_wb']);
  const unique = codecs.filter((codec, index) => allowed.has(codec) && codecs.indexOf(codec) === index);
  const advertised = unique.length > 0 ? unique : ['pcm'];
  return `tx_codec_caps:0,${advertised.join(',')};`;
}

export function buildAnrCommands(prefs: Pick<RadioPrefs, 'rxAnrTaps' | 'rxAnrDelay' | 'rxAnrGain' | 'rxAnrLeakage'>): string[] {
  return [
    `rx_anr_taps:0,${clampDspTapCount(prefs.rxAnrTaps)};`,
    `rx_anr_delay:0,${clampDspDelay(prefs.rxAnrDelay)};`,
    `rx_anr_gain:0,${clampDspGain(prefs.rxAnrGain, 0.0002).toFixed(6)};`,
    `rx_anr_leakage:0,${clampDspLeakage(prefs.rxAnrLeakage, 0.00005).toFixed(6)};`,
  ];
}

export function buildAnfCommands(
  prefs: Pick<RadioPrefs, 'anfEnabled' | 'rxAnfTaps' | 'rxAnfDelay' | 'rxAnfGain' | 'rxAnfLeakage'>,
): string[] {
  return [
    `rx_anf:0,${Boolean(prefs.anfEnabled)};`,
    `rx_anf_taps:0,${clampDspTapCount(prefs.rxAnfTaps)};`,
    `rx_anf_delay:0,${clampDspDelay(prefs.rxAnfDelay)};`,
    `rx_anf_gain:0,${clampDspGain(prefs.rxAnfGain, 0.00012).toFixed(6)};`,
    `rx_anf_leakage:0,${clampDspLeakage(prefs.rxAnfLeakage, 0.00008).toFixed(6)};`,
  ];
}

export function buildTwoToneCommands(
  prefs: Pick<RadioPrefs, 'txTwoToneFreq1' | 'txTwoToneFreq2' | 'txTwoToneLevelDb' | 'txTwoToneInvertLsb' | 'txTwoToneDelayMs'>,
): string[] {
  return [
    `tx_two_tone_freq1:0,${clampTwoToneFreqHz(prefs.txTwoToneFreq1, 700)};`,
    `tx_two_tone_freq2:0,${clampTwoToneFreqHz(prefs.txTwoToneFreq2, 1900)};`,
    `tx_two_tone_level_db:0,${clampTwoToneLevelDb(prefs.txTwoToneLevelDb).toFixed(1)};`,
    `tx_two_tone_invert_lsb:0,${Boolean(prefs.txTwoToneInvertLsb)};`,
    `tx_two_tone_delay_ms:0,${clampTwoToneDelayMs(prefs.txTwoToneDelayMs)};`,
  ];
}

function sanitizeEqBand(bands: number[] | undefined, index: number): number {
  if (!bands || !Array.isArray(bands)) return 0;
  const v = Number(bands[index]);
  return Number.isFinite(v) ? Math.max(-12, Math.min(12, Math.round(v))) : 0;
}

function sanitizeCfcBand(bands: number[] | undefined, index: number): string {
  if (!bands || !Array.isArray(bands)) return '0.0';
  return Math.max(0, Math.min(20, Number(bands[index]) || 0)).toFixed(1);
}

export function buildAllRadioPrefsCommands(prefs: RadioPrefs): string[] {
  const cmds: string[] = [
    `iq_samplerate:${clampSampleRateHz(prefs.sampleRate)};`,
    `rx_adc:0,${clampRxAdc(prefs.rxAdc)};`,
    `rx_antenna:0,${clampRxAntenna(prefs.rxAntenna)};`,
    `modulation:0,${clampDemodMode(prefs.mode)};`,
    `rx_volume:0,0,${clampRxVolumeDb(prefs.rxVolumeDb).toFixed(1)};`,
    `rx_nr_mode:0,${normalizeNrMode(prefs.rxNoiseReductionMode)};`,
    `rx_nr_level:0,${clampRxNoiseReductionLevel(prefs.rxNoiseReductionLevel)};`,
    `rx_nr2_gain_method:0,${normalizeNr2GainMethod(prefs.rxNr2GainMethod)};`,
    `rx_nr2_npe_method:0,${normalizeNr2NpeMethod(prefs.rxNr2NpeMethod)};`,
    `rx_nr2_post_filter:0,${Boolean(prefs.rxNr2PostFilterEnabled)};`,
    `rx_wbfm_deemphasis:0,${normalizeWbfmDeemphasis(prefs.rxWbfmDeemphasis)};`,
    `rx_nb:0,${normalizeNbMode(prefs.rxNbMode)};`,
    `rx_nb_threshold:0,${clampRxNbThreshold(prefs.rxNbThreshold).toFixed(2)};`,
    ...buildAnrCommands(prefs),
    ...buildAnfCommands(prefs),
    `rx_agc:0,${normalizeAgcMode(prefs.agcMode)};`,
    `rx_agc_gain:0,${clampAgcGain(prefs.agcGain)};`,
    buildRxFilterBandCommand(prefs.filterLow, prefs.filterHigh, prefs.mode),
    `tx_drive:0,${clampTxDriveWatts(prefs.txDrive)};`,
    `tx_mic_gain:0,${clampTxMicGainDb(prefs.txMicGainDb).toFixed(1)};`,
    buildTxFilterBandCommand(prefs.txFilterLow, prefs.txFilterHigh, prefs.mode),
    `rx_eq_enable:0,${Boolean(prefs.rxEqEnabled)};`,
    `tx_eq_enable:0,${Boolean(prefs.txEqEnabled)};`,
  ];

  for (let i = 1; i <= 10; i += 1) {
    cmds.push(`rx_eq_band:0,${i},${sanitizeEqBand(prefs.rxEqBands, i)};`);
    cmds.push(`tx_eq_band:0,${i},${sanitizeEqBand(prefs.txEqBands, i)};`);
    cmds.push(`tx_cfc_band:0,${i},${sanitizeCfcBand(prefs.cfcBands, i)};`);
  }

  cmds.push(`tx_cfc_enable:0,${Boolean(prefs.cfcEnabled)};`);
  cmds.push(`tx_cfc_precomp:0,${(Number(prefs.cfcPrecomp) || 0).toFixed(1)};`);
  cmds.push(`tx_phase_rotator:0,${Boolean(prefs.txPhaseRotatorEnabled)};`);
  cmds.push(`tx_phase_rotator_auto:0,${Boolean(prefs.txPhaseRotatorAuto)};`);
  cmds.push(`tx_phase_rotator_corner:0,${clampTxPhaseRotatorCornerHz(prefs.txPhaseRotatorCornerHz)};`);
  cmds.push(`tx_puresignal:0,${Boolean(prefs.pureSignalEnabled)};`);
  cmds.push(`tx_puresignal_auto_attenuate:0,${Boolean(prefs.pureSignalAutoAttenuate)};`);
  cmds.push(`tx_puresignal_attenuation:0,${clampPureSignalAttenuationDb(prefs.pureSignalAttenuationDb)};`);
  cmds.push(...buildTwoToneCommands(prefs));
  cmds.push(`tx_two_tone:0,${Boolean(prefs.twoToneEnabled)};`);

  return cmds;
}
