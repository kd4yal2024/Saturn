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
  finalAudioBackpressureDrops: number;
  finalRxWorkletDrops: number;
  finalSnapshot: PerfSnapshot;
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
  const backpressure = samples.map((s) => Number(s.audioBackpressureDrops) || 0);
  const workletDrops = samples.map((s) => Number(s.rxWorkletDrops) || 0);
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
    finalAudioBackpressureDrops,
    finalRxWorkletDrops,
    finalSnapshot: samples[samples.length - 1] || currentSnapshot,
  };
}
