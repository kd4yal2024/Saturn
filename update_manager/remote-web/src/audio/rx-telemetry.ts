export type RxJitterPercentiles = {
  p50Ms: number;
  p95Ms: number;
  p99Ms: number;
  sampleCount: number;
};

function finiteNonNegative(value: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

export function percentileMs(values: readonly number[], percentile: number): number {
  const sorted = values
    .map(finiteNonNegative)
    .filter(Number.isFinite)
    .sort((a, b) => a - b);
  if (sorted.length === 0) return 0;
  const p = Math.max(0, Math.min(100, Number.isFinite(percentile) ? percentile : 0));
  const index = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, Math.min(sorted.length - 1, index))] ?? 0;
}

/**
 * Difference between observed packet spacing and the duration represented by
 * the preceding audio packet. This measures arrival jitter without requiring
 * synchronized browser and bridge clocks.
 */
export function rxAudioArrivalJitterMs(
  arrivalAtMs: number,
  previousArrivalAtMs: number,
  expectedPacketDurationMs: number,
): number | null {
  if (
    !Number.isFinite(arrivalAtMs) ||
    !Number.isFinite(previousArrivalAtMs) ||
    previousArrivalAtMs <= 0 ||
    arrivalAtMs < previousArrivalAtMs ||
    !Number.isFinite(expectedPacketDurationMs) ||
    expectedPacketDurationMs <= 0
  ) {
    return null;
  }
  return Math.abs((arrivalAtMs - previousArrivalAtMs) - expectedPacketDurationMs);
}

export function summarizeRxAudioJitter(values: readonly number[]): RxJitterPercentiles {
  return {
    p50Ms: percentileMs(values, 50),
    p95Ms: percentileMs(values, 95),
    p99Ms: percentileMs(values, 99),
    sampleCount: values.length,
  };
}

export function audioFramesToMilliseconds(frames: number, sampleRate: number): number {
  if (!Number.isFinite(frames) || frames <= 0 || !Number.isFinite(sampleRate) || sampleRate <= 0) {
    return 0;
  }
  return (frames / sampleRate) * 1000;
}
