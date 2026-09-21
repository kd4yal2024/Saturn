/** Local presentation only. Never included in radio command construction. */
export type TerrainSettings = {
  mode: 'traditional' | '3d'; height: number; depth: number; elevation: number;
  floor: number; ceiling: number; gamma: number; smoothing: number;
  palette: string; split: number; quality: 'auto' | 'performance' | 'balanced' | 'high';
  diagnostics: boolean; gridOpacity: number; cleanup: number; cleanupBaseline: number | null;
};
export const TERRAIN_DEFAULTS: TerrainSettings = {
  mode: 'traditional', height: 0.65, depth: 128, elevation: 35,
  floor: -140, ceiling: -40, gamma: 0.85, smoothing: 0,
  palette: 'reference', split: 0.3, quality: 'auto', diagnostics: false, gridOpacity: 0, cleanup: 0.6, cleanupBaseline: null,
};
export function normalizeTerrain(input: unknown): TerrainSettings {
  const s = input && typeof input === 'object' ? input as Record<string, unknown> : {};
  const n = (key: keyof TerrainSettings, min: number, max: number) =>
    typeof s[key] === 'number' && Number.isFinite(s[key])
      ? Math.max(min, Math.min(max, s[key] as number)) : TERRAIN_DEFAULTS[key] as number;
  const floor = n('floor', -200, 19);
  return {
    mode: s.mode === '3d' ? '3d' : 'traditional', height: n('height', 0, 1),
    depth: Math.round(n('depth', 8, 256)), elevation: n('elevation', 15, 60),
    floor, ceiling: Math.max(floor + 1, n('ceiling', -199, 20)), gamma: n('gamma', 0.3, 3),
    smoothing: n('smoothing', 0, 1), split: n('split', 0.2, 0.65),
    palette: ['reference', 'reference-dark', 'classic', 'ember', 'ice', 'forest', 'enhanced'].includes(String(s.palette)) ? String(s.palette) : 'reference',
    quality: s.quality === 'performance' || s.quality === 'balanced' || s.quality === 'high' ? s.quality : 'auto',
    diagnostics: s.diagnostics === true, gridOpacity: n('gridOpacity', 0, 1),
    cleanup: n('cleanup', 0, .85),
    cleanupBaseline: typeof s.cleanupBaseline === 'number' && Number.isFinite(s.cleanupBaseline) ? Math.max(-200,Math.min(20,s.cleanupBaseline)) : null,
  };
}
export const REFERENCE_DARK_STOPS = [
  [0, 1, 3, 12], [0.12, 6, 13, 24], [0.30, 24, 48, 80],
  [0.48, 35, 132, 153], [0.62, 75, 160, 102], [0.76, 208, 190, 79],
  [0.87, 231, 133, 58], [0.96, 222, 65, 48], [1, 255, 255, 255],
];
// Color only: rich low-level blues without changing the selected dB range or height.
export const REFERENCE_STOPS = [
  [0, 1, 3, 12], [0.04, 3, 16, 80], [0.12, 6, 38, 170],
  [0.28, 12, 90, 245], [0.46, 0, 205, 245], [0.62, 32, 210, 85],
  [0.76, 245, 224, 30], [0.87, 255, 120, 18], [0.96, 240, 32, 24],
  [1, 255, 255, 255],
];
export function referenceColor(t: number, palette = 'reference'): number[] {
  const stops = palette === 'reference-dark' ? REFERENCE_DARK_STOPS : REFERENCE_STOPS;
  const v = Math.max(0, Math.min(1, Number.isFinite(t) ? t : 0));
  for (let i = 1; i < stops.length; i++) {
    const a = stops[i - 1]!, b = stops[i]!;
    if (v <= b[0]!) return [1, 2, 3].map(c => Math.round(a[c]! + (b[c]! - a[c]!) * (v - a[0]!) / (b[0]! - a[0]!)));
  }
  return [255, 255, 255];
}

/** One-shot visual range fit. Never changes samples or continuously follows RF. */
export function fitTerrainRange(bins: Float32Array): Pick<TerrainSettings, 'floor' | 'ceiling' | 'gamma'> | null {
  const levels = Array.from(bins).filter(v => Number.isFinite(v) && v > -999).sort((a, b) => a - b);
  if (!levels.length) return null;
  // Place typical background below the blue/cyan transition. A minimum 45 dB
  // range prevents quiet bands from becoming high-contrast noise. Robust upper
  // percentile leaves isolated strong carriers hot without letting one spike
  // compress every weak/moderate signal into navy.
  const percentile = (q: number) => levels[Math.floor((levels.length - 1) * q)]!;
  const floor = Math.max(-200, Math.min(-25, Math.floor(percentile(0.2) - 6)));
  const ceiling = Math.min(20, Math.max(floor + 45, Math.ceil(percentile(0.995) + 8)));
  return { floor, ceiling, gamma: 0.85 };
}
