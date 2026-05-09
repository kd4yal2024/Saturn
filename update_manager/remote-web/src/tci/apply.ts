import { parseTciText, type TciCommand, booleanArg, numericArg, trailingArg } from './parser';
import { clampDemodMode, uiCutsFromSignedPassband } from '../radio/passband';
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
import type { TciApplyResult, TciRadioState } from './state';

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

  if (command.name === 'ready') {
    ready = true;
    return { state: next, ready, displayCenterHzChanged, sampleRateChanged, rxVolumeChanged, txReleased };
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
      const cuts = uiCutsFromSignedPassband(fl, fh, next.mode);
      next.filterLow = cuts.lowHz;
      next.filterHigh = cuts.highHz;
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

  return { state: next, ready, displayCenterHzChanged, sampleRateChanged, rxVolumeChanged, txReleased };
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
    };
  }

  return result;
}
