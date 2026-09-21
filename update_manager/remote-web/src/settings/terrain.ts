/** Local presentation only. Never included in radio command construction. */
export type TerrainSettings = {
  mode: 'traditional' | '3d'; height: number; depth: number; elevation: number;
  floor: number; ceiling: number; gamma: number; smoothing: number;
  palette: string; split: number; quality: 'auto' | 'performance' | 'balanced' | 'high';
  diagnostics: boolean;
};
export const TERRAIN_DEFAULTS: TerrainSettings = {
  mode: 'traditional', height: 0.65, depth: 128, elevation: 35,
  floor: -140, ceiling: -40, gamma: 1, smoothing: 0,
  palette: 'reference', split: 0.3, quality: 'auto', diagnostics: false,
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
    palette: ['reference', 'classic', 'ember', 'ice', 'forest', 'enhanced'].includes(String(s.palette)) ? String(s.palette) : 'reference',
    quality: s.quality === 'performance' || s.quality === 'balanced' || s.quality === 'high' ? s.quality : 'auto',
    diagnostics: s.diagnostics === true,
  };
}
export const REFERENCE_STOPS = [
  [0, 1, 3, 12], [0.12, 3, 10, 48], [0.30, 7, 40, 168],
  [0.48, 0, 190, 230], [0.62, 30, 205, 70], [0.76, 245, 220, 25],
  [0.87, 255, 110, 10], [0.96, 240, 25, 20], [1, 255, 255, 255],
];
export function referenceColor(t: number): number[] {
  const v = Math.max(0, Math.min(1, Number.isFinite(t) ? t : 0));
  for (let i = 1; i < REFERENCE_STOPS.length; i++) {
    const a = REFERENCE_STOPS[i - 1]!, b = REFERENCE_STOPS[i]!;
    if (v <= b[0]!) return [1, 2, 3].map(c => Math.round(a[c]! + (b[c]! - a[c]!) * (v - a[0]!) / (b[0]! - a[0]!)));
  }
  return [255, 255, 255];
}
