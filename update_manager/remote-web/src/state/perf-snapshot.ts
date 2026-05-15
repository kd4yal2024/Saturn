export interface PerfSnapshotSource {
  connected: boolean;
  connectPending: boolean;
  iqStreaming: boolean;
  audioStreaming: boolean;
  sampleRate: number;
  audioSampleRate: number;
  displayZoom: number;
  frameRate: number;
  frameCounter: number;
  iqPackets: { length: number };
  fftWidth: number;
  audioFramesPlayed: number;
  audioBackpressureDrops: number;
  rxWorkletDrops: number;
  bridgeRttMs: number | null;
  backpressureSafetyP50Us: number;
  backpressureSafetyP95Us: number;
  backpressureSafetyP99Us: number;
  backpressureControlP50Us: number;
  backpressureControlP95Us: number;
  backpressureControlP99Us: number;
  displayReplacedPerSec: number;
  displayDroppedPerSec: number;
  bridgeAudioDroppedPerSec: number;
  bridgeAudioSeqGapCount: number;
  audioSeqGapCount: number;
  audioPanicDrainCount: number;
  sendBlockedMs: number;
  outboundHighWatermarkBytes: number;
  safetyQueueDepthOverflowCount: number;
  lastFrameAt: number;
  lastSpectrumRenderAt: number;
  waterfallSettleFrames: number;
  displayCaption: string;
}

export interface PerfSnapshot {
  wallTime: string;
  nowMs: number;
  connected: boolean;
  connectPending: boolean;
  iqStreaming: boolean;
  audioStreaming: boolean;
  sampleRate: number;
  audioSampleRate: number;
  displayZoom: number;
  frameRate: number;
  frameCounter: number;
  iqPackets: number;
  fftWidth: number;
  audioFramesPlayed: number;
  audioBackpressureDrops: number;
  rxWorkletDrops: number;
  bridgeRttMs: number | null;
  backpressureSafetyP50Us: number;
  backpressureSafetyP95Us: number;
  backpressureSafetyP99Us: number;
  backpressureControlP50Us: number;
  backpressureControlP95Us: number;
  backpressureControlP99Us: number;
  displayReplacedPerSec: number;
  displayDroppedPerSec: number;
  bridgeAudioDroppedPerSec: number;
  bridgeAudioSeqGapCount: number;
  audioSeqGapCount: number;
  audioPanicDrainCount: number;
  sendBlockedMs: number;
  outboundHighWatermarkBytes: number;
  safetyQueueDepthOverflowCount: number;
  iqIdleMs: number;
  spectrumLatencyMs: number;
  waterfallSettleFrames: number;
  displayCaption: string;
}

export interface PerfSummary {
  sampleCount: number;
  startedAt: string | null;
  endedAt: string | null;
  avgFrameRate: number;
  minFrameRate: number;
  maxFrameRate: number;
  maxIqIdleMs: number;
  maxBridgeRttMs: number;
  maxBackpressureSafetyP99Us: number;
  maxBackpressureControlP99Us: number;
  totalDisplayReplaced: number;
  totalDisplayDropped: number;
  totalBridgeAudioDropped: number;
  finalBridgeAudioSeqGapCount: number;
  finalAudioSeqGapCount: number;
  totalAudioPanicDrainCount: number;
  totalSendBlockedMs: number;
  maxOutboundHighWatermarkBytes: number;
  totalSafetyQueueDepthOverflowCount: number;
  finalAudioBackpressureDrops: number;
  finalRxWorkletDrops: number;
  finalSnapshot: PerfSnapshot;
}

function finiteNumber(value: unknown, fallback = 0): number {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

function nonNegativeInteger(value: unknown): number {
  return Math.max(0, Math.round(finiteNumber(value, 0)));
}

export function buildPerfSnapshot(s: PerfSnapshotSource, nowMs: number): PerfSnapshot {
  return {
    wallTime: new Date().toISOString(),
    nowMs: Math.round(nowMs),
    connected: Boolean(s.connected),
    connectPending: Boolean(s.connectPending),
    iqStreaming: Boolean(s.iqStreaming),
    audioStreaming: Boolean(s.audioStreaming),
    sampleRate: Math.round(Number(s.sampleRate) || 0),
    audioSampleRate: Math.round(Number(s.audioSampleRate) || 0),
    displayZoom: Number(s.displayZoom) || 0,
    frameRate: Number((Number(s.frameRate) || 0).toFixed(1)),
    frameCounter: Math.round(Number(s.frameCounter) || 0),
    iqPackets: s.iqPackets ? s.iqPackets.length : 0,
    fftWidth: Math.round(Number(s.fftWidth) || 0),
    audioFramesPlayed: Math.round(Number(s.audioFramesPlayed) || 0),
    audioBackpressureDrops: Math.round(Number(s.audioBackpressureDrops) || 0),
    rxWorkletDrops: Math.round(Number(s.rxWorkletDrops) || 0),
    bridgeRttMs: s.bridgeRttMs == null ? null : nonNegativeInteger(s.bridgeRttMs),
    backpressureSafetyP50Us: nonNegativeInteger(s.backpressureSafetyP50Us),
    backpressureSafetyP95Us: nonNegativeInteger(s.backpressureSafetyP95Us),
    backpressureSafetyP99Us: nonNegativeInteger(s.backpressureSafetyP99Us),
    backpressureControlP50Us: nonNegativeInteger(s.backpressureControlP50Us),
    backpressureControlP95Us: nonNegativeInteger(s.backpressureControlP95Us),
    backpressureControlP99Us: nonNegativeInteger(s.backpressureControlP99Us),
    displayReplacedPerSec: nonNegativeInteger(s.displayReplacedPerSec),
    displayDroppedPerSec: nonNegativeInteger(s.displayDroppedPerSec),
    bridgeAudioDroppedPerSec: nonNegativeInteger(s.bridgeAudioDroppedPerSec),
    bridgeAudioSeqGapCount: nonNegativeInteger(s.bridgeAudioSeqGapCount),
    audioSeqGapCount: nonNegativeInteger(s.audioSeqGapCount),
    audioPanicDrainCount: nonNegativeInteger(s.audioPanicDrainCount),
    sendBlockedMs: nonNegativeInteger(s.sendBlockedMs),
    outboundHighWatermarkBytes: nonNegativeInteger(s.outboundHighWatermarkBytes),
    safetyQueueDepthOverflowCount: nonNegativeInteger(s.safetyQueueDepthOverflowCount),
    iqIdleMs: Math.max(0, Math.round(nowMs - (Number(s.lastFrameAt) || 0))),
    spectrumLatencyMs: Math.max(0, Math.round(nowMs - (Number(s.lastSpectrumRenderAt) || 0))),
    waterfallSettleFrames: Math.round(Number(s.waterfallSettleFrames) || 0),
    displayCaption: `${s.displayCaption || ''}`,
  };
}

export function buildPerfSummary(snapshots: PerfSnapshot[], currentSnapshot: PerfSnapshot): PerfSummary {
  const samples = snapshots.slice();
  const frameRates = samples.map((s) => Number(s.frameRate) || 0);
  const iqIdle = samples.map((s) => Number(s.iqIdleMs) || 0);
  const bridgeRtt = samples
    .map((s) => (s.bridgeRttMs == null ? null : Number(s.bridgeRttMs)))
    .filter((value): value is number => value != null && Number.isFinite(value));
  const safetyP99 = samples.map((s) => Number(s.backpressureSafetyP99Us) || 0);
  const controlP99 = samples.map((s) => Number(s.backpressureControlP99Us) || 0);
  const backpressure = samples.map((s) => Number(s.audioBackpressureDrops) || 0);
  const workletDrops = samples.map((s) => Number(s.rxWorkletDrops) || 0);
  const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);
  const average = (values: number[]) =>
    values.length ? values.reduce((sum, v) => sum + v, 0) / values.length : 0;
  const finalAudioBackpressureDrops = backpressure.length
    ? backpressure[backpressure.length - 1] ?? 0
    : 0;
  const finalRxWorkletDrops = workletDrops.length
    ? workletDrops[workletDrops.length - 1] ?? 0
    : 0;
  return {
    sampleCount: samples.length,
    startedAt: samples[0]?.wallTime || null,
    endedAt: samples[samples.length - 1]?.wallTime || null,
    avgFrameRate: Number(average(frameRates).toFixed(2)),
    minFrameRate: frameRates.length ? Math.min(...frameRates) : 0,
    maxFrameRate: frameRates.length ? Math.max(...frameRates) : 0,
    maxIqIdleMs: iqIdle.length ? Math.max(...iqIdle) : 0,
    maxBridgeRttMs: bridgeRtt.length ? Math.max(...bridgeRtt) : 0,
    maxBackpressureSafetyP99Us: safetyP99.length ? Math.max(...safetyP99) : 0,
    maxBackpressureControlP99Us: controlP99.length ? Math.max(...controlP99) : 0,
    totalDisplayReplaced: sum(samples.map((s) => Number(s.displayReplacedPerSec) || 0)),
    totalDisplayDropped: sum(samples.map((s) => Number(s.displayDroppedPerSec) || 0)),
    totalBridgeAudioDropped: sum(samples.map((s) => Number(s.bridgeAudioDroppedPerSec) || 0)),
    finalBridgeAudioSeqGapCount: samples[samples.length - 1]?.bridgeAudioSeqGapCount ?? 0,
    finalAudioSeqGapCount: samples[samples.length - 1]?.audioSeqGapCount ?? 0,
    totalAudioPanicDrainCount: sum(samples.map((s) => Number(s.audioPanicDrainCount) || 0)),
    totalSendBlockedMs: sum(samples.map((s) => Number(s.sendBlockedMs) || 0)),
    maxOutboundHighWatermarkBytes: samples.length
      ? Math.max(...samples.map((s) => Number(s.outboundHighWatermarkBytes) || 0))
      : 0,
    totalSafetyQueueDepthOverflowCount: sum(
      samples.map((s) => Number(s.safetyQueueDepthOverflowCount) || 0),
    ),
    finalAudioBackpressureDrops,
    finalRxWorkletDrops,
    finalSnapshot: samples[samples.length - 1] || currentSnapshot,
  };
}
