/** Display-only monotone soft knee. No source samples, neighbors or time averages. */
export function estimateNoiseFloor(bins: Float32Array): number | null {
  const values = Array.from(bins).filter(v => Number.isFinite(v) && v > -999).sort((a,b)=>a-b);
  return values.length ? values[Math.floor((values.length-1)/2)]! : null;
}
export function cleanupNormalized(db: number, floor: number, ceiling: number, baseline: number, strength: number): number {
  const n = Math.max(0, Math.min(1, (db-floor)/(ceiling-floor)));
  const end = Math.min(baseline+8, ceiling);
  const start = end-4;
  const x = Math.max(0, Math.min(1, (db-start)/4));
  const shoulder = x*x*(3-2*x);
  return n * (1-Math.max(0,Math.min(.85,strength))*(1-shoulder));
}
// Same pointwise mapping in vertex height and fragment color. Strictly increasing
// within the selected range; baseline+8 dB and above are entirely unchanged.
export const CLEANUP_GLSL = `
uniform float noiseFloor, cleanupStrength;
float displayNormalized(float db) {
  float n = clamp((db-floorDb)/(ceilingDb-floorDb),0.0,1.0);
  float endDb = min(noiseFloor+8.0,ceilingDb);
  float x = clamp((db-(endDb-4.0))/4.0,0.0,1.0);
  return n*(1.0-cleanupStrength*(1.0-x*x*(3.0-2.0*x)));
}
`;
