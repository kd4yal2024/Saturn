import type { DemodMode } from '../radio/passband';

export type LayoutMode = 'desktop' | 'phone';
export type ThemeMode = 'dark' | 'light';
export type StreamMode = 'lan' | 'wan';
export type NoiseReductionMode = 'OFF' | 'NR1' | 'NR2' | 'NR3' | 'NR4';
export type Nr2GainMethod = 'GAUSSIAN' | 'GAUSSIAN_LOG' | 'GAMMA' | 'TRAINED';
export type Nr2NpeMethod = 'OSMS' | 'MMSE' | 'NSTAT';
export type WbfmDeemphasis = 'OFF' | 'NA_75US' | 'EU_50US';
export type NoiseBlankerMode = 'OFF' | 'NB1' | 'NB2';
export type AgcMode = 'OFF' | 'LONG' | 'SLOW' | 'MEDIUM' | 'FAST';
export type TxMeterMode = 'peak' | 'avg';
export type WaterfallPalette = 'classic' | 'ember' | 'ice' | 'forest';

export type PhonePanels = {
  radio: boolean;
  dsp: boolean;
  tx: boolean;
  meters: boolean;
  setup: boolean;
  logs: boolean;
};

export type RadioPrefs = {
  sampleRate: number;
  rxAdc: number;
  rxAntenna: number;
  mode: DemodMode;
  rxVolumeDb: number;
  rxNoiseReductionMode: NoiseReductionMode;
  rxNoiseReductionLevel: number;
  rxNr2GainMethod: Nr2GainMethod;
  rxNr2NpeMethod: Nr2NpeMethod;
  rxNr2PostFilterEnabled: boolean;
  rxWbfmDeemphasis: WbfmDeemphasis;
  rxNbMode: NoiseBlankerMode;
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
  agcMode: AgcMode;
  agcGain: number;
  filterLow: number;
  filterHigh: number;
  txDrive: number;
  txMicGainDb: number;
  txFilterLow: number;
  txFilterHigh: number;
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
  txMeterMode: TxMeterMode;
  twoToneEnabled: boolean;
  txTwoToneFreq1: number;
  txTwoToneFreq2: number;
  txTwoToneLevelDb: number;
  txTwoToneInvertLsb: boolean;
  txTwoToneDelayMs: number;
  txNoiseGateEnabled: boolean;
  txNoiseGateThresholdDb: number;
  txTimeoutEnabled: boolean;
  txTimeoutSeconds: number;
};

export type DisplayPrefs = {
  spectrumAutoRange: boolean;
  spectrumFloorDb: number;
  spectrumCeilingDb: number;
  waterfallAutoRange: boolean;
  waterfallFloorDb: number;
  waterfallCeilingDb: number;
  spectrumAverage: number;
  waterfallSpeed: number;
  waterfallPalette: WaterfallPalette;
  spectrumPeakHold: boolean;
  spectrumTraceColor: string;
  spectrumTraceSmoothing: number;
  spectrumPeakGlow: number;
  spectrumGlassSheen: number;
  showGrid: boolean;
  showCenterLine: boolean;
  showBandEdges: boolean;
};

export type BandMemoryEntry = {
  frequency: number;
  radioPrefs: Partial<RadioPrefs>;
  displayPrefs: Partial<DisplayPrefs>;
  displayZoom: number;
};

export type BandMemory = Record<string, BandMemoryEntry>;

export type ProfileSettings = {
  wsUrl: string;
  displayZoom: number;
  frequencyLock: boolean;
  keepScreenAwake: boolean;
  streamMode: StreamMode;
  layoutMode: LayoutMode;
  theme: ThemeMode;
  phonePanels: PhonePanels;
  displayPrefs: DisplayPrefs;
  radioPrefs: RadioPrefs;
  bandMemory: BandMemory;
};

export type RemoteSettings = ProfileSettings & {
  activeProfile: string | null;
};

export type SettingsState = {
  wsUrl: string;
  displayZoom: number;
  frequencyLock: boolean;
  keepScreenAwake: boolean;
  streamMode: StreamMode;
  layoutMode: LayoutMode;
  theme: ThemeMode;
  phonePanels: PhonePanels;
  activeProfile: string;
  radioPrefs: RadioPrefs;
  displayPrefs: DisplayPrefs;
  bandMemory: BandMemory;
};

export type WsUrlEnvironment = {
  protocol: string;
  host: string;
  hostname: string;
  href: string;
};

export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends (infer U)[]
    ? U[]
    : T[K] extends object
      ? DeepPartial<T[K]>
      : T[K];
};
