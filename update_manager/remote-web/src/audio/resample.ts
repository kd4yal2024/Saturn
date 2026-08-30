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

export type AdaptivePlaybackRateOptions = {
  enabled?: boolean;
  targetQueueMs?: number;
  proportionalGainPerMs?: number;
  maxCorrectionRatio?: number;
  slewPerSecond?: number;
};

export type AdaptivePlaybackRateTelemetry = {
  enabled: boolean;
  ratio: number;
  correctionPpm: number;
  correctionCount: number;
  targetQueueMs: number;
  lastQueueMs: number | null;
};

const DEFAULT_TARGET_QUEUE_MS = 42;
const DEFAULT_PROPORTIONAL_GAIN_PER_MS = 0.000025;
const DEFAULT_MAX_CORRECTION_RATIO = 0.002;
const DEFAULT_SLEW_PER_SECOND = 0.0005;

function finiteOr(value: number | undefined, fallback: number): number {
  return Number.isFinite(value) ? Number(value) : fallback;
}

function clamp(value: number, low: number, high: number): number {
  return Math.max(low, Math.min(high, value));
}

/**
 * Occupancy servo for the independent bridge and browser playback clocks.
 *
 * A queue below the target produces slightly more browser-rate frames; a
 * queue above target produces slightly fewer. Fractional output frames are
 * carried between packets so ppm-scale corrections are not lost to rounding.
 */
export class AdaptivePlaybackRateController {
  private readonly targetQueueMs: number;
  private readonly proportionalGainPerMs: number;
  private readonly maxCorrectionRatio: number;
  private readonly slewPerSecond: number;
  private enabled: boolean;
  private ratio = 1;
  private frameRemainder = 0;
  private correctionCount = 0;
  private lastQueueMs: number | null = null;
  private lastObservationMs: number | null = null;

  constructor(options: AdaptivePlaybackRateOptions = {}) {
    this.enabled = options.enabled !== false;
    this.targetQueueMs = Math.max(
      1,
      finiteOr(options.targetQueueMs, DEFAULT_TARGET_QUEUE_MS),
    );
    this.proportionalGainPerMs = Math.max(
      0,
      finiteOr(options.proportionalGainPerMs, DEFAULT_PROPORTIONAL_GAIN_PER_MS),
    );
    this.maxCorrectionRatio = clamp(
      finiteOr(options.maxCorrectionRatio, DEFAULT_MAX_CORRECTION_RATIO),
      0,
      0.02,
    );
    this.slewPerSecond = clamp(
      finiteOr(options.slewPerSecond, DEFAULT_SLEW_PER_SECOND),
      0,
      0.02,
    );
  }

  reset(): void {
    this.ratio = 1;
    this.frameRemainder = 0;
    this.correctionCount = 0;
    this.lastQueueMs = null;
    this.lastObservationMs = null;
  }

  observeQueue(queuedMs: number | null, nowMs: number): AdaptivePlaybackRateTelemetry {
    if (!this.enabled || queuedMs == null || !Number.isFinite(queuedMs)) {
      return this.telemetry();
    }
    const queue = Math.max(0, queuedMs);
    if (!Number.isFinite(nowMs)) return this.telemetry();
    if (this.lastObservationMs != null && nowMs <= this.lastObservationMs) {
      return this.telemetry();
    }

    const elapsedSeconds = this.lastObservationMs == null
      ? 0
      : Math.min(1, Math.max(0, (nowMs - this.lastObservationMs) / 1000));
    this.lastObservationMs = nowMs;
    this.lastQueueMs = queue;

    const queueErrorMs = this.targetQueueMs - queue;
    const desiredRatio = 1 + clamp(
      queueErrorMs * this.proportionalGainPerMs,
      -this.maxCorrectionRatio,
      this.maxCorrectionRatio,
    );
    const maxStep = this.slewPerSecond * elapsedSeconds;
    const nextRatio = elapsedSeconds === 0
      ? this.ratio
      : this.ratio + clamp(desiredRatio - this.ratio, -maxStep, maxStep);
    if (Math.abs(nextRatio - this.ratio) > Number.EPSILON) {
      this.correctionCount += 1;
      this.ratio = nextRatio;
    }
    return this.telemetry();
  }

  prepare(
    left: Float32Array,
    right: Float32Array,
    sourceFrames: number,
    sourceRate: number,
    targetRate: number,
  ): PreparedAudio {
    if (!this.enabled) {
      return prepareAudioForPlayback(left, right, sourceFrames, sourceRate, targetRate);
    }
    const exactFrames =
      (Math.max(0, sourceFrames) * Math.max(1, targetRate) * this.ratio) /
        Math.max(1, sourceRate) +
      this.frameRemainder;
    const outputFrames = Math.max(1, Math.floor(exactFrames));
    this.frameRemainder = exactFrames - outputFrames;
    return prepareAudioForPlayback(
      left,
      right,
      sourceFrames,
      sourceRate,
      targetRate,
      outputFrames,
    );
  }

  telemetry(): AdaptivePlaybackRateTelemetry {
    return {
      enabled: this.enabled,
      ratio: this.ratio,
      correctionPpm: (this.ratio - 1) * 1_000_000,
      correctionCount: this.correctionCount,
      targetQueueMs: this.targetQueueMs,
      lastQueueMs: this.lastQueueMs,
    };
  }
}

export function prepareAudioForPlayback(
  left: Float32Array,
  right: Float32Array,
  sourceFrames: number,
  sourceRate: number,
  targetRate: number,
  outputFramesOverride?: number,
): PreparedAudio {
  const outputFrames = outputFramesOverride == null
    ? Math.max(1, Math.round((sourceFrames * targetRate) / sourceRate))
    : Math.max(1, Math.round(outputFramesOverride));
  if (Math.abs(sourceRate - targetRate) < 1 && outputFrames === sourceFrames) {
    return { left, right, frames: sourceFrames, sampleRate: sourceRate };
  }
  const effectiveTargetRate = sourceFrames > 0
    ? (sourceRate * outputFrames) / sourceFrames
    : targetRate;
  return {
    left: resampleChannel(left, sourceRate, effectiveTargetRate, outputFrames),
    right: resampleChannel(right, sourceRate, effectiveTargetRate, outputFrames),
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
