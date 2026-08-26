export interface AudioScopeSnapshot {
  left: Float32Array;
  right: Float32Array;
  leftPeak: number;
  rightPeak: number;
  leftPeakDbfs: number;
  rightPeakDbfs: number;
}

function finiteSample(value: number | undefined): number {
  return Number.isFinite(value) ? Math.max(-1, Math.min(1, Number(value))) : 0;
}

function peakDbfs(peak: number): number {
  return peak > 0 ? Math.max(-90, 20 * Math.log10(peak)) : -90;
}

/**
 * Build a small display-only waveform snapshot from decoded stereo audio.
 * Bucket averaging prevents the operations scope from retaining full packets,
 * while peak readings are calculated from all samples for reliable clip cues.
 */
export function buildAudioScopeSnapshot(
  leftInput: Float32Array,
  rightInput: Float32Array,
  pointCount = 256,
): AudioScopeSnapshot {
  const inputLength = Math.max(leftInput?.length ?? 0, rightInput?.length ?? 0);
  const outputLength = Math.max(8, Math.min(1024, Math.round(Number(pointCount) || 256)));
  const left = new Float32Array(outputLength);
  const right = new Float32Array(outputLength);
  let leftPeak = 0;
  let rightPeak = 0;

  for (let index = 0; index < inputLength; index += 1) {
    leftPeak = Math.max(leftPeak, Math.abs(finiteSample(leftInput?.[index])));
    rightPeak = Math.max(rightPeak, Math.abs(finiteSample(rightInput?.[index])));
  }

  for (let point = 0; point < outputLength; point += 1) {
    const start = Math.floor((point * inputLength) / outputLength);
    const end = Math.max(start + 1, Math.floor(((point + 1) * inputLength) / outputLength));
    let leftSum = 0;
    let rightSum = 0;
    let samples = 0;
    for (let index = start; index < Math.min(inputLength, end); index += 1) {
      leftSum += finiteSample(leftInput?.[index]);
      rightSum += finiteSample(rightInput?.[index]);
      samples += 1;
    }
    if (samples > 0) {
      left[point] = leftSum / samples;
      right[point] = rightSum / samples;
    }
  }

  return {
    left,
    right,
    leftPeak,
    rightPeak,
    leftPeakDbfs: peakDbfs(leftPeak),
    rightPeakDbfs: peakDbfs(rightPeak),
  };
}
