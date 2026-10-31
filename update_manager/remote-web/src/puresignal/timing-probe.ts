/** Finite stimulus preparation only. No microphone, socket, RF or PS control.
 * Irregularly spaced multisine avoids the 700/1900 Hz beat-period ambiguity.
 * A finite record is normalized BEFORE use, never while samples are streaming.
 */
export const TIMING_PROBE_VERSION = 'saturn-timing-multisine-v1';
export const TIMING_PROBE_SEED = 0x53465031;
export function createTimingProbePcm(durationMs = 1000) {
  if (!Number.isSafeInteger(durationMs) || durationMs < 100 || durationMs > 45000)
    throw new Error('Timing fixture duration must be 100..45000 ms');
  const rateHz = 48000, count = durationMs * 48, frequencies: number[] = [], phases: number[] = [];
  let random = TIMING_PROBE_SEED;
  const uniform = () => { random = (Math.imul(random, 1664525) + 1013904223) >>> 0; return random / 4294967296; };
  for (let k = 0; k < 32; k++) {
    frequencies.push(400 + (k + .1 + .8 * uniform()) * (2200 / 32));
    phases.push(2 * Math.PI * uniform());
  }
  const prepared = new Float64Array(count);
  const fade = 240; // 5ms raised-cosine edges, not abrupt PTT/signal steps.
  let peak = 0;
  for (let n = 0; n < count; n++) {
    let value = 0;
    for (let k = 0; k < frequencies.length; k++) value += Math.sin(2 * Math.PI * frequencies[k]! * n / rateHz + phases[k]!);
    const edge = Math.min(n, count-1-n);
    if (edge < fade) value *= .5 - .5 * Math.cos(Math.PI * edge / fade);
    prepared[n] = value; peak = Math.max(peak, Math.abs(value));
  }
  if (!(peak > 0)) throw new Error('Silent timing fixture');
  // Leave float32 rounding room below the same .4 ceiling as the two-tone.
  const samples = Float32Array.from(prepared, v => v * .399999 / peak);
  const rms = Math.sqrt(samples.reduce((sum, v) => sum + v*v, 0)/samples.length);
  return {samples, rateHz, durationMs, version:TIMING_PROBE_VERSION, seed:TIMING_PROBE_SEED,
    frequenciesHz:frequencies, phasesRadians:phases,
    componentPeak:samples.reduce((p,v)=>Math.max(p,Math.abs(v)),0), rms,
    intendedUse:'Timing diagnostic; MeasurementOnly repeats a prepared 1000 ms record; not two-tone IMD or calibration'};
}
