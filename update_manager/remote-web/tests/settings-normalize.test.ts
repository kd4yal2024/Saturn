import { describe, expect, it } from 'vitest';
import { createDefaultSettingsState } from '../src/settings/defaults';
import {
  applyRemoteSettingsToState,
  normalizeBandMemory,
  normalizeDisplayPrefs,
  normalizeProfileSettings,
  normalizeRadioPrefs,
  normalizeRemoteSettings,
  normalizeStreamMode,
  normalizeWsUrlPreference,
  sanitizePhonePanels,
  settingsStateToRemoteSettings,
} from '../src/settings/normalize';

const browserEnv = {
  protocol: 'https:',
  host: 'radio.local:8443',
  hostname: 'radio.local',
  href: 'https://radio.local:8443/remote',
};

describe('settings normalization', () => {
  it('defaults TX mic gain to a phone-safe -12 dB', () => {
    expect(createDefaultSettingsState().radioPrefs.txMicGainDb).toBe(-12);
  });

  it('sanitizes phone panel state', () => {
    expect(sanitizePhonePanels({ radio: true, tx: false, nope: true })).toEqual({
      radio: true,
      dsp: true,
      tx: false,
      meters: false,
      setup: true,
      logs: true,
    });
  });

  it('forces same-origin ws URLs for browser remote use', () => {
    expect(normalizeWsUrlPreference('ws://elsewhere:50001', browserEnv)).toBe('wss://radio.local:8443/tci');
    expect(normalizeWsUrlPreference('wss://radio.local:8443/tci', browserEnv)).toBe('wss://radio.local:8443/tci');
  });

  it('clamps and normalizes radio preferences', () => {
    const prefs = normalizeRadioPrefs({
      sampleRate: 123456,
      rxAdc: 7,
      rxAntenna: -1,
      mode: 'bad',
      rxNoiseReductionMode: 'emnr',
      rxNr2GainMethod: 'trained',
      rxNr2NpeMethod: 'nstat',
      rxNr2PostFilterEnabled: false,
      rxWbfmDeemphasis: 'europe',
      rxNbMode: '2',
      agcMode: '4',
      txDrive: 200,
      txMicGainDb: 99,
      rxEqBands: [5, 99, -99],
      cfcBands: [5, 40],
      txPhaseRotatorEnabled: true,
      txPhaseRotatorAuto: true,
      txPhaseRotatorCornerHz: 9000,
      pureSignalEnabled: true,
      pureSignalAutoAttenuate: false,
      pureSignalAttenuationDb: 99,
    } as any);

    expect(prefs.sampleRate).toBe(192000);
    expect(prefs.rxAdc).toBe(2);
    expect(prefs.rxAntenna).toBe(1);
    expect(prefs.mode).toBe('USB');
    expect(prefs.rxNoiseReductionMode).toBe('NR2');
    expect(prefs.rxNr2GainMethod).toBe('TRAINED');
    expect(prefs.rxNr2NpeMethod).toBe('NSTAT');
    expect(prefs.rxNr2PostFilterEnabled).toBe(false);
    expect(prefs.rxWbfmDeemphasis).toBe('EU_50US');
    expect(prefs.rxNbMode).toBe('NB2');
    expect(prefs.agcMode).toBe('FAST');
    expect(prefs.txDrive).toBe(100);
    expect(prefs.txMicGainDb).toBe(20);
    expect(prefs.rxEqBands[1]).toBe(20);
    expect(prefs.cfcBands[1]).toBe(20);
    expect(prefs.txPhaseRotatorEnabled).toBe(true);
    expect(prefs.txPhaseRotatorAuto).toBe(true);
    expect(prefs.txPhaseRotatorCornerHz).toBe(2000);
    expect(prefs.pureSignalEnabled).toBe(true);
    expect(prefs.pureSignalAutoAttenuate).toBe(false);
    expect(prefs.pureSignalAttenuationDb).toBe(31);
  });

  it('normalizes stream mode as an explicit LAN/WAN preference', () => {
    expect(normalizeStreamMode('wan')).toBe('wan');
    expect(normalizeStreamMode('WAN')).toBe('wan');
    expect(normalizeStreamMode('raw')).toBe('lan');
    expect(normalizeStreamMode(null)).toBe('lan');
  });

  it('normalizes panadapter appearance preferences', () => {
    const prefs = normalizeDisplayPrefs({
      spectrumTraceColor: '#A1B2C3',
      spectrumTraceSmoothing: 140,
      spectrumTraceFill: 68.4,
      spectrumPeakGlow: -12,
      spectrumGlassSheen: 42.6,
    });
    expect(prefs.spectrumTraceColor).toBe('#a1b2c3');
    expect(prefs.spectrumTraceSmoothing).toBe(100);
    expect(prefs.spectrumTraceFill).toBe(68);
    expect(prefs.spectrumPeakGlow).toBe(0);
    expect(prefs.spectrumGlassSheen).toBe(43);

    expect(normalizeDisplayPrefs({ spectrumTraceColor: 'not-a-color' }).spectrumTraceColor).toBe('#62d0ff');
  });

  it('normalizes profile and remote settings', () => {
    const profile = normalizeProfileSettings({
      wsUrl: '',
      displayZoom: 99,
      layoutMode: 'phone',
      theme: 'light',
      streamMode: 'wan',
      phonePanels: { radio: true, setup: false },
    }, browserEnv);

    expect(profile.wsUrl).toBe('wss://radio.local:8443/tci');
    expect(profile.displayZoom).toBe(32);
    expect(profile.layoutMode).toBe('phone');
    expect(profile.theme).toBe('light');
    expect(profile.streamMode).toBe('wan');
    expect(profile.phonePanels.radio).toBe(true);
    expect(profile.phonePanels.setup).toBe(false);

    const remote = normalizeRemoteSettings({
      activeProfile: '  Field Day  ',
      wsUrl: 'ws://bad-host:50001',
    }, browserEnv);
    expect(remote.activeProfile).toBe('Field Day');
    expect(remote.wsUrl).toBe('wss://radio.local:8443/tci');
  });

  it('normalizes per-band memory and drops unknown bands', () => {
    const memory = normalizeBandMemory({
      '40m': {
        displayZoom: 12,
        radioPrefs: {
          mode: 'LSB',
          agcMode: 'fast',
          filterHigh: 2400,
        },
        displayPrefs: {
          waterfallPalette: 'ice',
          showBandEdges: false,
        },
      },
      bad: {
        displayZoom: 2,
      },
    });

    expect(Object.keys(memory)).toEqual(['40m']);
    const forty = memory['40m'];
    expect(forty).toBeDefined();
    expect(forty?.displayZoom).toBe(12);
    expect(forty?.radioPrefs.mode).toBe('LSB');
    expect(forty?.radioPrefs.agcMode).toBe('FAST');
    expect(forty?.radioPrefs.filterHigh).toBe(2400);
    expect(forty?.displayPrefs.waterfallPalette).toBe('ice');
    expect(forty?.displayPrefs.showBandEdges).toBe(false);
  });

  it('retains FM broadcast band memory', () => {
    const memory = normalizeBandMemory({
      FM: {
        frequency: 99_500_000,
        radioPrefs: { mode: 'WFM', rxWbfmDeemphasis: 'NA_75US' },
      },
    });
    expect(memory.FM?.frequency).toBe(99_500_000);
    expect(memory.FM?.radioPrefs.mode).toBe('WFM');
    expect(memory.FM?.radioPrefs.rxWbfmDeemphasis).toBe('NA_75US');
  });

  it('applies remote settings into state and round-trips them', () => {
    const state = createDefaultSettingsState();
    applyRemoteSettingsToState({
      activeProfile: 'Portable',
      layoutMode: 'phone',
      displayZoom: 4,
      streamMode: 'wan',
      phonePanels: { radio: true, tx: false },
      radioPrefs: { mode: 'DIGL', txDrive: 65 },
      bandMemory: {
        '20m': {
          displayZoom: 6,
          radioPrefs: { mode: 'USB', agcMode: 'SLOW' },
          displayPrefs: { waterfallPalette: 'ember' },
        },
      },
    }, state, browserEnv);

    expect(state.activeProfile).toBe('Portable');
    expect(state.layoutMode).toBe('phone');
    expect(state.streamMode).toBe('wan');
    expect(state.displayZoom).toBe(4);
    expect(state.phonePanels.radio).toBe(true);
    expect(state.phonePanels.tx).toBe(false);
    expect(state.radioPrefs.mode).toBe('DIGL');
    expect(state.radioPrefs.txDrive).toBe(65);
    expect(state.bandMemory['20m']?.displayZoom).toBe(6);

    const remote = settingsStateToRemoteSettings(state, browserEnv);
    expect(remote.activeProfile).toBe('Portable');
    expect(remote.layoutMode).toBe('phone');
    expect(remote.streamMode).toBe('wan');
    expect(remote.displayZoom).toBe(4);
    expect(remote.radioPrefs.mode).toBe('DIGL');
    expect(remote.bandMemory['20m']?.radioPrefs.agcMode).toBe('SLOW');
  });
});
