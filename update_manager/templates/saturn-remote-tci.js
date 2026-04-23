(function () {
  const DEMOD_MODES = ["USB", "LSB", "AM", "SAM", "FM", "DIGU", "DIGL", "CWU", "CWL"];
  const NEGATIVE_PASSBAND_MODES = new Set(["LSB", "DIGL", "CWL"]);
  const SYMMETRIC_PASSBAND_MODES = new Set(["AM", "SAM", "FM"]);

  function argAt(args, index) {
    return index >= 0 && index < args.length ? args[index] : undefined;
  }

  function trailingArg(args) {
    return args.length > 0 ? args[args.length - 1] : undefined;
  }

  function numericArg(value) {
    if (value == null) return null;
    const parsed = Number(String(value).trim());
    return Number.isFinite(parsed) ? parsed : null;
  }

  function booleanArg(value) {
    if (value == null) return null;
    const normalized = String(value).trim().toLowerCase();
    if (["1", "true", "on", "yes"].includes(normalized)) return true;
    if (["0", "false", "off", "no"].includes(normalized)) return false;
    return null;
  }

  function normalizeDemodMode(value) {
    const normalized = String(value || "").trim().toUpperCase();
    return DEMOD_MODES.includes(normalized) ? normalized : "USB";
  }

  function clampFilterLowHz(value) {
    if (!Number.isFinite(value)) return 50;
    return Math.max(0, Math.min(300, Math.round(value)));
  }

  function clampFilterHighHz(value) {
    if (!Number.isFinite(value)) return 3050;
    return Math.max(500, Math.min(6000, Math.round(value)));
  }

  function clampRxAdc(value) {
    if (!Number.isFinite(value)) return 0;
    return Math.max(0, Math.min(2, Math.round(value)));
  }

  function clampRxAntenna(value) {
    if (!Number.isFinite(value)) return 1;
    return Math.max(1, Math.min(3, Math.round(value)));
  }

  function clampSampleRateHz(value) {
    if (!Number.isFinite(value)) return 192000;
    const allowed = [48000, 96000, 192000, 384000];
    const rounded = Math.round(value);
    return allowed.includes(rounded) ? rounded : 192000;
  }

  function clampRxVolumeDb(value) {
    if (!Number.isFinite(value)) return -10;
    return Math.max(-40, Math.min(12, Math.round(value * 10) / 10));
  }

  function clampRxNoiseReductionLevel(value) {
    if (!Number.isFinite(value)) return 10;
    return Math.max(0, Math.min(100, Math.round(value)));
  }

  function clampRxNbThreshold(value) {
    if (!Number.isFinite(value)) return 4.95;
    return Math.max(2.5, Math.min(82.5, value));
  }

  function clampDspTapCount(value) {
    if (!Number.isFinite(value)) return 64;
    return Math.max(1, Math.min(128, Math.round(value)));
  }

  function clampDspDelay(value) {
    if (!Number.isFinite(value)) return 16;
    return Math.max(0, Math.min(127, Math.round(value)));
  }

  function clampDspGain(value, fallback) {
    if (!Number.isFinite(value)) return fallback;
    return Math.max(0, Math.min(1, value));
  }

  function clampDspLeakage(value, fallback) {
    if (!Number.isFinite(value)) return fallback;
    return Math.max(0, Math.min(1, value));
  }

  function clampAgcGain(value) {
    if (!Number.isFinite(value)) return 80;
    return Math.max(0, Math.min(100, Math.round(value)));
  }

  function clampTxDriveWatts(value) {
    if (!Number.isFinite(value)) return 100;
    return Math.max(0, Math.min(100, Math.round(value)));
  }

  function clampTwoToneFreqHz(value, fallback) {
    if (!Number.isFinite(value)) return fallback;
    return Math.max(10, Math.min(10000, Math.round(value)));
  }

  function clampTwoToneLevelDb(value) {
    if (!Number.isFinite(value)) return 0;
    return Math.max(-40, Math.min(0, Math.round(value * 10) / 10));
  }

  function clampTwoToneDelayMs(value) {
    if (!Number.isFinite(value)) return 0;
    return Math.max(0, Math.min(2000, Math.round(value)));
  }

  function normalizeNrMode(value) {
    const normalized = String(value || "").trim().toUpperCase();
    if (["NR1", "ANR", "1"].includes(normalized)) return "NR1";
    if (["NR2", "EMNR", "2"].includes(normalized)) return "NR2";
    if (["NR3", "RNNR", "3"].includes(normalized)) return "NR3";
    if (["NR4", "SBNR", "4"].includes(normalized)) return "NR4";
    return "OFF";
  }

  function normalizeNbMode(value) {
    const normalized = String(value || "").trim().toUpperCase();
    if (["NB1", "NB", "1"].includes(normalized)) return "NB1";
    if (["NB2", "NOB", "2"].includes(normalized)) return "NB2";
    return "OFF";
  }

  function normalizeAgcMode(value) {
    const normalized = String(value || "").trim().toUpperCase();
    if (["OFF", "0"].includes(normalized)) return "OFF";
    if (["LONG", "1"].includes(normalized)) return "LONG";
    if (["SLOW", "2"].includes(normalized)) return "SLOW";
    if (["FAST", "4"].includes(normalized)) return "FAST";
    return "MEDIUM";
  }

  function uiCutsFromSignedPassband(lowHz, highHz, mode) {
    const normalized = normalizeDemodMode(mode);
    if (SYMMETRIC_PASSBAND_MODES.has(normalized)) {
      const edge = Math.max(Math.abs(lowHz), Math.abs(highHz));
      return { lowCutHz: 0, highCutHz: clampFilterHighHz(edge) };
    }
    if (NEGATIVE_PASSBAND_MODES.has(normalized)) {
      return {
        lowCutHz: clampFilterLowHz(Math.abs(highHz)),
        highCutHz: clampFilterHighHz(Math.abs(lowHz)),
      };
    }
    return {
      lowCutHz: clampFilterLowHz(lowHz),
      highCutHz: clampFilterHighHz(highHz),
    };
  }

  function cloneState(current) {
    return {
      ...current,
      rxEqBands: Array.isArray(current.rxEqBands) ? current.rxEqBands.slice() : [],
      txEqBands: Array.isArray(current.txEqBands) ? current.txEqBands.slice() : [],
      cfcBands: Array.isArray(current.cfcBands) ? current.cfcBands.slice() : [],
    };
  }

  function applyCommands(commands, current) {
    const next = cloneState(current);
    const events = {
      ready: false,
      displayCenterHzChanged: false,
      sampleRateChanged: false,
      rxVolumeChanged: false,
      txReleased: false,
    };

    for (const command of commands) {
      const name = String(command?.name || "").toLowerCase();
      const args = Array.isArray(command?.args) ? command.args : [];
      if (name === "ready") {
        events.ready = true;
        continue;
      }
      if (!String(command?.raw || "").includes(":")) {
        continue;
      }
      if (name === "vfo" && args.length >= 3) {
        const which = argAt(args, 1);
        const hz = numericArg(argAt(args, 2));
        if (hz != null) {
          if (which === "0") next.vfoA = hz;
          if (which === "1") next.vfoB = hz;
        }
      } else if (name === "dds" && args.length >= 2) {
        const dds = numericArg(argAt(args, 1));
        if (dds != null && dds !== next.dds) {
          next.dds = dds;
          events.displayCenterHzChanged = true;
        }
      } else if (name === "rx_adc") {
        const adc = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (adc != null) next.rxAdc = clampRxAdc(adc);
      } else if (name === "rx_antenna") {
        const antenna = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (antenna != null) next.rxAntenna = clampRxAntenna(antenna);
      } else if (name === "iq_samplerate" && args.length >= 1) {
        const rate = numericArg(argAt(args, 0));
        if (rate != null && rate > 0 && rate !== next.sampleRate) {
          next.sampleRate = rate;
          events.sampleRateChanged = true;
        }
      } else if (name === "modulation" && args.length >= 2) {
        next.mode = normalizeDemodMode(argAt(args, 1));
      } else if (name === "rx_volume") {
        const volume = numericArg(argAt(args, 2) ?? trailingArg(args));
        if (volume != null) {
          next.rxVolumeDb = clampRxVolumeDb(volume);
          events.rxVolumeChanged = true;
        }
      } else if (name === "rx_nr_mode" || name === "nr_mode") {
        next.rxNoiseReductionMode = normalizeNrMode(argAt(args, 1) ?? trailingArg(args));
      } else if (name === "rx_nr" || name === "nr") {
        const enabled = booleanArg(argAt(args, 1) ?? trailingArg(args));
        if (enabled === false) next.rxNoiseReductionMode = "OFF";
        if (enabled === true && next.rxNoiseReductionMode === "OFF") next.rxNoiseReductionMode = "NR1";
      } else if (name === "rx_nr_level" || name === "nr_level") {
        const level = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (level != null) next.rxNoiseReductionLevel = clampRxNoiseReductionLevel(level);
      } else if (name === "rx_nb") {
        next.rxNbMode = normalizeNbMode(argAt(args, 1) ?? trailingArg(args));
      } else if (name === "rx_nb_threshold") {
        const threshold = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (threshold != null) next.rxNbThreshold = clampRxNbThreshold(threshold);
      } else if (name === "rx_anr_taps") {
        const taps = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (taps != null) next.rxAnrTaps = clampDspTapCount(taps);
      } else if (name === "rx_anr_delay") {
        const delay = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (delay != null) next.rxAnrDelay = clampDspDelay(delay);
      } else if (name === "rx_anr_gain") {
        const gain = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (gain != null) next.rxAnrGain = clampDspGain(gain, 0.0002);
      } else if (name === "rx_anr_leakage") {
        const leakage = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (leakage != null) next.rxAnrLeakage = clampDspLeakage(leakage, 0.00005);
      } else if (name === "rx_anf") {
        next.anfEnabled = booleanArg(argAt(args, 1) ?? trailingArg(args)) === true;
      } else if (name === "rx_anf_taps") {
        const taps = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (taps != null) next.rxAnfTaps = clampDspTapCount(taps);
      } else if (name === "rx_anf_delay") {
        const delay = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (delay != null) next.rxAnfDelay = clampDspDelay(delay);
      } else if (name === "rx_anf_gain") {
        const gain = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (gain != null) next.rxAnfGain = clampDspGain(gain, 0.00012);
      } else if (name === "rx_anf_leakage") {
        const leakage = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (leakage != null) next.rxAnfLeakage = clampDspLeakage(leakage, 0.00008);
      } else if (name === "rx_agc") {
        next.agcMode = normalizeAgcMode(argAt(args, 1) ?? trailingArg(args));
      } else if (name === "rx_agc_gain") {
        const gain = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (gain != null) next.agcGain = clampAgcGain(gain);
      } else if (name === "rx_filter_band" && args.length >= 3) {
        const fl = numericArg(argAt(args, 1));
        const fh = numericArg(argAt(args, 2));
        if (fl != null && fh != null) {
          const cuts = uiCutsFromSignedPassband(fl, fh, next.mode);
          next.filterLow = cuts.lowCutHz;
          next.filterHigh = cuts.highCutHz;
        }
      } else if (name === "rx_smeter") {
        const meter = numericArg(argAt(args, 2) ?? argAt(args, 0));
        if (meter != null) next.meterDbm = meter;
      } else if (name === "tx_power") {
        const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
        if (value != null) next.txPower = value;
      } else if (name === "tx_drive") {
        const value = numericArg(argAt(args, 1) ?? trailingArg(args));
        if (value != null) next.txDrive = clampTxDriveWatts(value);
      } else if (name === "swr") {
        const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
        if (value != null) next.swr = value;
      } else if (name === "trx" && args.length >= 2) {
        const on = String(argAt(args, 1) || "").trim().toLowerCase() === "true";
        const wasTxEnabled = next.txEnabled;
        next.txEnabled = on;
        if (on) {
          next.moxRequested = true;
        } else {
          if (wasTxEnabled) events.txReleased = true;
          next.moxRequested = false;
        }
      } else if (name === "tx_filter_band" && args.length >= 3) {
        const fl = numericArg(argAt(args, 1));
        const fh = numericArg(argAt(args, 2));
        if (fl != null && fh != null) {
          const cuts = uiCutsFromSignedPassband(fl, fh, next.mode);
          next.txFilterLow = cuts.lowCutHz;
          next.txFilterHigh = cuts.highCutHz;
        }
      } else if (name === "rx_eq_enable") {
        next.rxEqEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
      } else if (name === "tx_eq_enable") {
        next.txEqEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
      } else if (name === "rx_eq_band" && args.length >= 3) {
        const band = numericArg(argAt(args, 1));
        const gain = numericArg(argAt(args, 2));
        if (band != null && gain != null && band >= 1 && band <= 10) next.rxEqBands[band] = Math.round(gain);
      } else if (name === "tx_eq_band" && args.length >= 3) {
        const band = numericArg(argAt(args, 1));
        const gain = numericArg(argAt(args, 2));
        if (band != null && gain != null && band >= 1 && band <= 10) next.txEqBands[band] = Math.round(gain);
      } else if (name === "tx_cfc_enable") {
        next.cfcEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
      } else if (name === "tx_cfc_precomp") {
        const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
        if (value != null) next.cfcPrecomp = value;
      } else if (name === "tx_cfc_band" && args.length >= 3) {
        const band = numericArg(argAt(args, 1));
        const gain = numericArg(argAt(args, 2));
        if (band != null && gain != null && band >= 1 && band <= 10) next.cfcBands[band] = Math.max(0, Math.min(20, gain));
      } else if (name === "tx_two_tone") {
        next.twoToneEnabled = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
      } else if (name === "tx_two_tone_freq1") {
        const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
        if (value != null) next.txTwoToneFreq1 = clampTwoToneFreqHz(value, 700);
      } else if (name === "tx_two_tone_freq2") {
        const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
        if (value != null) next.txTwoToneFreq2 = clampTwoToneFreqHz(value, 1900);
      } else if (name === "tx_two_tone_level_db") {
        const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
        if (value != null) next.txTwoToneLevelDb = clampTwoToneLevelDb(value);
      } else if (name === "tx_two_tone_invert_lsb") {
        next.txTwoToneInvertLsb = booleanArg(argAt(args, 1) ?? argAt(args, 0)) === true;
      } else if (name === "tx_two_tone_delay_ms") {
        const value = numericArg(argAt(args, 1) ?? argAt(args, 0));
        if (value != null) next.txTwoToneDelayMs = clampTwoToneDelayMs(value);
      } else if (name === "audio_start") {
        next.audioStreaming = true;
      } else if (name === "audio_stop") {
        next.audioStreaming = false;
      } else if (name === "audio_samplerate" && args.length >= 1) {
        const rate = numericArg(argAt(args, 0));
        if (rate != null && rate > 0) next.audioSampleRate = clampSampleRateHz(rate);
      }
    }

    next.mode = normalizeDemodMode(next.mode);
    next.filterLow = clampFilterLowHz(next.filterLow);
    next.filterHigh = clampFilterHighHz(next.filterHigh);
    next.txFilterLow = clampFilterLowHz(next.txFilterLow);
    next.txFilterHigh = clampFilterHighHz(next.txFilterHigh);

    return { state: next, events: events };
  }

  window.SaturnRemoteTci = {
    applyCommands,
  };
})();
