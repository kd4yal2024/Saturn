import { describe, expect, it } from 'vitest';
import { createDefaultSettingsState } from '../src/settings/defaults';
import {
  applyRemoteSettingsToState,
  normalizeProfileSettings,
  normalizeRadioPrefs,
  normalizeRemoteSettings,
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
      rxNbMode: '2',
      agcMode: '4',
      txDrive: 120,
      txMicGainDb: 99,
      rxEqBands: [5, 99, -99],
      cfcBands: [5, 40],
    } as any);

    expect(prefs.sampleRate).toBe(192000);
    expect(prefs.rxAdc).toBe(2);
    expect(prefs.rxAntenna).toBe(1);
    expect(prefs.mode).toBe('USB');
    expect(prefs.rxNoiseReductionMode).toBe('NR2');
    expect(prefs.rxNbMode).toBe('NB2');
    expect(prefs.agcMode).toBe('FAST');
    expect(prefs.txDrive).toBe(100);
    expect(prefs.txMicGainDb).toBe(20);
    expect(prefs.rxEqBands[1]).toBe(20);
    expect(prefs.cfcBands[1]).toBe(20);
  });

  it('normalizes profile and remote settings', () => {
    const profile = normalizeProfileSettings({
      wsUrl: '',
      displayZoom: 99,
      layoutMode: 'phone',
      theme: 'light',
      phonePanels: { radio: true, setup: false },
    }, browserEnv);

    expect(profile.wsUrl).toBe('wss://radio.local:8443/tci');
    expect(profile.displayZoom).toBe(32);
    expect(profile.layoutMode).toBe('phone');
    expect(profile.theme).toBe('light');
    expect(profile.phonePanels.radio).toBe(true);
    expect(profile.phonePanels.setup).toBe(false);

    const remote = normalizeRemoteSettings({
      activeProfile: '  Field Day  ',
      wsUrl: 'ws://bad-host:50001',
    }, browserEnv);
    expect(remote.activeProfile).toBe('Field Day');
    expect(remote.wsUrl).toBe('wss://radio.local:8443/tci');
  });

  it('applies remote settings into state and round-trips them', () => {
    const state = createDefaultSettingsState();
    applyRemoteSettingsToState({
      activeProfile: 'Portable',
      layoutMode: 'phone',
      displayZoom: 4,
      phonePanels: { radio: true, tx: false },
      radioPrefs: { mode: 'DIGL', txDrive: 65 },
    }, state, browserEnv);

    expect(state.activeProfile).toBe('Portable');
    expect(state.layoutMode).toBe('phone');
    expect(state.displayZoom).toBe(4);
    expect(state.phonePanels.radio).toBe(true);
    expect(state.phonePanels.tx).toBe(false);
    expect(state.radioPrefs.mode).toBe('DIGL');
    expect(state.radioPrefs.txDrive).toBe(65);

    const remote = settingsStateToRemoteSettings(state, browserEnv);
    expect(remote.activeProfile).toBe('Portable');
    expect(remote.layoutMode).toBe('phone');
    expect(remote.displayZoom).toBe(4);
    expect(remote.radioPrefs.mode).toBe('DIGL');
  });
});
