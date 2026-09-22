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
/** Lower-waterfall-only, pointwise background suppression with a broad 24 dB shoulder.
 * The numerical sample is unchanged; weak peaks above the held floor retain
 * contrast, and strong levels rejoin the unshaped scale without a sharp knee.
 */
export function waterfallCleanupNormalized(db: number, floor: number, ceiling: number, baseline: number, strength: number): number {
  const n = Math.max(0, Math.min(1, (db - floor) / (ceiling - floor)));
  const amount = Math.max(0, Math.min(1, strength));
  if (amount === 0 || baseline <= floor || baseline + 8 >= ceiling) return n;
  const end = Math.min(baseline + 24, ceiling);
  const attenuation = 1 - 0.9 * amount;
  if (db <= baseline) return n * attenuation;
  if (db >= end) return n;
  const anchor = (baseline - floor) / (ceiling - floor);
  const endLevel = (end - floor) / (ceiling - floor);
  return anchor * attenuation + (endLevel - anchor * attenuation) * (db - baseline) / (end - baseline);
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
export const WATERFALL_CLEANUP_GLSL = `
uniform float waterfallCleanupStrength;
float waterfallNormalized(float db) {
  float n = clamp((db-floorDb)/(ceilingDb-floorDb),0.0,1.0);
  float amount = clamp(waterfallCleanupStrength,0.0,1.0);
  if(amount <= 0.0 || noiseFloor <= floorDb || noiseFloor+8.0 >= ceilingDb) return n;
  float endDb = min(noiseFloor+24.0,ceilingDb);
  float attenuation = 1.0-0.9*amount;
  if(db <= noiseFloor) return n*attenuation;
  if(db >= endDb) return n;
  float anchor = (noiseFloor-floorDb)/(ceilingDb-floorDb);
  float endLevel = (endDb-floorDb)/(ceilingDb-floorDb);
  return mix(anchor*attenuation,endLevel,(db-noiseFloor)/(endDb-noiseFloor));
}
`;
