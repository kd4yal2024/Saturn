export type DemodMode = 'USB' | 'LSB' | 'AM' | 'SAM' | 'FM' | 'WFM' | 'DIGU' | 'DIGL' | 'CWU' | 'CWL';

export type Passband = {
  lowHz: number;
  highHz: number;
};

export const DEMOD_MODES: readonly DemodMode[] = ['USB', 'LSB', 'AM', 'SAM', 'FM', 'WFM', 'DIGU', 'DIGL', 'CWU', 'CWL'];

const NEGATIVE_PASSBAND_MODES = new Set<DemodMode>(['LSB', 'DIGL', 'CWL']);
const SYMMETRIC_PASSBAND_MODES = new Set<DemodMode>(['AM', 'SAM', 'FM', 'WFM']);

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
  if (normalized === 'WFM') return { lowHz: -90_000, highHz: 90_000 };
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

export function shiftedSignedPassbandFromUiCuts(
  lowCutHz: number,
  highCutHz: number,
  mode: string,
  shiftHz = 0,
): Passband {
  const passband = signedPassbandFromUiCuts(lowCutHz, highCutHz, mode);
  if (isSymmetricPassbandMode(mode)) return passband;
  const shift = Number.isFinite(Number(shiftHz)) ? Math.round(Number(shiftHz)) : 0;
  return {
    lowHz: passband.lowHz + shift,
    highHz: passband.highHz + shift,
  };
}

export function decomposeSignedPassbandWithShift(
  lowHz: number,
  highHz: number,
  mode: string,
  preferredLowCutHz = 50,
): { lowCutHz: number; highCutHz: number; shiftHz: number } {
  if (normalizeDemodMode(mode) === 'WFM') {
    return { lowCutHz: 0, highCutHz: 90_000, shiftHz: 0 };
  }
  if (isSymmetricPassbandMode(mode)) {
    const cuts = uiCutsFromSignedPassband(lowHz, highHz, mode);
    return { lowCutHz: cuts.lowHz, highCutHz: cuts.highHz, shiftHz: 0 };
  }

  let low = Math.round(Number(lowHz));
  let high = Math.round(Number(highHz));
  if (!Number.isFinite(low)) low = 50;
  if (!Number.isFinite(high)) high = 3050;
  if (low > high) [low, high] = [high, low];

  const width = Math.max(1, Math.min(6000, high - low));
  if (!isNegativePassbandMode(mode) && low >= 0 && low <= 300 && high >= 500 && high <= 6000) {
    return { lowCutHz: low, highCutHz: high, shiftHz: 0 };
  }
  if (isNegativePassbandMode(mode)) {
    const zeroShiftLowCut = Math.abs(high);
    const zeroShiftHighCut = Math.abs(low);
    if (
      zeroShiftLowCut >= 0 &&
      zeroShiftLowCut <= 300 &&
      zeroShiftHighCut >= 500 &&
      zeroShiftHighCut <= 6000
    ) {
      return { lowCutHz: zeroShiftLowCut, highCutHz: zeroShiftHighCut, shiftHz: 0 };
    }
  }

  const maxLowCut = Math.max(0, Math.min(300, 6000 - width));
  const lowCut = Math.max(0, Math.min(maxLowCut, clampFilterLowHz(preferredLowCutHz)));
  const highCut = Math.max(lowCut + 1, Math.min(6000, lowCut + width));
  const shift = isNegativePassbandMode(mode)
    ? high + lowCut
    : low - lowCut;

  return {
    lowCutHz: lowCut,
    highCutHz: highCut,
    shiftHz: Number.isFinite(shift) ? Math.round(shift) : 0,
  };
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
    case 'WFM':
      return { lowHz: -90_000, highHz: 90_000 };
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
    case 'WFM':
      return { lowHz: -3000, highHz: 3000 };
    case 'CWL':
      return { lowHz: -800, highHz: -200 };
    case 'CWU':
      return { lowHz: 200, highHz: 800 };
    case 'DIGU':
      return { lowHz: 0, highHz: 3000 };
    case 'USB':
    default:
      return { lowHz: 250, highHz: 3000 };
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
    return { lowHz: 0, highHz: normalized === 'WFM' ? 90_000 : clampFilterHighHz(edge) };
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
