export type DemodMode = 'USB' | 'LSB' | 'AM' | 'SAM' | 'FM' | 'DIGU' | 'DIGL' | 'CWU' | 'CWL';

export type Passband = {
  lowHz: number;
  highHz: number;
};

export const DEMOD_MODES: readonly DemodMode[] = ['USB', 'LSB', 'AM', 'SAM', 'FM', 'DIGU', 'DIGL', 'CWU', 'CWL'];

const NEGATIVE_PASSBAND_MODES = new Set<DemodMode>(['LSB', 'DIGL', 'CWL']);
const SYMMETRIC_PASSBAND_MODES = new Set<DemodMode>(['AM', 'SAM', 'FM']);

export function normalizeDemodMode(value: string | undefined | null): DemodMode {
  const normalized = String(value || '').trim().toUpperCase();
  return DEMOD_MODES.includes(normalized as DemodMode) ? (normalized as DemodMode) : 'USB';
}

export function clampDemodMode(value: string | undefined | null): DemodMode {
  return normalizeDemodMode(value);
}

export function isNegativePassbandMode(mode: string): boolean {
  return NEGATIVE_PASSBAND_MODES.has(normalizeDemodMode(mode));
}

export function isSymmetricPassbandMode(mode: string): boolean {
  return SYMMETRIC_PASSBAND_MODES.has(normalizeDemodMode(mode));
}

export function clampFilterLowHz(value: number | string): number {
  const n = Number(value);
  if (!Number.isFinite(n)) return 50;
  return Math.max(0, Math.min(300, Math.round(n)));
}

export function clampFilterHighHz(value: number | string): number {
  const n = Number(value);
  if (!Number.isFinite(n)) return 3_050;
  return Math.max(500, Math.min(6_000, Math.round(n)));
}

export function signedPassbandFromUiCuts(lowCutHz: number, highCutHz: number, mode: string): Passband {
  const normalized = normalizeDemodMode(mode);
  const low = clampFilterLowHz(lowCutHz);
  const high = Math.max(low + 1, clampFilterHighHz(highCutHz));

  if (SYMMETRIC_PASSBAND_MODES.has(normalized)) {
    const edge = Math.max(Math.abs(low), Math.abs(high));
    return { lowHz: -edge, highHz: edge };
  }

  if (NEGATIVE_PASSBAND_MODES.has(normalized)) {
    return { lowHz: -high, highHz: -low };
  }

  return { lowHz: low, highHz: high };
}

export function defaultSignedRxPassbandForMode(mode: string): Passband {
  switch (normalizeDemodMode(mode)) {
    case 'LSB':
    case 'DIGL':
      return { lowHz: -3000, highHz: -300 };
    case 'CWL':
      return { lowHz: -800, highHz: -200 };
    case 'CWU':
      return { lowHz: 200, highHz: 800 };
    case 'AM':
    case 'SAM':
      return { lowHz: -4000, highHz: 4000 };
    case 'FM':
      return { lowHz: -6000, highHz: 6000 };
    case 'DIGU':
      return { lowHz: 300, highHz: 3000 };
    case 'USB':
    default:
      return { lowHz: 50, highHz: 3050 };
  }
}

export function defaultSignedTxPassbandForMode(mode: string): Passband {
  switch (normalizeDemodMode(mode)) {
    case 'LSB':
      return { lowHz: -3000, highHz: -300 };
    case 'DIGL':
      return { lowHz: -3000, highHz: 0 };
    case 'AM':
    case 'SAM':
    case 'FM':
      return { lowHz: -3000, highHz: 3000 };
    case 'CWL':
      return { lowHz: -800, highHz: -200 };
    case 'CWU':
      return { lowHz: 200, highHz: 800 };
    case 'DIGU':
      return { lowHz: 0, highHz: 3000 };
    case 'USB':
    default:
      return { lowHz: 50, highHz: 3050 };
  }
}

export function defaultFilterCutsForMode(mode: string): { rxLow: number; rxHigh: number; txLow: number; txHigh: number } {
  const rx = defaultSignedRxPassbandForMode(mode);
  const rxCuts = uiCutsFromSignedPassband(rx.lowHz, rx.highHz, mode);
  const tx = defaultSignedTxPassbandForMode(mode);
  const txCuts = uiCutsFromSignedPassband(tx.lowHz, tx.highHz, mode);
  return { rxLow: rxCuts.lowHz, rxHigh: rxCuts.highHz, txLow: txCuts.lowHz, txHigh: txCuts.highHz };
}

export function uiCutsFromSignedPassband(lowHz: number, highHz: number, mode: string): Passband {
  const normalized = normalizeDemodMode(mode);

  if (SYMMETRIC_PASSBAND_MODES.has(normalized)) {
    const edge = Math.max(Math.abs(lowHz), Math.abs(highHz));
    return { lowHz: 0, highHz: clampFilterHighHz(edge) };
  }

  if (NEGATIVE_PASSBAND_MODES.has(normalized)) {
    return {
      lowHz: clampFilterLowHz(Math.abs(highHz)),
      highHz: clampFilterHighHz(Math.abs(lowHz)),
    };
  }

  return {
    lowHz: clampFilterLowHz(lowHz),
    highHz: clampFilterHighHz(highHz),
  };
}

export function clickTuneCarrierOffsetHz(filterLow: number, filterHigh: number, mode: string): number {
  const normalized = normalizeDemodMode(mode);
  if (
    normalized !== 'LSB' && normalized !== 'DIGL' &&
    normalized !== 'DIGU' && normalized !== 'CWL' && normalized !== 'CWU'
  ) {
    return 0;
  }
  const passband = signedPassbandFromUiCuts(filterLow, filterHigh, normalized);
  return (passband.lowHz + passband.highHz) / 2;
}
