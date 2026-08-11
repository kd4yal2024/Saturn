import { describe, expect, it } from 'vitest';
import {
  buildTxAudioProfileCommands,
  voodoo38kTxAudioProfile,
} from '../src/audio/tx-audio-profile';

describe('Voodoo 3.8k TX audio profile', () => {
  it('uses a true 3.8 kHz warm ESSB passband with restrained gain', () => {
    const profile = voodoo38kTxAudioProfile();
    expect(profile.label).toBe('Voodoo 3.8k');
    expect(profile.filterHighHz - profile.filterLowHz).toBe(3_800);
    expect(profile.eqEnabled).toBe(true);
    expect(profile.cfcEnabled).toBe(true);
    expect(profile.noiseGateEnabled).toBe(false);
    expect(Math.max(...profile.eqBands.map((gain) => Math.abs(gain)))).toBeLessThanOrEqual(4);
    expect(Math.max(...profile.cfcBands)).toBeLessThanOrEqual(2);
  });

  it('builds sideband-correct commands without radio-control fields', () => {
    const profile = voodoo38kTxAudioProfile();
    const usb = buildTxAudioProfileCommands(profile, 'USB').join('');
    const lsb = buildTxAudioProfileCommands(profile, 'LSB').join('');
    expect(usb).toContain('tx_filter_band:0,50,3850;');
    expect(lsb).toContain('tx_filter_band:0,-3850,-50;');
    expect(usb).toContain('tx_eq_band:0,3,4;');
    expect(usb).toContain('tx_cfc_precomp:0,1.0;');
    expect(usb).not.toMatch(/(?:trx|tx_drive|vfo|dds):/);
  });

  it('returns independent band arrays for application to mutable state', () => {
    const first = voodoo38kTxAudioProfile();
    const second = voodoo38kTxAudioProfile();
    expect(first.eqBands).not.toBe(second.eqBands);
    expect(first.cfcBands).not.toBe(second.cfcBands);
  });
});
