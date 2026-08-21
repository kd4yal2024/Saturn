/**
 * Pure math helpers for analog meter rendering (S-meter, TX power, SWR).
 * No DOM dependencies — only numeric computations.
 */

/** Linear interpolation. */
export function lerp(start: number, end: number, t: number): number {
  return start + (end - start) * t;
}

/**
 * Move a value toward a target with asymmetric attack/release rates.
 * Returns current if target is not finite; returns target if current is not finite.
 */
export function moveToward(
  current: number | null,
  target: number,
  risePerSec: number,
  fallPerSec: number,
  dtSec: number,
): number {
  if (!Number.isFinite(target)) return current ?? 0;
  if (current == null || !Number.isFinite(current)) return target;
  if (target >= current) {
    return Math.min(target, current + risePerSec * dtSec);
  }
  return Math.max(target, current - fallPerSec * dtSec);
}

/** SVG arc path string between two angles (degrees, clockwise from 3-o'clock). */
export function svgArcPath(
  cx: number, cy: number, r: number,
  startDeg: number, endDeg: number,
): string {
  const rad = (d: number) => d * Math.PI / 180;
  const sx = cx + r * Math.cos(rad(startDeg));
  const sy = cy + r * Math.sin(rad(startDeg));
  const ex = cx + r * Math.cos(rad(endDeg));
  const ey = cy + r * Math.sin(rad(endDeg));
  const large = ((endDeg - startDeg + 360) % 360) > 180 ? 1 : 0;
  return `M ${sx.toFixed(2)} ${sy.toFixed(2)} A ${r} ${r} 0 ${large} 1 ${ex.toFixed(2)} ${ey.toFixed(2)}`;
}

// ── S-Meter ──────────────────────────────────────────────────────────────────

/** S-Meter geometry constants. */
export const SM_CX = 180, SM_CY = 165, SM_R = 130;
export const SM_START = 210, SM_END = 330;
export const SM_S9_DEG = SM_START + ((-73 + 121) / 108) * 120;

/** Convert dBm to S-meter SVG degrees. */
export function dbmToSmDeg(dbm: number): number {
  return SM_START + Math.max(0, Math.min(1, (dbm + 121) / 108)) * 120;
}

/**
 * Format an HF S-meter value without pretending that a missing sample is S0.
 * S1 starts at -121 dBm, S9 is -73 dBm, and readings above S9 are expressed
 * as decibels over S9 in the familiar transceiver form.
 */
export function formatSignalStrength(dbm: number | null): string {
  if (dbm == null || !Number.isFinite(dbm)) return '--';
  if (dbm < -121) return '< S1';
  if (dbm < -73) {
    const sUnit = Math.min(8, Math.max(1, Math.floor((dbm + 127) / 6)));
    return `S${sUnit}`;
  }
  const overS9 = Math.max(0, Math.round(dbm + 73));
  return overS9 === 0 ? 'S9' : `S9 +${overS9}`;
}

/** Convert a normalized linear peak to a stable dBFS readout. */
export function linearLevelToDbfs(level: number | null, floorDb = -120): number | null {
  if (level == null || !Number.isFinite(level)) return null;
  const floor = Math.min(0, Number.isFinite(floorDb) ? floorDb : -120);
  if (level <= 0) return floor;
  return Math.max(floor, Math.min(0, 20 * Math.log10(level)));
}

export type InstrumentOperatingState = 'rx' | 'armed' | 'tx';

/** Derive presentation state only from authoritative TX state. */
export function instrumentOperatingState(
  txPhase: string,
  moxRequested: boolean,
  txEnabled: boolean,
  txReady = false,
): InstrumentOperatingState {
  if (txPhase === 'keyed' || txEnabled) return 'tx';
  if (txPhase === 'armed' || moxRequested || txReady) return 'armed';
  return 'rx';
}

// ── Aux meters (TX Power, SWR) ──────────────────────────────────────────────

/** Aux meter geometry constants. */
export const AX_CX = 87, AX_CY = 100, AX_R = 68;
export const AX_START = 205, AX_END = 335;
export const AX_SPAN = AX_END - AX_START;

/** Map a linear value to aux-meter SVG degrees. */
export function auxDeg(value: number, min: number, max: number): number {
  return AX_START + Math.max(0, Math.min(1, (value - min) / (max - min))) * AX_SPAN;
}

/** Map TX power (watts) to aux-meter SVG degrees (log-ish scale). */
export function txPowerToAuxDeg(watts: number): number {
  const w = Math.max(0, Math.min(120, Number(watts) || 0));
  if (w <= 5)   return lerp(AX_START, AX_START + AX_SPAN * 0.1875, w / 5);
  if (w <= 10)  return lerp(AX_START + AX_SPAN * 0.1875, AX_START + AX_SPAN * 0.375, (w - 5) / 5);
  if (w <= 50)  return lerp(AX_START + AX_SPAN * 0.375, AX_START + AX_SPAN * 0.5625, (w - 10) / 40);
  if (w <= 100) return lerp(AX_START + AX_SPAN * 0.5625, AX_START + AX_SPAN * 0.75, (w - 50) / 50);
  return lerp(AX_START + AX_SPAN * 0.75, AX_END, (w - 100) / 20);
}

/** Map SWR (1.0–5.0) to aux-meter SVG degrees. */
export function swrToAuxDeg(swr: number): number {
  const value = Math.max(1, Math.min(5, Number(swr) || 1));
  if (value <= 3) {
    return lerp(AX_START, AX_START + AX_SPAN * 0.75, (value - 1) / 2);
  }
  return lerp(AX_START + AX_SPAN * 0.75, AX_END, (value - 3) / 2);
}

// ── Meter dynamics constants ────────────────────────────────────────────────

export const SMETER_ATTACK_PER_SEC = 85;
export const SMETER_RELEASE_PER_SEC = 22;
export const SMETER_PEAK_HOLD_MS = 900;
export const SMETER_PEAK_DROP_PER_SEC = 18;

export const TX_POWER_ATTACK_PER_SEC = 140;
export const TX_POWER_RELEASE_PER_SEC = 45;
export const TX_POWER_PEAK_HOLD_MS = 1100;
export const TX_POWER_PEAK_DROP_PER_SEC = 40;

export const SWR_ATTACK_PER_SEC = 5;
export const SWR_RELEASE_PER_SEC = 3;
