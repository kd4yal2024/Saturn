/**
 * Typed application state for Saturn Remote.
 *
 * This is the central mutable state object shared by all modules.
 * Every field has a documented default matching the inline HTML initializer.
 */

export interface PhonePanels {
  session: boolean;
  audio: boolean;
  routing: boolean;
  tuning: boolean;
  demod: boolean;
  display: boolean;
  log: boolean;
  telemetry: boolean;
  tx: boolean;
}

export const PHONE_PANEL_DEFAULTS: Readonly<PhonePanels> = Object.freeze({
  session: false,
  audio: false,
  routing: true,
  tuning: false,
  demod: false,
  display: false,
  log: true,
  telemetry: true,
  tx: false,
});

export interface AppState {
  // ── Connection ──────────────────────────────────────────────────────────
  ws: WebSocket | null;
  wsUrl: string;
  connected: boolean;
  bridgeReady: boolean;
  connectPending: boolean;
  connectionGeneration: number;
  iqStreaming: boolean;
  audioStreaming: boolean;
  demoMode: boolean;
  intentionalDisconnect: boolean;
  connectionAttemptStartedAt: number;
  connectionOpenMs: number | null;
  connectionLossStartedAt: number;
  connectionLossCount: number;
  connectionRecoveryMs: number | null;
  connectionRecoveredAt: number;
  wsMediaBacklogBytes: number;

  // ── Radio ───────────────────────────────────────────────────────────────
  mode: string;
  vfoA: number;
  vfoB: number;
  dds: number;
  rxAdc: number;
  rxAntenna: number;
  sampleRate: number;
  streamMode: string;

  // ── RX DSP ──────────────────────────────────────────────────────────────
  rxVolumeDb: number;
  rxNoiseReductionMode: string;
  rxNoiseReductionLevel: number;
  rxNr2GainMethod: string;
  rxNr2NpeMethod: string;
  rxNr2PostFilterEnabled: boolean;
  rxWbfmSupported: boolean;
  rxWbfmDeemphasis: string;
  rxWbfmStereoDetected: boolean;
  rxNbMode: string;
  rxNbThreshold: number;
  rxAnrTaps: number;
  rxAnrDelay: number;
  rxAnrGain: number;
  rxAnrLeakage: number;
  anfEnabled: boolean;
  rxAnfTaps: number;
  rxAnfDelay: number;
  rxAnfGain: number;
  rxAnfLeakage: number;
  agcMode: string;
  agcGain: number;

  // ── Filter ──────────────────────────────────────────────────────────────
  filterLow: number;
  filterHigh: number;
  rxFilterShiftHz: number;
  txFilterLow: number;
  txFilterHigh: number;

  // ── TX ──────────────────────────────────────────────────────────────────
  txEnabled: boolean;
  moxRequested: boolean;
  txPhase: 'rx' | 'armed' | 'keyed';
  txDrive: number;
  remoteTxRfEnabled: boolean | null;
  txMicGainDb: number;
  txMeterMode: string;
  micStream: MediaStream | null;
  micNode: { ctx: AudioContext; source: MediaStreamAudioSourceNode; proc: AudioNode; silentGain: GainNode } | null;
  micCapturing: boolean;
  pttRequestId: number;
  twoToneEnabled: boolean;
  twoTonePttOwned: boolean;
  txTwoToneFreq1: number;
  txTwoToneFreq2: number;
  txTwoToneLevelDb: number;
  txTwoToneInvertLsb: boolean;
  txTwoToneDelayMs: number;
  txNoiseGateEnabled: boolean;
  txNoiseGateThresholdDb: number;
  txTimeoutEnabled: boolean;
  txTimeoutSeconds: number;
  txTimeoutTimerId: ReturnType<typeof setTimeout> | null;
  armTimeoutTimerId: ReturnType<typeof setTimeout> | null;
  txReadyExpiresAt: number;
  txReadyTimerId: ReturnType<typeof setInterval> | null;
  txLockReason: string;

  // ── EQ / CFC ────────────────────────────────────────────────────────────
  rxEqEnabled: boolean;
  rxEqBands: number[];
  txEqEnabled: boolean;
  txEqBands: number[];
  cfcEnabled: boolean;
  cfcPrecomp: number;
  cfcBands: number[];
  txPhaseRotatorEnabled: boolean;
  txPhaseRotatorAuto: boolean;
  txPhaseRotatorCornerHz: number;
  pureSignalEnabled: boolean;
  pureSignalAutoAttenuate: boolean;
  pureSignalAttenuationDb: number;
  pureSignalState: 'off' | 'waiting' | 'calibrating' | 'correcting' | 'fault';
  pureSignalFeedbackLevel: number;
  pureSignalCalibrationCount: number;
  pureSignalCorrecting: boolean;
  pureSignalMaxTx: number;
  pureSignalFeedbackPackets: number;
  pureSignalFeedbackGaps: number;

  // ── Meters ──────────────────────────────────────────────────────────────
  meterDbm: number | null;
  txPower: number | null;
  swr: number | null;
  bridgeRttMs: number | null;
  bridgeRttAt: number;
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
  safetyQueueDepthOverflowCount: number;
  txUplinkDegraded: boolean;
  txMicSeq: number;
  txMicDroppedCount: number;
  txUplinkBufferedBytes: number;
  txUplinkBufferedHwmBytes: number;
  txUplinkStatsAt: number;
  txUplinkDegradedAt: number;
  txMicLastArrivedSeq: number;
  txMicSeqGapCount: number;
  txMicAgeMs: number;
  txFaultReason: string | null;
  txCodecDecodeErrorCount: number;
  txCodecStaleDropCount: number;
  txCodecReleaseFlushCount: number;
  txCodecRequested: 'pcm' | 'opus_nb' | 'opus_wb';
  txCodecAccepted: 'pcm' | 'opus_nb' | 'opus_wb' | null;
  txCodecNegotiatedAt: number;
  txCodecRejectReason: string | null;
  txCodecDetectedCaps: Array<'pcm' | 'opus_nb' | 'opus_wb'>;
  txCodecAdvertisedCaps: Array<'pcm' | 'opus_nb' | 'opus_wb'>;
  txCodecWebCodecsAudioEncoder: boolean;
  txCodecDetectionAt: number;
  remoteClientRole: 'operator' | 'viewer' | null;
  remoteClientId: string | null;
  bridgeRttNonce: number;
  bridgeRttTimerId: ReturnType<typeof setInterval> | null;
  meterDbmVisual: number | null;
  meterDbmPeak: number | null;
  meterDbmPeakAt: number;
  txPowerVisual: number | null;
  txPowerPeak: number | null;
  txPowerPeakAt: number;
  txPowerAvg: number | null;
  swrVisual: number | null;
  lastMeterVisualAt: number;

  // ── Display ─────────────────────────────────────────────────────────────
  displayZoom: number;
  displayFloorDb: number;
  displayCeilingDb: number;
  displayCaption: string;
  spectrumAutoRange: boolean;
  spectrumAverage: number;
  spectrumPeakHold: boolean;
  spectrumTraceColor: string;
  spectrumTraceSmoothing: number;
  spectrumTraceFill: number;
  spectrumPeakGlow: number;
  spectrumGlassSheen: number;
  waterfallContrast: number;
  waterfallSmoothing: number;
  spectrumAverageBypassFrames: number;
  waterfallAutoRange: boolean;
  waterfallFloorDb: number;
  waterfallCeilingDb: number;
  waterfallSpeed: number;
  waterfallPalette: string;
  showGrid: boolean;
  showCenterLine: boolean;
  showBandEdges: boolean;

  // ── FFT / Spectrum State ────────────────────────────────────────────────
  latestBins: Float32Array;
  fftWidth: number;
  peakHoldBins: Float32Array;
  iqPackets: Array<{ iqs: Float32Array; sampleRate: number; receivedAt: number }>;
  lastSpectrumRenderAt: number;
  spectrumShiftCarryBins: number;
  waterfallShiftCarryPixels: number;
  waterfallSettleFrames: number;
  waterfallFrameSkipCounter: number;

  // ── Audio ───────────────────────────────────────────────────────────────
  audioFramesPlayed: number;
  audioSampleRate: number;
  audioChannels: number;
  audioCtx: AudioContext | null;
  audioGainNode: GainNode | null;
  audioSources: Set<AudioBufferSourceNode>;
  audioNextTime: number;
  audioBackpressureDrops: number;
  lastAudioSequence: number | null;
  lastAudioSeqGapReportAt: number;
  audioWorkletMode: 'sab' | 'msg' | null;
  rxWorkletNode: AudioWorkletNode | null;
  rxRingBuf: SharedArrayBuffer | null;
  rxCtrlBuf: SharedArrayBuffer | null;
  rxRingF32: Float32Array | null;
  rxCtrlU32: Uint32Array | null;
  rxCtrlF32: Float32Array | null;
  rxWorkletDrops: number;
  rxWorkletQueuedMs: number;
  rxWorkletUnderruns: number;
  rxWorkletOverflows: number;
  rxWorkletTelemetryAt: number;
  audioContextBaseLatencyMs: number | null;
  audioContextOutputLatencyMs: number | null;
  lastAudioFrameAt: number;
  lastAudioArrivalAt: number;
  lastAudioPacketDurationMs: number;
  rxAudioJitterSamples: number[];
  rxAudioJitterP50Ms: number;
  rxAudioJitterP95Ms: number;
  rxAudioJitterP99Ms: number;
  rxAudioJitterSummaryAt: number;

  // ── Frame Stats ─────────────────────────────────────────────────────────
  lastFrameAt: number;
  frameCounter: number;
  frameRate: number;
  lastSummaryAt: number;

  // ── UI / Layout ─────────────────────────────────────────────────────────
  setupPanel: string;
  setupDspPanel: string;
  activeProfile: string;
  startupProfile: string;
  theme: string;
  layoutMode: string;
  phonePanels: PhonePanels;
  phoneWaterfallVisible: boolean;
  vfoTuneStepHz: number;
  frequencyLock: boolean;
  keepScreenAwake: boolean;
  wakeLockSentinel: WakeLockSentinel | null;
  wakeLockActive: boolean;
  wakeLockPending: boolean;

  // ── Profiles / Band Memory ──────────────────────────────────────────────
  remoteProfiles: Record<string, unknown>;
  bandMemory: Record<string, unknown>;
  applyingBandMemory: boolean;

  // ── Performance ─────────────────────────────────────────────────────────
  displayProfile: string;
  perfCaptureEnabled: boolean;
  perfConsoleLogging: boolean;
  perfSnapshots: unknown[];
  perfSnapshotsMax: number;
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

/** Create a fresh AppState with all defaults. */
export function createAppState(): AppState {
  return {
    ws: null,
    wsUrl: '',
    connected: false,
    bridgeReady: false,
    connectPending: false,
    connectionGeneration: 0,
    iqStreaming: false,
    audioStreaming: false,
    demoMode: false,
    intentionalDisconnect: false,
    connectionAttemptStartedAt: 0,
    connectionOpenMs: null,
    connectionLossStartedAt: 0,
    connectionLossCount: 0,
    connectionRecoveryMs: null,
    connectionRecoveredAt: 0,
    wsMediaBacklogBytes: 0,

    mode: 'USB',
    vfoA: 14200000,
    vfoB: 14200000,
    dds: 14200000,
    rxAdc: 0,
    rxAntenna: 1,
    sampleRate: 192000,
    streamMode: 'lan',

    rxVolumeDb: -10.0,
    rxNoiseReductionMode: 'NR2',
    rxNoiseReductionLevel: 100,
    rxNr2GainMethod: 'GAMMA',
    rxNr2NpeMethod: 'OSMS',
    rxNr2PostFilterEnabled: true,
    rxWbfmSupported: false,
    rxWbfmDeemphasis: 'NA_75US',
    rxWbfmStereoDetected: false,
    rxNbMode: 'OFF',
    rxNbThreshold: 4.95,
    rxAnrTaps: 64,
    rxAnrDelay: 16,
    rxAnrGain: 0.0002,
    rxAnrLeakage: 0.00005,
    anfEnabled: false,
    rxAnfTaps: 64,
    rxAnfDelay: 16,
    rxAnfGain: 0.00012,
    rxAnfLeakage: 0.00008,
    agcMode: 'MEDIUM',
    agcGain: 80,

    filterLow: 50,
    filterHigh: 3050,
    rxFilterShiftHz: 0,
    txFilterLow: 250,
    txFilterHigh: 3000,

    txEnabled: false,
    moxRequested: false,
    txPhase: 'rx',
    txDrive: 10,
    remoteTxRfEnabled: null,
    txMicGainDb: -12,
    txMeterMode: 'peak',
    micStream: null,
    micNode: null,
    micCapturing: false,
    pttRequestId: 0,
    twoToneEnabled: false,
    twoTonePttOwned: false,
    txTwoToneFreq1: 700,
    txTwoToneFreq2: 1900,
    txTwoToneLevelDb: 0.0,
    txTwoToneInvertLsb: true,
    txTwoToneDelayMs: 0,
    txNoiseGateEnabled: true,
    txNoiseGateThresholdDb: -30.0,
    txTimeoutEnabled: true,
    txTimeoutSeconds: 180,
    txTimeoutTimerId: null,
    armTimeoutTimerId: null,
    txReadyExpiresAt: 0,
    txReadyTimerId: null,
    txLockReason: 'connect',

    rxEqEnabled: false,
    rxEqBands: new Array(11).fill(0),
    txEqEnabled: false,
    txEqBands: new Array(11).fill(0),
    cfcEnabled: false,
    cfcPrecomp: 0.0,
    cfcBands: new Array(11).fill(0.0),
    txPhaseRotatorEnabled: false,
    txPhaseRotatorAuto: false,
    txPhaseRotatorCornerHz: 338,
    pureSignalEnabled: false,
    pureSignalAutoAttenuate: true,
    pureSignalAttenuationDb: 0,
    pureSignalState: 'off',
    pureSignalFeedbackLevel: 0,
    pureSignalCalibrationCount: 0,
    pureSignalCorrecting: false,
    pureSignalMaxTx: 0,
    pureSignalFeedbackPackets: 0,
    pureSignalFeedbackGaps: 0,

    meterDbm: null,
    txPower: null,
    swr: null,
    bridgeRttMs: null,
    bridgeRttAt: 0,
    backpressureSafetyP50Us: 0,
    backpressureSafetyP95Us: 0,
    backpressureSafetyP99Us: 0,
    backpressureControlP50Us: 0,
    backpressureControlP95Us: 0,
    backpressureControlP99Us: 0,
    displayReplacedPerSec: 0,
    displayDroppedPerSec: 0,
    bridgeAudioDroppedPerSec: 0,
    bridgeAudioSeqGapCount: 0,
    audioSeqGapCount: 0,
    audioPanicDrainCount: 0,
    sendBlockedMs: 0,
    outboundHighWatermarkBytes: 0,
    bridgeOutboundQueuedBytes: 0,
    bridgeTcpOutqHighWatermarkBytes: 0,
    displayRateLimitedPerSec: 0,
    safetyQueueDepthOverflowCount: 0,
    txUplinkDegraded: false,
    txMicSeq: 0,
    txMicDroppedCount: 0,
    txUplinkBufferedBytes: 0,
    txUplinkBufferedHwmBytes: 0,
    txUplinkStatsAt: 0,
    txUplinkDegradedAt: 0,
    txMicLastArrivedSeq: 0,
    txMicSeqGapCount: 0,
    txMicAgeMs: 0,
    txFaultReason: null,
    txCodecDecodeErrorCount: 0,
    txCodecStaleDropCount: 0,
    txCodecReleaseFlushCount: 0,
    txCodecRequested: 'pcm',
    txCodecAccepted: null,
    txCodecNegotiatedAt: 0,
    txCodecRejectReason: null,
    txCodecDetectedCaps: ['pcm'],
    txCodecAdvertisedCaps: ['pcm'],
    txCodecWebCodecsAudioEncoder: false,
    txCodecDetectionAt: 0,
    remoteClientRole: null,
    remoteClientId: null,
    bridgeRttNonce: 0,
    bridgeRttTimerId: null,
    meterDbmVisual: null,
    meterDbmPeak: null,
    meterDbmPeakAt: 0,
    txPowerVisual: null,
    txPowerPeak: null,
    txPowerPeakAt: 0,
    txPowerAvg: null,
    swrVisual: null,
    lastMeterVisualAt: 0,

    displayZoom: 1,
    displayFloorDb: -200,
    displayCeilingDb: -120,
    displayCaption: 'Waiting for TCI IQ frames from saturn-bridge',
    spectrumAutoRange: true,
    spectrumAverage: 1,
    spectrumPeakHold: false,
    spectrumTraceColor: '#62d0ff',
    spectrumTraceSmoothing: 0,
    spectrumTraceFill: 0,
    spectrumPeakGlow: 0,
    spectrumGlassSheen: 0,
    waterfallContrast: 100,
    waterfallSmoothing: 0,
    spectrumAverageBypassFrames: 0,
    waterfallAutoRange: true,
    waterfallFloorDb: -200,
    waterfallCeilingDb: -120,
    waterfallSpeed: 1,
    waterfallPalette: 'classic',
    showGrid: true,
    showCenterLine: true,
    showBandEdges: true,

    latestBins: new Float32Array(1024),
    fftWidth: 1024,
    peakHoldBins: new Float32Array(1024),
    iqPackets: [],
    lastSpectrumRenderAt: 0,
    spectrumShiftCarryBins: 0,
    waterfallShiftCarryPixels: 0,
    waterfallSettleFrames: 0,
    waterfallFrameSkipCounter: 0,

    audioFramesPlayed: 0,
    audioSampleRate: 48000,
    audioChannels: 2,
    audioCtx: null,
    audioGainNode: null,
    audioSources: new Set(),
    audioNextTime: 0,
    audioBackpressureDrops: 0,
    lastAudioSequence: null,
    lastAudioSeqGapReportAt: 0,
    audioWorkletMode: null,
    rxWorkletNode: null,
    rxRingBuf: null,
    rxCtrlBuf: null,
    rxRingF32: null,
    rxCtrlU32: null,
    rxCtrlF32: null,
    rxWorkletDrops: 0,
    rxWorkletQueuedMs: 0,
    rxWorkletUnderruns: 0,
    rxWorkletOverflows: 0,
    rxWorkletTelemetryAt: 0,
    audioContextBaseLatencyMs: null,
    audioContextOutputLatencyMs: null,
    lastAudioFrameAt: 0,
    lastAudioArrivalAt: 0,
    lastAudioPacketDurationMs: 0,
    rxAudioJitterSamples: [],
    rxAudioJitterP50Ms: 0,
    rxAudioJitterP95Ms: 0,
    rxAudioJitterP99Ms: 0,
    rxAudioJitterSummaryAt: 0,

    lastFrameAt: 0,
    frameCounter: 0,
    frameRate: 0,
    lastSummaryAt: 0,

    setupPanel: 'profiles',
    setupDspPanel: 'nr',
    activeProfile: '',
    startupProfile: '',
    theme: 'dark',
    layoutMode: 'desktop',
    phonePanels: { ...PHONE_PANEL_DEFAULTS },
    phoneWaterfallVisible: false,
    vfoTuneStepHz: 100,
    frequencyLock: false,
    keepScreenAwake: false,
    wakeLockSentinel: null,
    wakeLockActive: false,
    wakeLockPending: false,

    remoteProfiles: {},
    bandMemory: {},
    applyingBandMemory: false,

    displayProfile: '',
    perfCaptureEnabled: false,
    perfConsoleLogging: false,
    perfSnapshots: [],
    perfSnapshotsMax: 600,
    browserMainLagP95Ms: 0,
    browserMainLagP99Ms: 0,
    browserMainLagMaxMs: 0,
    browserRafIntervalP95Ms: 0,
    browserRafIntervalP99Ms: 0,
    browserRafIntervalMaxMs: 0,
    txWorkletToMainP95Ms: 0,
    txWorkletToMainP99Ms: 0,
    txWorkletToMainMaxMs: 0,
    txMainToSendP95Ms: 0,
    txMainToSendP99Ms: 0,
    txMainToSendMaxMs: 0,
    txWsSendP95Ms: 0,
    txWsSendP99Ms: 0,
    txWsSendMaxMs: 0,
    txTimingFrameCount: 0,
    txTimingDroppedFrameCount: 0,
  };
}
