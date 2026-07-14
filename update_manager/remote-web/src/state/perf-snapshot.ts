export interface PerfSnapshotSource {
  connected: boolean;
  connectPending: boolean;
  iqStreaming: boolean;
  audioStreaming: boolean;
  sampleRate: number;
  audioSampleRate: number;
  audioChannels: number;
  streamMode: string;
  audioWorkletMode: string | null;
  displayZoom: number;
  frameRate: number;
  frameCounter: number;
  iqPackets: { length: number };
  fftWidth: number;
  audioFramesPlayed: number;
  audioBackpressureDrops: number;
  rxWorkletDrops: number;
  rxWorkletQueuedMs: number;
  rxWorkletUnderruns: number;
  rxWorkletOverflows: number;
  audioContextBaseLatencyMs: number | null;
  audioContextOutputLatencyMs: number | null;
  rxAudioJitterSamples: { length: number };
  rxAudioJitterP50Ms: number;
  rxAudioJitterP95Ms: number;
  rxAudioJitterP99Ms: number;
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
  bridgeOutboundQueuedBytes: number;
  bridgeTcpOutqHighWatermarkBytes: number;
  displayRateLimitedPerSec: number;
  wsMediaBacklogBytes: number;
  safetyQueueDepthOverflowCount: number;
  lastFrameAt: number;
  lastAudioFrameAt: number;
  connectionLossStartedAt: number;
  connectionLossCount: number;
  connectionRecoveryMs: number | null;
  connectionOpenMs: number | null;
  lastSpectrumRenderAt: number;
  waterfallSettleFrames: number;
  displayCaption: string;
  displayProfile?: string;
  browserMainLagP95Ms: number;
  browserMainLagP99Ms: number;
  browserMainLagMaxMs: number;
  browserRafIntervalP95Ms: number;
  browserRafIntervalP99Ms: number;
  browserRafIntervalMaxMs: number;
  txWorkletToMainP95Ms: number;
  txWorkletToMainP99Ms: number;
  txWorkletToMainMaxMs: number;
  txMainToSendP95Ms: number;
  txMainToSendP99Ms: number;
  txMainToSendMaxMs: number;
  txWsSendP95Ms: number;
  txWsSendP99Ms: number;
  txWsSendMaxMs: number;
  txTimingFrameCount: number;
  txTimingDroppedFrameCount: number;
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
  audioChannels: number;
  networkProfile: string;
  audioProfile: string;
  displayZoom: number;
  frameRate: number;
  frameCounter: number;
  iqPackets: number;
  fftWidth: number;
  audioFramesPlayed: number;
  audioBackpressureDrops: number;
  rxWorkletDrops: number;
  rxWorkletQueuedMs: number;
  rxWorkletUnderruns: number;
  rxWorkletOverflows: number;
  audioContextBaseLatencyMs: number | null;
  audioContextOutputLatencyMs: number | null;
  rxAudioJitterSampleCount: number;
  rxAudioJitterP50Ms: number;
  rxAudioJitterP95Ms: number;
  rxAudioJitterP99Ms: number;
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
  bridgeOutboundQueuedBytes: number;
  bridgeTcpOutqHighWatermarkBytes: number;
  displayRateLimitedPerSec: number;
  wsMediaBacklogBytes: number;
  safetyQueueDepthOverflowCount: number;
  iqIdleMs: number;
  audioIdleMs: number;
  connectionLossCount: number;
  connectionDownMs: number;
  connectionRecoveryMs: number | null;
  connectionOpenMs: number | null;
  spectrumLatencyMs: number;
  waterfallSettleFrames: number;
  displayCaption: string;
  displayProfile: string;
  browserMainLagP95Ms: number;
  browserMainLagP99Ms: number;
  browserMainLagMaxMs: number;
  browserRafIntervalP95Ms: number;
  browserRafIntervalP99Ms: number;
  browserRafIntervalMaxMs: number;
  txWorkletToMainP95Ms: number;
  txWorkletToMainP99Ms: number;
  txWorkletToMainMaxMs: number;
  txMainToSendP95Ms: number;
  txMainToSendP99Ms: number;
  txMainToSendMaxMs: number;
  txWsSendP95Ms: number;
  txWsSendP99Ms: number;
  txWsSendMaxMs: number;
  txTimingFrameCount: number;
  txTimingDroppedFrameCount: number;
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
  finalRxWorkletUnderruns: number;
  finalRxWorkletOverflows: number;
  maxRxWorkletQueuedMs: number;
  maxRxAudioJitterP99Ms: number;
  maxWsMediaBacklogBytes: number;
  maxBridgeOutboundQueuedBytes: number;
  maxConnectionRecoveryMs: number;
  maxBrowserMainLagP99Ms: number;
  maxBrowserRafIntervalP99Ms: number;
  maxTxWorkletToMainP99Ms: number;
  maxTxMainToSendP99Ms: number;
  maxTxWsSendP99Ms: number;
  totalTxTimingFrames: number;
  totalTxTimingDroppedFrames: number;
  displayProfile: string;
  finalSnapshot: PerfSnapshot;
}

function finiteNumber(value: unknown, fallback = 0): number {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

function nonNegativeInteger(value: unknown): number {
  return Math.max(0, Math.round(finiteNumber(value, 0)));
}

function nonNegativeDecimal(value: unknown, digits = 2): number {
  return Number(Math.max(0, finiteNumber(value, 0)).toFixed(digits));
}

function nullableNonNegativeDecimal(value: unknown, digits = 2): number | null {
  return value == null ? null : nonNegativeDecimal(value, digits);
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
    audioChannels: Math.max(1, Math.round(Number(s.audioChannels) || 1)),
    networkProfile: `${s.streamMode || 'lan'}`,
    audioProfile: `PCM F32 ${Math.round((Number(s.audioSampleRate) || 0) / 1000)} kHz ${Math.max(1, Math.round(Number(s.audioChannels) || 1)) === 1 ? 'mono' : 'stereo'} / ${s.audioWorkletMode || 'legacy'}`,
    displayZoom: Number(s.displayZoom) || 0,
    frameRate: Number((Number(s.frameRate) || 0).toFixed(1)),
    frameCounter: Math.round(Number(s.frameCounter) || 0),
    iqPackets: s.iqPackets ? s.iqPackets.length : 0,
    fftWidth: Math.round(Number(s.fftWidth) || 0),
    audioFramesPlayed: Math.round(Number(s.audioFramesPlayed) || 0),
    audioBackpressureDrops: Math.round(Number(s.audioBackpressureDrops) || 0),
    rxWorkletDrops: Math.round(Number(s.rxWorkletDrops) || 0),
    rxWorkletQueuedMs: nonNegativeDecimal(s.rxWorkletQueuedMs),
    rxWorkletUnderruns: nonNegativeInteger(s.rxWorkletUnderruns),
    rxWorkletOverflows: nonNegativeInteger(s.rxWorkletOverflows),
    audioContextBaseLatencyMs: nullableNonNegativeDecimal(s.audioContextBaseLatencyMs),
    audioContextOutputLatencyMs: nullableNonNegativeDecimal(s.audioContextOutputLatencyMs),
    rxAudioJitterSampleCount: nonNegativeInteger(s.rxAudioJitterSamples?.length),
    rxAudioJitterP50Ms: nonNegativeDecimal(s.rxAudioJitterP50Ms),
    rxAudioJitterP95Ms: nonNegativeDecimal(s.rxAudioJitterP95Ms),
    rxAudioJitterP99Ms: nonNegativeDecimal(s.rxAudioJitterP99Ms),
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
    bridgeOutboundQueuedBytes: nonNegativeInteger(s.bridgeOutboundQueuedBytes),
    bridgeTcpOutqHighWatermarkBytes: nonNegativeInteger(s.bridgeTcpOutqHighWatermarkBytes),
    displayRateLimitedPerSec: nonNegativeInteger(s.displayRateLimitedPerSec),
    wsMediaBacklogBytes: nonNegativeInteger(s.wsMediaBacklogBytes),
    safetyQueueDepthOverflowCount: nonNegativeInteger(s.safetyQueueDepthOverflowCount),
    iqIdleMs: Math.max(0, Math.round(nowMs - (Number(s.lastFrameAt) || 0))),
    audioIdleMs: Math.max(0, Math.round(nowMs - (Number(s.lastAudioFrameAt) || 0))),
    connectionLossCount: nonNegativeInteger(s.connectionLossCount),
    connectionDownMs: s.connectionLossStartedAt
      ? Math.max(0, Math.round(nowMs - s.connectionLossStartedAt))
      : 0,
    connectionRecoveryMs: nullableNonNegativeDecimal(s.connectionRecoveryMs),
    connectionOpenMs: nullableNonNegativeDecimal(s.connectionOpenMs),
    spectrumLatencyMs: Math.max(0, Math.round(nowMs - (Number(s.lastSpectrumRenderAt) || 0))),
    waterfallSettleFrames: Math.round(Number(s.waterfallSettleFrames) || 0),
    displayCaption: `${s.displayCaption || ''}`,
    displayProfile: `${s.displayProfile || ''}`,
    browserMainLagP95Ms: nonNegativeDecimal(s.browserMainLagP95Ms),
    browserMainLagP99Ms: nonNegativeDecimal(s.browserMainLagP99Ms),
    browserMainLagMaxMs: nonNegativeDecimal(s.browserMainLagMaxMs),
    browserRafIntervalP95Ms: nonNegativeDecimal(s.browserRafIntervalP95Ms),
    browserRafIntervalP99Ms: nonNegativeDecimal(s.browserRafIntervalP99Ms),
    browserRafIntervalMaxMs: nonNegativeDecimal(s.browserRafIntervalMaxMs),
    txWorkletToMainP95Ms: nonNegativeDecimal(s.txWorkletToMainP95Ms),
    txWorkletToMainP99Ms: nonNegativeDecimal(s.txWorkletToMainP99Ms),
    txWorkletToMainMaxMs: nonNegativeDecimal(s.txWorkletToMainMaxMs),
    txMainToSendP95Ms: nonNegativeDecimal(s.txMainToSendP95Ms),
    txMainToSendP99Ms: nonNegativeDecimal(s.txMainToSendP99Ms),
    txMainToSendMaxMs: nonNegativeDecimal(s.txMainToSendMaxMs),
    txWsSendP95Ms: nonNegativeDecimal(s.txWsSendP95Ms),
    txWsSendP99Ms: nonNegativeDecimal(s.txWsSendP99Ms),
    txWsSendMaxMs: nonNegativeDecimal(s.txWsSendMaxMs),
    txTimingFrameCount: nonNegativeInteger(s.txTimingFrameCount),
    txTimingDroppedFrameCount: nonNegativeInteger(s.txTimingDroppedFrameCount),
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
  const workletQueued = samples.map((s) => Number(s.rxWorkletQueuedMs) || 0);
  const jitterP99 = samples.map((s) => Number(s.rxAudioJitterP99Ms) || 0);
  const wsBacklog = samples.map((s) => Number(s.wsMediaBacklogBytes) || 0);
  const bridgeQueued = samples.map((s) => Number(s.bridgeOutboundQueuedBytes) || 0);
  const recovery = samples.map((s) => Number(s.connectionRecoveryMs) || 0);
  const browserMainLagP99 = samples.map((s) => Number(s.browserMainLagP99Ms) || 0);
  const browserRafP99 = samples.map((s) => Number(s.browserRafIntervalP99Ms) || 0);
  const txWorkletToMainP99 = samples.map((s) => Number(s.txWorkletToMainP99Ms) || 0);
  const txMainToSendP99 = samples.map((s) => Number(s.txMainToSendP99Ms) || 0);
  const txWsSendP99 = samples.map((s) => Number(s.txWsSendP99Ms) || 0);
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
    finalRxWorkletUnderruns: samples[samples.length - 1]?.rxWorkletUnderruns ?? 0,
    finalRxWorkletOverflows: samples[samples.length - 1]?.rxWorkletOverflows ?? 0,
    maxRxWorkletQueuedMs: workletQueued.length ? Math.max(...workletQueued) : 0,
    maxRxAudioJitterP99Ms: jitterP99.length ? Math.max(...jitterP99) : 0,
    maxWsMediaBacklogBytes: wsBacklog.length ? Math.max(...wsBacklog) : 0,
    maxBridgeOutboundQueuedBytes: bridgeQueued.length ? Math.max(...bridgeQueued) : 0,
    maxConnectionRecoveryMs: recovery.length ? Math.max(...recovery) : 0,
    maxBrowserMainLagP99Ms: browserMainLagP99.length ? Math.max(...browserMainLagP99) : 0,
    maxBrowserRafIntervalP99Ms: browserRafP99.length ? Math.max(...browserRafP99) : 0,
    maxTxWorkletToMainP99Ms: txWorkletToMainP99.length ? Math.max(...txWorkletToMainP99) : 0,
    maxTxMainToSendP99Ms: txMainToSendP99.length ? Math.max(...txMainToSendP99) : 0,
    maxTxWsSendP99Ms: txWsSendP99.length ? Math.max(...txWsSendP99) : 0,
    totalTxTimingFrames: sum(samples.map((s) => Number(s.txTimingFrameCount) || 0)),
    totalTxTimingDroppedFrames: sum(samples.map((s) => Number(s.txTimingDroppedFrameCount) || 0)),
    displayProfile:
      samples[samples.length - 1]?.displayProfile || currentSnapshot.displayProfile || '',
    finalSnapshot: samples[samples.length - 1] || currentSnapshot,
  };
}
