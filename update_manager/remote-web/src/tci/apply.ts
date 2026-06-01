import { parseTciText, type TciCommand, booleanArg, numericArg, trailingArg } from './parser';
import { clampDemodMode, decomposeSignedPassbandWithShift, uiCutsFromSignedPassband } from '../radio/passband';
import {
  clampAgcGain,
  clampDspDelay,
  clampDspGain,
  clampDspLeakage,
  clampDspTapCount,
  clampFilterHighHz,
  clampFilterLowHz,
  clampRxAdc,
  clampRxAntenna,
  clampRxNoiseReductionLevel,
  clampRxNbThreshold,
  clampRxVolumeDb,
  clampSampleRateHz,
  clampTxDriveWatts,
  clampTwoToneDelayMs,
  clampTwoToneFreqHz,
  clampTwoToneLevelDb,
  normalizeAgcMode,
  normalizeNbMode,
  normalizeNrMode,
} from '../settings/normalize';
import type { TciApplyResult, TciClientRole, TciRadioState, TxCodecName } from './state';

function argAt(args: readonly string[], index: number): string | undefined {
  return index >= 0 && index < args.length ? args[index] : undefined;
}

function parseBandGain(args: readonly string[]): { band: number; gain: number } | null {
  const band = numericArg(argAt(args, 1));
  const gain = numericArg(argAt(args, 2));
  if (band == null || gain == null) return null;
  if (band < 1 || band > 10) return null;
  return { band, gain };
}

function nowMs(): number {
  const perf = globalThis.performance;
  return perf && typeof perf.now === 'function' ? perf.now() : Date.now();
}

function nonNegativeIntArg(args: readonly string[], index: number): number | null {
  const value = numericArg(argAt(args, index));
  if (value == null) return null;
  return Math.max(0, Math.round(value));
}

function describeTxFault(args: readonly string[]): string {
  const reason = String(argAt(args, 1) ?? argAt(args, 0) ?? 'fault')
    .trim()
    .toLowerCase();
  const actual = numericArg(argAt(args, 2));
  const limit = numericArg(argAt(args, 3));

  if (reason === 'power_trip') {
    if (actual != null && limit != null) {
      return `Power trip ${actual.toFixed(1)} W > ${limit.toFixed(1)} W`;
    }
    return 'Power trip';
  }

  if (reason === 'uplink_late') {
    if (actual != null && limit != null) {
      return `Uplink late ${Math.round(actual)} ms > ${Math.round(limit)} ms`;
    }
    return 'Uplink late';
  }

  return `Bridge fault: ${reason.replace(/[_-]+/g, ' ') || 'TX fault'}`;
}

function normalizeClientRole(value: string | undefined): TciClientRole | null {
  const normalized = String(value ?? '').trim().toLowerCase();
  if (normalized === 'operator' || normalized === 'owner') return 'operator';
  if (normalized === 'viewer' || normalized === 'view') return 'viewer';
  return null;
}

function normalizeTxCodecName(value: string | undefined): TxCodecName | null {
  const normalized = String(value ?? '').trim().toLowerCase().replace(/-/g, '_');
  if (normalized === '0' || normalized === 'pcm' || normalized === 's16') return 'pcm';
  if (normalized === '1' || normalized === 'opus_nb' || normalized === 'opus_narrowband') return 'opus_nb';
  if (normalized === '2' || normalized === 'opus_wb' || normalized === 'opus_wideband') return 'opus_wb';
  return null;
}

function txCodecArgOffset(args: readonly string[]): number {
  return String(argAt(args, 0) ?? '').trim() === '0' ? 1 : 0;
}

function parseClientRole(args: readonly string[]): { role: TciClientRole; id: string | null } | null {
  const firstRole = normalizeClientRole(argAt(args, 0));
  if (firstRole) {
    const id = String(argAt(args, 1) ?? '').trim();
    return { role: firstRole, id: id.length > 0 ? id.slice(0, 32) : null };
  }

  const secondRole = normalizeClientRole(argAt(args, 1));
  if (secondRole) {
    const id = String(argAt(args, 2) ?? '').trim();
    return { role: secondRole, id: id.length > 0 ? id.slice(0, 32) : null };
  }

  return null;
}

export function applyTciCommand(command: TciCommand, current: TciRadioState): TciApplyResult {
  const next: TciRadioState = {
    ...current,
    rxEqBands: current.rxEqBands.slice(),
    txEqBands: current.txEqBands.slice(),
    cfcBands: current.cfcBands.slice(),
  };

  let ready = false;
  let displayCenterHzChanged = false;
  let sampleRateChanged = false;
  let rxVolumeChanged = false;
  let txReleased = false;
  let txFault: string | null = null;

  if (command.name === 'ready') {
    ready = true;
    return { state: next, ready, displayCenterHzChanged, sampleRateChanged, rxVolumeChanged, txReleased, txFault };
  }

  const args = command.args;

  if (command.name === 'vfo' && args.length >= 3) {
    const which = argAt(args, 1);
    const hz = numericArg(argAt(args, 2));
    if (hz != null) {
      if (which === '0') next.vfoA = hz;
      if (which === '1') next.vfoB = hz;
    }
  } else if (command.name === 'dds' && args.length >= 2) {
    const dds = numericArg(argAt(args, 1));
    if (dds != null && dds !== next.dds) {
      next.dds = dds;
      displayCenterHzChanged = true;
    }
  } else if (command.name === 'rx_adc') {
    const adc = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (adc != null) next.rxAdc = clampRxAdc(adc);
  } else if (command.name === 'rx_antenna') {
    const antenna = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (antenna != null) next.rxAntenna = clampRxAntenna(antenna);
  } else if (command.name === 'iq_samplerate' && args.length >= 1) {
    const rate = numericArg(argAt(args, 0));
    if (rate != null && rate > 0 && rate !== next.sampleRate) {
      next.sampleRate = rate;
      sampleRateChanged = true;
    }
  } else if (command.name === 'modulation' && args.length >= 2) {
    next.mode = clampDemodMode(argAt(args, 1));
  } else if (command.name === 'rx_volume') {
    const volume = numericArg(argAt(args, 2) ?? trailingArg(args));
    if (volume != null) {
      next.rxVolumeDb = clampRxVolumeDb(volume);
      rxVolumeChanged = true;
    }
  } else if (command.name === 'rx_nr_mode' || command.name === 'nr_mode') {
    next.rxNoiseReductionMode = normalizeNrMode(argAt(args, 1) ?? trailingArg(args));
  } else if (command.name === 'rx_nr' || command.name === 'nr') {
    const enabled = booleanArg(argAt(args, 1) ?? trailingArg(args));
    if (enabled === false) next.rxNoiseReductionMode = 'OFF';
    if (enabled === true && next.rxNoiseReductionMode === 'OFF') next.rxNoiseReductionMode = 'NR1';
  } else if (command.name === 'rx_nr_level' || command.name === 'nr_level') {
    const level = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (level != null) next.rxNoiseReductionLevel = clampRxNoiseReductionLevel(level);
  } else if (command.name === 'rx_nb') {
    next.rxNbMode = normalizeNbMode(argAt(args, 1) ?? trailingArg(args));
  } else if (command.name === 'rx_nb_threshold') {
    const threshold = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (threshold != null) next.rxNbThreshold = clampRxNbThreshold(threshold);
  } else if (command.name === 'rx_anr_taps') {
    const taps = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (taps != null) next.rxAnrTaps = clampDspTapCount(taps);
  } else if (command.name === 'rx_anr_delay') {
    const delay = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (delay != null) next.rxAnrDelay = clampDspDelay(delay);
  } else if (command.name === 'rx_anr_gain') {
    const gain = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (gain != null) next.rxAnrGain = clampDspGain(gain, 0.0002);
  } else if (command.name === 'rx_anr_leakage') {
    const leakage = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (leakage != null) next.rxAnrLeakage = clampDspLeakage(leakage, 0.00005);
  } else if (command.name === 'rx_anf') {
    next.anfEnabled = booleanArg(argAt(args, 1) ?? trailingArg(args)) === true;
  } else if (command.name === 'rx_anf_taps') {
    const taps = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (taps != null) next.rxAnfTaps = clampDspTapCount(taps);
  } else if (command.name === 'rx_anf_delay') {
    const delay = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (delay != null) next.rxAnfDelay = clampDspDelay(delay);
  } else if (command.name === 'rx_anf_gain') {
    const gain = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (gain != null) next.rxAnfGain = clampDspGain(gain, 0.00012);
  } else if (command.name === 'rx_anf_leakage') {
    const leakage = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (leakage != null) next.rxAnfLeakage = clampDspLeakage(leakage, 0.00008);
  } else if (command.name === 'rx_agc') {
    next.agcMode = normalizeAgcMode(argAt(args, 1) ?? trailingArg(args));
  } else if (command.name === 'rx_agc_gain') {
    const gain = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (gain != null) next.agcGain = clampAgcGain(gain);
  } else if (command.name === 'rx_filter_band' && args.length >= 3) {
    const fl = numericArg(argAt(args, 1));
    const fh = numericArg(argAt(args, 2));
    if (fl != null && fh != null) {
      const filter = decomposeSignedPassbandWithShift(fl, fh, next.mode, next.filterLow);
      next.filterLow = filter.lowCutHz;
      next.filterHigh = filter.highCutHz;
      next.rxFilterShiftHz = filter.shiftHz;
    }
  } else if (command.name === 'rx_smeter') {
    const meter = numericArg(argAt(args, 2) ?? argAt(args, 0));
    if (meter != null) next.meterDbm = meter;
  } else if (command.name === 'tx_power') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.txPower = value;
  } else if (command.name === 'tx_drive') {
    const value = numericArg(argAt(args, 1) ?? trailingArg(args));
    if (value != null) next.txDrive = clampTxDriveWatts(value);
  } else if (command.name === 'remote_tx_rf_enabled' || command.name === 'tx_rf_enabled') {
    const enabled = booleanArg(argAt(args, 1) ?? argAt(args, 0) ?? trailingArg(args));
    if (enabled != null) next.remoteTxRfEnabled = enabled;
  } else if (command.name === 'remote_client_role' || command.name === 'client_role') {
    const parsed = parseClientRole(args);
    if (parsed) {
      next.remoteClientRole = parsed.role;
      next.remoteClientId = parsed.id;
    }
  } else if (command.name === 'swr') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.swr = value;
  } else if (command.name === 'saturn_pong') {
    const sentAt = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (sentAt != null) {
      const receivedAt = nowMs();
      next.bridgeRttMs = Math.max(0, receivedAt - sentAt);
      next.bridgeRttAt = receivedAt;
    }
  } else if (command.name === 'remote_backpressure') {
    const offset = args.length >= 15 ? 1 : 0;
    next.backpressureSafetyP50Us = nonNegativeIntArg(args, offset) ?? next.backpressureSafetyP50Us;
    next.backpressureSafetyP95Us = nonNegativeIntArg(args, offset + 1) ?? next.backpressureSafetyP95Us;
    next.backpressureSafetyP99Us = nonNegativeIntArg(args, offset + 2) ?? next.backpressureSafetyP99Us;
    next.backpressureControlP50Us = nonNegativeIntArg(args, offset + 3) ?? next.backpressureControlP50Us;
    next.backpressureControlP95Us = nonNegativeIntArg(args, offset + 4) ?? next.backpressureControlP95Us;
    next.backpressureControlP99Us = nonNegativeIntArg(args, offset + 5) ?? next.backpressureControlP99Us;
    next.displayReplacedPerSec = nonNegativeIntArg(args, offset + 6) ?? next.displayReplacedPerSec;
    next.displayDroppedPerSec = nonNegativeIntArg(args, offset + 7) ?? next.displayDroppedPerSec;
    next.bridgeAudioDroppedPerSec = nonNegativeIntArg(args, offset + 8) ?? next.bridgeAudioDroppedPerSec;
    next.bridgeAudioSeqGapCount =
      nonNegativeIntArg(args, offset + 9) ?? next.bridgeAudioSeqGapCount;
    next.audioPanicDrainCount = nonNegativeIntArg(args, offset + 10) ?? next.audioPanicDrainCount;
    next.sendBlockedMs = nonNegativeIntArg(args, offset + 11) ?? next.sendBlockedMs;
    next.outboundHighWatermarkBytes =
      nonNegativeIntArg(args, offset + 12) ?? next.outboundHighWatermarkBytes;
    next.safetyQueueDepthOverflowCount =
      nonNegativeIntArg(args, offset + 13) ?? next.safetyQueueDepthOverflowCount;
  } else if (command.name === 'remote_tx_uplink') {
    const offset = args.length >= 8 ? 1 : 0;
    const degraded = booleanArg(argAt(args, offset));
    if (degraded != null) next.txUplinkDegraded = degraded;
    next.txMicDroppedCount = nonNegativeIntArg(args, offset + 1) ?? next.txMicDroppedCount;
    next.txUplinkBufferedBytes = nonNegativeIntArg(args, offset + 2) ?? next.txUplinkBufferedBytes;
    next.txUplinkBufferedHwmBytes =
      nonNegativeIntArg(args, offset + 3) ?? next.txUplinkBufferedHwmBytes;
    next.txMicLastArrivedSeq =
      nonNegativeIntArg(args, offset + 4) ?? next.txMicLastArrivedSeq;
    next.txMicSeqGapCount = nonNegativeIntArg(args, offset + 5) ?? next.txMicSeqGapCount;
    next.txMicAgeMs = nonNegativeIntArg(args, offset + 6) ?? next.txMicAgeMs;
  } else if (command.name === 'tx_codec_accept') {
    const codec = normalizeTxCodecName(argAt(args, txCodecArgOffset(args)));
    if (codec) {
      next.txCodecAccepted = codec;
      next.txCodecRequested = codec;
      next.txCodecNegotiatedAt = nowMs();
      next.txCodecRejectReason = null;
    }
  } else if (command.name === 'tx_codec_reject') {
    const offset = txCodecArgOffset(args);
    const codec = normalizeTxCodecName(argAt(args, offset));
    next.txCodecAccepted = null;
    if (codec) next.txCodecRequested = codec;
    next.txCodecNegotiatedAt = 0;
    next.txCodecRejectReason = String(argAt(args, offset + 1) ?? 'rejected').trim() || 'rejected';
  } else if (command.name === 'tx_fault') {
    txFault = describeTxFault(args);
    next.txFaultReason = txFault;
    if (next.txPhase === 'keyed' || next.txEnabled) txReleased = true;
    next.moxRequested = false;
    next.txEnabled = false;
    next.txPhase = 'rx';
  } else if (command.name === 'tx_state' && args.length >= 2) {
    const phase = String(argAt(args, 1) || '').trim().toLowerCase();
    if (phase === 'rx' || phase === 'armed' || phase === 'keyed') {
      const wasTxPhase = next.txPhase;
      next.txPhase = phase;
      if (phase === 'rx') {
        if (wasTxPhase === 'keyed' || next.txEnabled) txReleased = true;
        next.moxRequested = false;
        next.txEnabled = false;
      } else if (phase === 'armed') {
        next.moxRequested = true;
      } else if (phase === 'keyed') {
        next.moxRequested = true;
        next.txEnabled = true;
      }
    }
  } else if (command.name === 'trx' && args.length >= 2) {
    const on = String(argAt(args, 1) || '').trim().toLowerCase() === 'true';
    const wasTxEnabled = next.txEnabled;
    next.txEnabled = on;
    if (on) {
      next.moxRequested = true;
      if (next.txPhase !== 'keyed') next.txPhase = 'keyed';
    } else {
      if (wasTxEnabled) txReleased = true;
      if (wasTxEnabled || !next.moxRequested) {
        next.moxRequested = false;
        if (next.txPhase !== 'armed') next.txPhase = 'rx';
      }
    }
  } else if (command.name === 'tx_filter_band' && args.length >= 3) {
    const fl = numericArg(argAt(args, 1));
    const fh = numericArg(argAt(args, 2));
    if (fl != null && fh != null) {
      const cuts = uiCutsFromSignedPassband(fl, fh, next.mode);
      next.txFilterLow = cuts.lowHz;
      next.txFilterHigh = cuts.highHz;
    }
  } else if (command.name === 'rx_eq_enable') {
    next.rxEqEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
  } else if (command.name === 'tx_eq_enable') {
    next.txEqEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
  } else if (command.name === 'rx_eq_band' && args.length >= 3) {
    const parsed = parseBandGain(args);
    if (parsed) next.rxEqBands[parsed.band] = Math.round(parsed.gain);
  } else if (command.name === 'tx_eq_band' && args.length >= 3) {
    const parsed = parseBandGain(args);
    if (parsed) next.txEqBands[parsed.band] = Math.round(parsed.gain);
  } else if (command.name === 'tx_cfc_enable') {
    next.cfcEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
  } else if (command.name === 'tx_cfc_precomp') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.cfcPrecomp = value;
  } else if (command.name === 'tx_cfc_band' && args.length >= 3) {
    const parsed = parseBandGain(args);
    if (parsed) next.cfcBands[parsed.band] = Math.max(0, Math.min(20, parsed.gain));
  } else if (command.name === 'tx_two_tone') {
    next.twoToneEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
  } else if (command.name === 'tx_two_tone_freq1') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.txTwoToneFreq1 = clampTwoToneFreqHz(value, 700);
  } else if (command.name === 'tx_two_tone_freq2') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.txTwoToneFreq2 = clampTwoToneFreqHz(value, 1900);
  } else if (command.name === 'tx_two_tone_level_db') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.txTwoToneLevelDb = clampTwoToneLevelDb(value);
  } else if (command.name === 'tx_two_tone_invert_lsb') {
    next.txTwoToneInvertLsb = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
  } else if (command.name === 'tx_two_tone_delay_ms') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.txTwoToneDelayMs = clampTwoToneDelayMs(value);
  } else if (command.name === 'tx_noise_gate') {
    next.txNoiseGateEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
  } else if (command.name === 'tx_noise_gate_threshold') {
    const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
    if (value != null) next.txNoiseGateThresholdDb = Math.max(-80, Math.min(0, value));
  } else if (command.name === 'audio_start') {
    next.audioStreaming = true;
  } else if (command.name === 'audio_stop') {
    next.audioStreaming = false;
  } else if (command.name === 'audio_samplerate' && args.length >= 1) {
    const rate = numericArg(argAt(args, 0));
    if (rate != null && rate > 0) next.audioSampleRate = clampSampleRateHz(rate);
  }

  next.filterLow = clampFilterLowHz(next.filterLow);
  next.filterHigh = clampFilterHighHz(next.filterHigh);
  next.txFilterLow = clampFilterLowHz(next.txFilterLow);
  next.txFilterHigh = clampFilterHighHz(next.txFilterHigh);

  return { state: next, ready, displayCenterHzChanged, sampleRateChanged, rxVolumeChanged, txReleased, txFault };
}

export function applyTciText(text: string, current: TciRadioState): TciApplyResult {
  let result: TciApplyResult = {
    state: {
      ...current,
      rxEqBands: current.rxEqBands.slice(),
      txEqBands: current.txEqBands.slice(),
      cfcBands: current.cfcBands.slice(),
    },
    ready: false,
    displayCenterHzChanged: false,
    sampleRateChanged: false,
    rxVolumeChanged: false,
    txReleased: false,
    txFault: null,
  };

  for (const command of parseTciText(text)) {
    const applied = applyTciCommand(command, result.state);
    result = {
      state: applied.state,
      ready: result.ready || applied.ready,
      displayCenterHzChanged: result.displayCenterHzChanged || applied.displayCenterHzChanged,
      sampleRateChanged: result.sampleRateChanged || applied.sampleRateChanged,
      rxVolumeChanged: result.rxVolumeChanged || applied.rxVolumeChanged,
      txReleased: result.txReleased || applied.txReleased,
      txFault: applied.txFault ?? result.txFault,
    };
  }

  return result;
}
