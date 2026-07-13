export type RxJitterPercentiles = {
  p50Ms: number;
  p95Ms: number;
  p99Ms: number;
  sampleCount: number;
};

export type RxLatencyDiagnostic = {
  capturedAtIso: string;
  connection: string;
  bridgeUrl: string;
  networkProfile: string;
  audioProfile: string;
  bridgeRttMs: number | null;
  jitterP50Ms: number;
  jitterP95Ms: number;
  jitterP99Ms: number;
  jitterSampleCount: number;
  audioQueueMs: number | null;
  workletUnderruns: number;
  workletOverflows: number;
  audioDropEvents: number;
  audioContextBaseLatencyMs: number | null;
  audioContextOutputLatencyMs: number | null;
  audioFrameAgeMs: number | null;
  iqFrameAgeMs: number | null;
  bridgeBacklogBytes: number;
  browserBacklogBytes: number;
  bridgeHighWaterBytes: number;
  tcpHighWaterBytes: number;
  connectionRecoveryMs: number | null;
  connectionLossCount: number;
  audioSequenceGaps: number;
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

function diagnosticNumber(value: number, digits = 1): string {
  return finiteNonNegative(value).toFixed(digits);
}

function diagnosticMilliseconds(value: number | null): string {
  return value == null || !Number.isFinite(value) ? 'unavailable' : `${diagnosticNumber(value)} ms`;
}

function diagnosticBytes(value: number): string {
  return `${Math.round(finiteNonNegative(value))} B`;
}

export function formatRxLatencyDiagnostic(value: RxLatencyDiagnostic): string {
  return [
    'Saturn Remote RX Latency',
    `Captured: ${value.capturedAtIso || 'unknown'}`,
    `Connection: ${value.connection || 'unknown'}`,
    `Bridge: ${value.bridgeUrl || 'default'}`,
    `Profiles: ${value.networkProfile || 'unknown'} / ${value.audioProfile || 'unknown'}`,
    `Bridge RTT: ${diagnosticMilliseconds(value.bridgeRttMs)}`,
    `Packet jitter p50/p95/p99: ${diagnosticNumber(value.jitterP50Ms)} / ${diagnosticNumber(value.jitterP95Ms)} / ${diagnosticNumber(value.jitterP99Ms)} ms (${Math.max(0, Math.round(value.jitterSampleCount || 0))} samples)`,
    `Audio queue: ${diagnosticMilliseconds(value.audioQueueMs)}`,
    `Worklet underruns/overflows: ${Math.max(0, Math.round(value.workletUnderruns || 0))} / ${Math.max(0, Math.round(value.workletOverflows || 0))}`,
    `Audio drop/resync events: ${Math.max(0, Math.round(value.audioDropEvents || 0))}`,
    `AudioContext base/output: ${diagnosticMilliseconds(value.audioContextBaseLatencyMs)} / ${diagnosticMilliseconds(value.audioContextOutputLatencyMs)}`,
    `Frame age audio/IQ: ${diagnosticMilliseconds(value.audioFrameAgeMs)} / ${diagnosticMilliseconds(value.iqFrameAgeMs)}`,
    `Media backlog bridge/browser: ${diagnosticBytes(value.bridgeBacklogBytes)} / ${diagnosticBytes(value.browserBacklogBytes)}`,
    `Backlog high-water bridge/TCP: ${diagnosticBytes(value.bridgeHighWaterBytes)} / ${diagnosticBytes(value.tcpHighWaterBytes)}`,
    `Recovery/losses: ${diagnosticMilliseconds(value.connectionRecoveryMs)} / ${Math.max(0, Math.round(value.connectionLossCount || 0))}`,
    `Audio sequence gaps: ${Math.max(0, Math.round(value.audioSequenceGaps || 0))}`,
  ].join('\n');
}
