export function resampleChannel(
  input: Float32Array,
  sourceRate: number,
  targetRate: number,
  outputFrames: number,
): Float32Array {
  const output = new Float32Array(outputFrames);
  if (!input.length || outputFrames < 1) return output;
  if (input.length === 1) {
    output.fill(input[0] ?? 0);
    return output;
  }
  for (let i = 0; i < outputFrames; i += 1) {
    const src = (i * sourceRate) / targetRate;
    const lo = Math.min(input.length - 1, Math.floor(src));
    const hi = Math.min(input.length - 1, lo + 1);
    const loSample = input[lo] ?? 0;
    const hiSample = input[hi] ?? loSample;
    output[i] = loSample + (hiSample - loSample) * (src - lo);
  }
  return output;
}

export type PreparedAudio = {
  left: Float32Array;
  right: Float32Array;
  frames: number;
  sampleRate: number;
};

export function prepareAudioForPlayback(
  left: Float32Array,
  right: Float32Array,
  sourceFrames: number,
  sourceRate: number,
  targetRate: number,
): PreparedAudio {
  if (Math.abs(sourceRate - targetRate) < 1) {
    return { left, right, frames: sourceFrames, sampleRate: sourceRate };
  }
  const outputFrames = Math.max(1, Math.round((sourceFrames * targetRate) / sourceRate));
  return {
    left: resampleChannel(left, sourceRate, targetRate, outputFrames),
    right: resampleChannel(right, sourceRate, targetRate, outputFrames),
    frames: outputFrames,
    sampleRate: targetRate,
  };
}

export function ringBufferFreeFrames(
  readIdx: number,
  writeIdx: number,
  capacity: number,
): number {
  const used = writeIdx >= readIdx ? writeIdx - readIdx : capacity - readIdx + writeIdx;
  return capacity - used - 1;
}
