import {
  DEFAULT_DISPLAY_PREFS,
  DEFAULT_PHONE_PANELS,
  DEFAULT_RADIO_PREFS,
  createDefaultSettingsState,
} from './defaults';
import {
  type BandMemory,
  type DeepPartial,
  type DisplayPrefs,
  type PhonePanels,
  type ProfileSettings,
  type RadioPrefs,
  type RemoteSettings,
  type SettingsState,
  type StreamMode,
  type WaterfallPalette,
  type WsUrlEnvironment,
} from './types';
import { clampDemodMode } from '../radio/passband';

const ALLOWED_SAMPLE_RATES = [48000, 96000, 192000, 384000] as const;
const WATERFALL_PALETTES: readonly WaterfallPalette[] = ['classic', 'ember', 'ice', 'forest'];
const HAM_BAND_LABELS = new Set(['160m', '80m', '60m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m', 'FM']);

export function safeFiniteNumber(value: unknown, fallback = 0): number {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

export function clampSampleRateHz(value: unknown): number {
  const sampleRate = Math.round(Number(value));
  return ALLOWED_SAMPLE_RATES.includes(sampleRate as (typeof ALLOWED_SAMPLE_RATES)[number]) ? sampleRate : 192000;
}

export function normalizeStreamMode(value: unknown): StreamMode {
  return String(value || '').trim().toLowerCase() === 'wan' ? 'wan' : 'lan';
}

export function clampRxAdc(value: unknown): number {
  return Math.max(0, Math.min(2, Math.round(Number(value))));
}

export function clampRxAntenna(value: unknown): number {
  return Math.max(1, Math.min(3, Math.round(Number(value))));
}

export function clampRxVolumeDb(value: unknown): number {
  return Math.max(-40, Math.min(12, safeFiniteNumber(value, DEFAULT_RADIO_PREFS.rxVolumeDb)));
}

export function normalizeNrMode(value: unknown): RadioPrefs['rxNoiseReductionMode'] {
  const text = String(value || '').trim().toUpperCase();
  if (['NR1', 'ANR', '1'].includes(text)) return 'NR1';
  if (['NR2', 'EMNR', '2'].includes(text)) return 'NR2';
  if (['NR3', 'RNNR', '3'].includes(text)) return 'NR3';
  if (['NR4', 'SBNR', '4'].includes(text)) return 'NR4';
  return 'OFF';
}

export function clampRxNoiseReductionLevel(value: unknown): number {
  return Math.max(0, Math.min(100, Math.round(safeFiniteNumber(value, DEFAULT_RADIO_PREFS.rxNoiseReductionLevel))));
}

export function normalizeNr2GainMethod(value: unknown): RadioPrefs['rxNr2GainMethod'] {
  const text = String(value || '').trim().toUpperCase();
  if (['GAUSSIAN', 'GAUSSIAN_LINEAR', '0'].includes(text)) return 'GAUSSIAN';
  if (['GAUSSIAN_LOG', 'LOG', '1'].includes(text)) return 'GAUSSIAN_LOG';
  if (['TRAINED', '3'].includes(text)) return 'TRAINED';
  return 'GAMMA';
}

export function normalizeNr2NpeMethod(value: unknown): RadioPrefs['rxNr2NpeMethod'] {
  const text = String(value || '').trim().toUpperCase();
  if (['MMSE', '1'].includes(text)) return 'MMSE';
  if (['NSTAT', '2'].includes(text)) return 'NSTAT';
  return 'OSMS';
}

export function normalizeWbfmDeemphasis(value: unknown): RadioPrefs['rxWbfmDeemphasis'] {
  const text = String(value || '').trim().toUpperCase();
  if (['OFF', 'NONE', '0'].includes(text)) return 'OFF';
  if (['EU', 'EUROPE', 'EU_50US', '50US', '2'].includes(text)) return 'EU_50US';
  return 'NA_75US';
}

export function clampTxPhaseRotatorCornerHz(value: unknown): number {
  return Math.max(50, Math.min(2000, Math.round(safeFiniteNumber(value, 338))));
}

export function clampPureSignalAttenuationDb(value: unknown): number {
  return Math.max(0, Math.min(31, Math.round(safeFiniteNumber(value, 0))));
}

export function normalizeNbMode(value: unknown): RadioPrefs['rxNbMode'] {
  const text = String(value || '').trim().toUpperCase();
  if (['NB1', 'NB', '1'].includes(text)) return 'NB1';
  if (['NB2', 'NOB', '2'].includes(text)) return 'NB2';
  if (['NB3', 'SNB', 'SNBA', '3'].includes(text)) return 'NB3';
  return 'OFF';
}

export function clampTxDexpThresholdDb(value: unknown): number {
  return Math.max(-80, Math.min(-6, safeFiniteNumber(value, DEFAULT_RADIO_PREFS.txDexpThresholdDb)));
}

export function clampTxDexpExpansionDb(value: unknown): number {
  return Math.max(0, Math.min(30, safeFiniteNumber(value, DEFAULT_RADIO_PREFS.txDexpExpansionDb)));
}

export function clampTxSpeechProcessorGainDb(value: unknown): number {
  return Math.max(0, Math.min(20, safeFiniteNumber(value, DEFAULT_RADIO_PREFS.txSpeechProcessorGainDb)));
}

export function clampRxNbThreshold(value: unknown): number {
  return Math.max(2.5, Math.min(82.5, safeFiniteNumber(value, DEFAULT_RADIO_PREFS.rxNbThreshold)));
}

export function normalizeAgcMode(value: unknown): RadioPrefs['agcMode'] {
  const text = String(value || '').trim().toUpperCase();
  if (['OFF', '0'].includes(text)) return 'OFF';
  if (['LONG', '1'].includes(text)) return 'LONG';
  if (['SLOW', '2'].includes(text)) return 'SLOW';
  if (['FAST', '4'].includes(text)) return 'FAST';
  return 'MEDIUM';
}

export function clampAgcGain(value: unknown): number {
  return Math.max(0, Math.min(100, Math.round(safeFiniteNumber(value, DEFAULT_RADIO_PREFS.agcGain))));
}

export function clampDspTapCount(value: unknown): number {
  return Math.max(1, Math.min(128, Math.round(safeFiniteNumber(value, 64))));
}

export function clampDspDelay(value: unknown): number {
  return Math.max(0, Math.min(127, Math.round(safeFiniteNumber(value, 16))));
}

export function clampDspGain(value: unknown, fallback = 0.0001): number {
  return Math.max(0, Math.min(1, safeFiniteNumber(value, fallback)));
}

export function clampDspLeakage(value: unknown, fallback = 0.0001): number {
  return Math.max(0, Math.min(1, safeFiniteNumber(value, fallback)));
}

export function clampFilterLowHz(value: unknown): number {
  return Math.max(0, Math.min(300, safeFiniteNumber(value, 50)));
}

export function clampFilterHighHz(value: unknown): number {
  return Math.max(500, Math.min(6000, safeFiniteNumber(value, 3050)));
}

export function clampTxDriveWatts(value: unknown): number {
  return Math.max(1, Math.min(100, Math.round(safeFiniteNumber(value, DEFAULT_RADIO_PREFS.txDrive))));
}

export function clampTxMicGainDb(value: unknown): number {
  return Math.max(-20, Math.min(20, safeFiniteNumber(value, DEFAULT_RADIO_PREFS.txMicGainDb)));
}

export function normalizeTxMeterMode(value: unknown): RadioPrefs['txMeterMode'] {
  return String(value || '').trim().toLowerCase() === 'avg' ? 'avg' : 'peak';
}

export function clampTwoToneFreqHz(value: unknown, fallback: number): number {
  return Math.max(10, Math.min(10_000, Math.round(safeFiniteNumber(value, fallback))));
}

export function clampTwoToneLevelDb(value: unknown): number {
  return Math.max(-40, Math.min(0, safeFiniteNumber(value, 0)));
}

export function clampTwoToneDelayMs(value: unknown): number {
  return Math.max(0, Math.min(2000, Math.round(safeFiniteNumber(value, 0))));
}

export function clampEqGainDb(value: unknown): number {
  return Math.max(-20, Math.min(20, Math.round(safeFiniteNumber(value, 0))));
}

export function sanitizeEqBands(values: unknown, fallback = new Array(11).fill(0)): number[] {
  const bands = fallback.slice(0, 11);
  while (bands.length < 11) bands.push(0);
  for (let i = 1; i <= 10; i += 1) {
    const next = Array.isArray(values) ? values[i] : undefined;
    bands[i] = Number.isFinite(Number(next)) ? clampEqGainDb(next) : clampEqGainDb(bands[i] || 0);
  }
  bands[0] = 0;
  return bands;
}

export function sanitizeCfcBands(values: unknown, fallback = new Array(11).fill(0)): number[] {
  const bands = fallback.slice(0, 11);
  while (bands.length < 11) bands.push(0);
  for (let i = 1; i <= 10; i += 1) {
    const next = Array.isArray(values) ? values[i] : undefined;
    bands[i] = Number.isFinite(Number(next))
      ? Math.max(0, Math.min(20, Number(next)))
      : Math.max(0, Math.min(20, Number(bands[i]) || 0));
  }
  bands[0] = 0;
  return bands;
}

export function clampDisplayDb(value: unknown, fallback: number): number {
  return Math.max(-220, Math.min(-20, safeFiniteNumber(value, fallback)));
}

export function clampSpectrumAverage(value: unknown): number {
  return Math.max(1, Math.min(12, Math.round(safeFiniteNumber(value, 1))));
}

export function clampWaterfallSpeed(value: unknown): number {
  return Math.max(1, Math.min(6, Math.round(safeFiniteNumber(value, 1))));
}

export function clampWaterfallContrast(value: unknown): number {
  return Math.max(50, Math.min(200, Math.round(safeFiniteNumber(value, 100))));
}

export function normalizeWaterfallPalette(value: unknown): WaterfallPalette {
  const palette = String(value || '').trim().toLowerCase();
  return WATERFALL_PALETTES.includes(palette as WaterfallPalette) ? (palette as WaterfallPalette) : 'classic';
}

export function normalizeSpectrumTraceColor(value: unknown): string {
  const color = String(value || '').trim().toLowerCase();
  return /^#[0-9a-f]{6}$/.test(color) ? color : DEFAULT_DISPLAY_PREFS.spectrumTraceColor;
}

export function clampSpectrumVisualEffect(value: unknown): number {
  return Math.max(0, Math.min(100, Math.round(safeFiniteNumber(value, 0))));
}

export function sanitizePhonePanels(values: unknown, fallback: PhonePanels = { ...DEFAULT_PHONE_PANELS }): PhonePanels {
  const panels = { ...fallback };
  if (!values || typeof values !== 'object') return panels;
  const record = values as Record<string, unknown>;
  for (const key of Object.keys(panels) as (keyof PhonePanels)[]) {
    const next = record[key];
    if (typeof next === 'boolean') {
      panels[key] = next;
    }
  }
  return panels;
}

export function sameOriginProxyWsUrl(env: WsUrlEnvironment): string {
  const scheme = env.protocol === 'https:' ? 'wss' : 'ws';
  const authority = env.host || env.hostname || '127.0.0.1';
  return `${scheme}://${authority}/tci`;
}

export function normalizeWsUrlPreference(raw: unknown, env: WsUrlEnvironment): string {
  const fallback = env.protocol === 'http:' || env.protocol === 'https:'
    ? sameOriginProxyWsUrl(env)
    : `ws://${env.hostname || '127.0.0.1'}:50001`;
  const text = String(raw || '').trim();
  if (!text) return fallback;

  if (env.protocol === 'https:' || env.protocol === 'http:') {
    try {
      const parsed = new URL(text, env.href);
      const expectedProtocol = env.protocol === 'https:' ? 'wss:' : 'ws:';
      if (parsed.protocol === expectedProtocol && parsed.host === env.host && parsed.pathname === '/tci') {
        return parsed.toString();
      }
    } catch {
      // Fall through to the same-origin proxy.
    }
    return sameOriginProxyWsUrl(env);
  }

  try {
    return new URL(text, env.href).toString();
  } catch {
    return fallback;
  }
}

export function normalizeRadioPrefs(input: DeepPartial<RadioPrefs> | null | undefined): RadioPrefs {
  const source = input || {};
  return {
    sampleRate: clampSampleRateHz(source.sampleRate),
    rxAdc: clampRxAdc(source.rxAdc),
    rxAntenna: clampRxAntenna(source.rxAntenna),
    mode: clampDemodMode(source.mode),
    rxVolumeDb: clampRxVolumeDb(source.rxVolumeDb),
    rxNoiseReductionMode: normalizeNrMode(source.rxNoiseReductionMode),
    rxNoiseReductionLevel: clampRxNoiseReductionLevel(source.rxNoiseReductionLevel),
    rxNr2GainMethod: normalizeNr2GainMethod(source.rxNr2GainMethod),
    rxNr2NpeMethod: normalizeNr2NpeMethod(source.rxNr2NpeMethod),
    rxNr2PostFilterEnabled: source.rxNr2PostFilterEnabled !== false,
    rxWbfmDeemphasis: normalizeWbfmDeemphasis(source.rxWbfmDeemphasis),
    rxNbMode: normalizeNbMode(source.rxNbMode),
    rxNbThreshold: clampRxNbThreshold(source.rxNbThreshold),
    rxAnrTaps: clampDspTapCount(source.rxAnrTaps),
    rxAnrDelay: clampDspDelay(source.rxAnrDelay),
    rxAnrGain: clampDspGain(source.rxAnrGain, DEFAULT_RADIO_PREFS.rxAnrGain),
    rxAnrLeakage: clampDspLeakage(source.rxAnrLeakage, DEFAULT_RADIO_PREFS.rxAnrLeakage),
    anfEnabled: Boolean(source.anfEnabled),
    rxAnfTaps: clampDspTapCount(source.rxAnfTaps),
    rxAnfDelay: clampDspDelay(source.rxAnfDelay),
    rxAnfGain: clampDspGain(source.rxAnfGain, DEFAULT_RADIO_PREFS.rxAnfGain),
    rxAnfLeakage: clampDspLeakage(source.rxAnfLeakage, DEFAULT_RADIO_PREFS.rxAnfLeakage),
    agcMode: normalizeAgcMode(source.agcMode),
    agcGain: clampAgcGain(source.agcGain),
    filterLow: clampFilterLowHz(source.filterLow),
    filterHigh: clampFilterHighHz(source.filterHigh),
    txDrive: clampTxDriveWatts(source.txDrive),
    txMicGainDb: clampTxMicGainDb(source.txMicGainDb),
    txFilterLow: clampFilterLowHz(source.txFilterLow),
    txFilterHigh: clampFilterHighHz(source.txFilterHigh),
    rxEqEnabled: Boolean(source.rxEqEnabled),
    rxEqBands: sanitizeEqBands(source.rxEqBands, DEFAULT_RADIO_PREFS.rxEqBands),
    txEqEnabled: Boolean(source.txEqEnabled),
    txEqBands: sanitizeEqBands(source.txEqBands, DEFAULT_RADIO_PREFS.txEqBands),
    cfcEnabled: Boolean(source.cfcEnabled),
    cfcPrecomp: safeFiniteNumber(source.cfcPrecomp, 0),
    cfcBands: sanitizeCfcBands(source.cfcBands, DEFAULT_RADIO_PREFS.cfcBands),
    txPhaseRotatorEnabled: Boolean(source.txPhaseRotatorEnabled),
    txPhaseRotatorAuto: Boolean(source.txPhaseRotatorAuto),
    txPhaseRotatorCornerHz: clampTxPhaseRotatorCornerHz(source.txPhaseRotatorCornerHz),
    pureSignalEnabled: Boolean(source.pureSignalEnabled),
    pureSignalAutoAttenuate: source.pureSignalAutoAttenuate ?? DEFAULT_RADIO_PREFS.pureSignalAutoAttenuate,
    pureSignalAttenuationDb: clampPureSignalAttenuationDb(source.pureSignalAttenuationDb),
    txMeterMode: normalizeTxMeterMode(source.txMeterMode),
    twoToneEnabled: Boolean(source.twoToneEnabled),
    txTwoToneFreq1: clampTwoToneFreqHz(source.txTwoToneFreq1, DEFAULT_RADIO_PREFS.txTwoToneFreq1),
    txTwoToneFreq2: clampTwoToneFreqHz(source.txTwoToneFreq2, DEFAULT_RADIO_PREFS.txTwoToneFreq2),
    txTwoToneLevelDb: clampTwoToneLevelDb(source.txTwoToneLevelDb),
    txTwoToneInvertLsb: source.txTwoToneInvertLsb ?? DEFAULT_RADIO_PREFS.txTwoToneInvertLsb,
    txTwoToneDelayMs: clampTwoToneDelayMs(source.txTwoToneDelayMs),
    txNoiseGateEnabled: source.txNoiseGateEnabled ?? DEFAULT_RADIO_PREFS.txNoiseGateEnabled,
    txNoiseGateThresholdDb: Math.max(-80, Math.min(0,
      Number(source.txNoiseGateThresholdDb) || DEFAULT_RADIO_PREFS.txNoiseGateThresholdDb)),
    txDexpEnabled: Boolean(source.txDexpEnabled),
    txDexpThresholdDb: clampTxDexpThresholdDb(source.txDexpThresholdDb),
    txDexpExpansionDb: clampTxDexpExpansionDb(source.txDexpExpansionDb),
    txSpeechProcessorEnabled: Boolean(source.txSpeechProcessorEnabled),
    txSpeechProcessorGainDb: clampTxSpeechProcessorGainDb(source.txSpeechProcessorGainDb),
    txCessbEnabled: Boolean(source.txCessbEnabled),
    txTimeoutEnabled: source.txTimeoutEnabled ?? DEFAULT_RADIO_PREFS.txTimeoutEnabled,
    txTimeoutSeconds: Math.max(
      15,
      Math.min(600, Math.round(safeFiniteNumber(source.txTimeoutSeconds, DEFAULT_RADIO_PREFS.txTimeoutSeconds))),
    ),
  };
}

export function normalizeDisplayPrefs(input: DeepPartial<DisplayPrefs> | null | undefined): DisplayPrefs {
  const source = input || {};
  return {
    spectrumAutoRange: source.spectrumAutoRange ?? DEFAULT_DISPLAY_PREFS.spectrumAutoRange,
    spectrumFloorDb: clampDisplayDb(source.spectrumFloorDb, DEFAULT_DISPLAY_PREFS.spectrumFloorDb),
    spectrumCeilingDb: clampDisplayDb(source.spectrumCeilingDb, DEFAULT_DISPLAY_PREFS.spectrumCeilingDb),
    waterfallAutoRange: source.waterfallAutoRange ?? DEFAULT_DISPLAY_PREFS.waterfallAutoRange,
    waterfallFloorDb: clampDisplayDb(source.waterfallFloorDb, DEFAULT_DISPLAY_PREFS.waterfallFloorDb),
    waterfallCeilingDb: clampDisplayDb(source.waterfallCeilingDb, DEFAULT_DISPLAY_PREFS.waterfallCeilingDb),
    spectrumAverage: clampSpectrumAverage(source.spectrumAverage),
    waterfallSpeed: clampWaterfallSpeed(source.waterfallSpeed),
    waterfallPalette: normalizeWaterfallPalette(source.waterfallPalette),
    spectrumPeakHold: source.spectrumPeakHold ?? DEFAULT_DISPLAY_PREFS.spectrumPeakHold,
    spectrumTraceColor: normalizeSpectrumTraceColor(source.spectrumTraceColor),
    spectrumTraceSmoothing: clampSpectrumVisualEffect(source.spectrumTraceSmoothing),
    spectrumTraceFill: clampSpectrumVisualEffect(source.spectrumTraceFill),
    spectrumPeakGlow: clampSpectrumVisualEffect(source.spectrumPeakGlow),
    spectrumGlassSheen: clampSpectrumVisualEffect(source.spectrumGlassSheen),
    waterfallContrast: clampWaterfallContrast(source.waterfallContrast),
    waterfallSmoothing: clampSpectrumVisualEffect(source.waterfallSmoothing),
    showGrid: source.showGrid ?? DEFAULT_DISPLAY_PREFS.showGrid,
    showCenterLine: source.showCenterLine ?? DEFAULT_DISPLAY_PREFS.showCenterLine,
    showBandEdges: source.showBandEdges ?? DEFAULT_DISPLAY_PREFS.showBandEdges,
    peakTuneAssistEnabled: source.peakTuneAssistEnabled ?? DEFAULT_DISPLAY_PREFS.peakTuneAssistEnabled,
  };
}

export function normalizeBandMemory(input: unknown): BandMemory {
  const output: BandMemory = {};
  if (!input || typeof input !== 'object') return output;
  const record = input as Record<string, unknown>;
  for (const [label, raw] of Object.entries(record)) {
    if (!HAM_BAND_LABELS.has(label) || !raw || typeof raw !== 'object') continue;
    const entry = raw as Record<string, unknown>;
    output[label] = {
      frequency: Math.max(0, Math.round(safeFiniteNumber(entry.frequency, 0))),
      radioPrefs: normalizeRadioPrefs(entry.radioPrefs as DeepPartial<RadioPrefs> | null | undefined),
      displayPrefs: normalizeDisplayPrefs(entry.displayPrefs as DeepPartial<DisplayPrefs> | null | undefined),
      displayZoom: Math.max(1, Math.min(32, Math.round(safeFiniteNumber(entry.displayZoom, 1)))),
    };
  }
  return output;
}

export function normalizeProfileSettings(
  input: DeepPartial<ProfileSettings> | null | undefined,
  env: WsUrlEnvironment,
): ProfileSettings {
  const source = input || {};
  return {
    wsUrl: normalizeWsUrlPreference(source.wsUrl, env),
    displayZoom: Math.max(1, Math.min(32, Math.round(safeFiniteNumber(source.displayZoom, 1)))),
    frequencyLock: Boolean(source.frequencyLock),
    keepScreenAwake: Boolean(source.keepScreenAwake),
    streamMode: normalizeStreamMode(source.streamMode),
    layoutMode: source.layoutMode === 'phone' ? 'phone' : 'desktop',
    theme: source.theme === 'light' ? 'light' : 'dark',
    phonePanels: sanitizePhonePanels(source.phonePanels),
    displayPrefs: normalizeDisplayPrefs(source.displayPrefs),
    radioPrefs: normalizeRadioPrefs(source.radioPrefs),
    bandMemory: normalizeBandMemory(source.bandMemory),
  };
}

export function normalizeRemoteSettings(
  input: DeepPartial<RemoteSettings> | null | undefined,
  env: WsUrlEnvironment,
): RemoteSettings {
  const normalized = normalizeProfileSettings(input, env);
  return {
    activeProfile: typeof input?.activeProfile === 'string' && input.activeProfile.trim().length > 0
      ? input.activeProfile.trim()
      : null,
    ...normalized,
  };
}

export function settingsStateToProfileSettings(state: SettingsState, env: WsUrlEnvironment): ProfileSettings {
  return normalizeProfileSettings({
    wsUrl: state.wsUrl,
    displayZoom: state.displayZoom,
    frequencyLock: state.frequencyLock,
    keepScreenAwake: state.keepScreenAwake,
    streamMode: state.streamMode,
    layoutMode: state.layoutMode,
    theme: state.theme,
    phonePanels: state.phonePanels,
    displayPrefs: state.displayPrefs,
    radioPrefs: state.radioPrefs,
    bandMemory: state.bandMemory,
  }, env);
}

export function settingsStateToRemoteSettings(state: SettingsState, env: WsUrlEnvironment): RemoteSettings {
  return {
    activeProfile: state.activeProfile.trim() || null,
    ...settingsStateToProfileSettings(state, env),
  };
}

export function applyRemoteSettingsToState(
  settings: DeepPartial<RemoteSettings> | null | undefined,
  state: SettingsState,
  env: WsUrlEnvironment,
): SettingsState {
  const normalized = normalizeRemoteSettings(settings, env);
  const next = createDefaultSettingsState();
  next.wsUrl = normalized.wsUrl;
  next.displayZoom = normalized.displayZoom;
  next.frequencyLock = normalized.frequencyLock;
  next.keepScreenAwake = normalized.keepScreenAwake;
  next.streamMode = normalized.streamMode;
  next.layoutMode = normalized.layoutMode;
  next.theme = normalized.theme;
  next.phonePanels = { ...normalized.phonePanels };
  next.activeProfile = normalized.activeProfile || '';
  next.radioPrefs = {
    ...normalized.radioPrefs,
    rxEqBands: normalized.radioPrefs.rxEqBands.slice(),
    txEqBands: normalized.radioPrefs.txEqBands.slice(),
    cfcBands: normalized.radioPrefs.cfcBands.slice(),
  };
  next.displayPrefs = { ...normalized.displayPrefs };
  next.bandMemory = normalizeBandMemory(normalized.bandMemory);
  Object.assign(state, next);
  return state;
}
