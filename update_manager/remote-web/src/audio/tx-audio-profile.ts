import { signedPassbandFromUiCuts } from '../radio/passband';

export type TxAudioProfile = Readonly<{
  id: string;
  label: string;
  description: string;
  filterLowHz: number;
  filterHighHz: number;
  micGainDb: number;
  eqEnabled: boolean;
  eqBands: readonly number[];
  cfcEnabled: boolean;
  cfcPrecompDb: number;
  cfcBands: readonly number[];
  noiseGateEnabled: boolean;
  noiseGateThresholdDb: number;
}>;

const VOODOO_38K_EQ_BANDS = Object.freeze([
  0, // WDSP graph-EQ preamp slot
  0, // 32 Hz: outside the occupied passband
  2, // 63 Hz
  4, // 125 Hz
  3, // 250 Hz
  1, // 500 Hz
  -1, // 1 kHz
  0, // 2 kHz
  2, // 4 kHz: presence at the upper shoulder
  0, // 8 kHz: outside the occupied passband
  0, // 16 kHz: outside the occupied passband
]);

const VOODOO_38K_CFC_BANDS = Object.freeze([
  0, // unused one-based UI slot
  0.5, // 50 Hz
  1.0, // 100 Hz
  1.5, // 200 Hz
  2.0, // 500 Hz
  1.5, // 1 kHz
  1.0, // 1.5 kHz
  0.5, // 2.5 kHz
  0.5, // 3 kHz
  0.0, // 5 kHz
  0.0, // 8 kHz
]);

/**
 * Warm, intelligible ESSB voice profile with 3.8 kHz occupied audio width.
 * The curve is intentionally restrained so widening the passband does not
 * turn the former out-of-band bass boosts into ALC-driving gain.
 */
export const VOODOO_38K_TX_AUDIO_PROFILE: TxAudioProfile = Object.freeze({
  id: 'voodoo-38k',
  label: 'Voodoo 3.8k',
  description: 'Warm ESSB voice, 50-3850 Hz, controlled presence',
  filterLowHz: 50,
  filterHighHz: 3_850,
  micGainDb: -6,
  eqEnabled: true,
  eqBands: VOODOO_38K_EQ_BANDS,
  cfcEnabled: true,
  cfcPrecompDb: 1,
  cfcBands: VOODOO_38K_CFC_BANDS,
  noiseGateEnabled: false,
  noiseGateThresholdDb: -50,
});

export function voodoo38kTxAudioProfile(): TxAudioProfile {
  return {
    ...VOODOO_38K_TX_AUDIO_PROFILE,
    eqBands: [...VOODOO_38K_TX_AUDIO_PROFILE.eqBands],
    cfcBands: [...VOODOO_38K_TX_AUDIO_PROFILE.cfcBands],
  };
}

/** Build only TX-audio commands. This intentionally excludes drive, tuning, PTT, and RF state. */
export function buildTxAudioProfileCommands(profile: TxAudioProfile, mode: string): string[] {
  const passband = signedPassbandFromUiCuts(profile.filterLowHz, profile.filterHighHz, mode);
  const commands = [
    `tx_mic_gain:0,${profile.micGainDb.toFixed(1)};`,
    `tx_filter_band:0,${passband.lowHz},${passband.highHz};`,
    `tx_noise_gate:0,${profile.noiseGateEnabled};`,
    `tx_noise_gate_threshold:0,${profile.noiseGateThresholdDb.toFixed(1)};`,
    `tx_eq_enable:0,${profile.eqEnabled};`,
    `tx_cfc_enable:0,${profile.cfcEnabled};`,
    `tx_cfc_precomp:0,${profile.cfcPrecompDb.toFixed(1)};`,
  ];
  for (let index = 1; index <= 10; index += 1) {
    commands.push(`tx_eq_band:0,${index},${Number(profile.eqBands[index] || 0).toFixed(0)};`);
    commands.push(`tx_cfc_band:0,${index},${Number(profile.cfcBands[index] || 0).toFixed(1)};`);
  }
  return commands;
}
