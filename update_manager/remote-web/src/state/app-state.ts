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
  connectPending: boolean;
  iqStreaming: boolean;
  audioStreaming: boolean;
  demoMode: boolean;

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
  txFilterLow: number;
  txFilterHigh: number;

  // ── TX ──────────────────────────────────────────────────────────────────
  txEnabled: boolean;
  moxRequested: boolean;
  txPhase: 'rx' | 'armed' | 'keyed';
  txDrive: number;
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

  // ── Meters ──────────────────────────────────────────────────────────────
  meterDbm: number | null;
  txPower: number | null;
  swr: number | null;
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
  audioCtx: AudioContext | null;
  audioGainNode: GainNode | null;
  audioSources: Set<AudioBufferSourceNode>;
  audioNextTime: number;
  audioBackpressureDrops: number;
  audioWorkletMode: 'sab' | 'msg' | null;
  rxWorkletNode: AudioWorkletNode | null;
  rxRingBuf: SharedArrayBuffer | null;
  rxCtrlBuf: SharedArrayBuffer | null;
  rxRingF32: Float32Array | null;
  rxCtrlU32: Uint32Array | null;
  rxCtrlF32: Float32Array | null;
  rxWorkletDrops: number;

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
  perfCaptureEnabled: boolean;
  perfConsoleLogging: boolean;
  perfSnapshots: unknown[];
  perfSnapshotsMax: number;
}

/** Create a fresh AppState with all defaults. */
export function createAppState(): AppState {
  return {
    ws: null,
    wsUrl: '',
    connected: false,
    connectPending: false,
    iqStreaming: false,
    audioStreaming: false,
    demoMode: false,

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
    txFilterLow: 50,
    txFilterHigh: 3050,

    txEnabled: false,
    moxRequested: false,
    txPhase: 'rx',
    txDrive: 10,
    txMicGainDb: 0,
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

    meterDbm: null,
    txPower: null,
    swr: null,
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
    audioCtx: null,
    audioGainNode: null,
    audioSources: new Set(),
    audioNextTime: 0,
    audioBackpressureDrops: 0,
    audioWorkletMode: null,
    rxWorkletNode: null,
    rxRingBuf: null,
    rxCtrlBuf: null,
    rxRingF32: null,
    rxCtrlU32: null,
    rxCtrlF32: null,
    rxWorkletDrops: 0,

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
    frequencyLock: false,
    keepScreenAwake: false,
    wakeLockSentinel: null,
    wakeLockActive: false,
    wakeLockPending: false,

    remoteProfiles: {},
    bandMemory: {},
    applyingBandMemory: false,

    perfCaptureEnabled: false,
    perfConsoleLogging: false,
    perfSnapshots: [],
    perfSnapshotsMax: 600,
  };
}
