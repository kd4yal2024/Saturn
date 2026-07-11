import { describe, it, expect } from 'vitest';
import {
  buildRxFilterBandCommand,
  buildTxFilterBandCommand,
  buildAnrCommands,
  buildAnfCommands,
  buildTwoToneCommands,
  buildAllRadioPrefsCommands,
  buildTxCodecCapsCommand,
} from '../src/tci/commands';
import { normalizeRadioPrefs } from '../src/settings/normalize';

describe('buildRxFilterBandCommand', () => {
  it('returns positive passband for USB', () => {
    expect(buildRxFilterBandCommand(50, 3050, 'USB')).toBe('rx_filter_band:0,50,3050;');
  });

  it('returns negative passband for LSB', () => {
    expect(buildRxFilterBandCommand(50, 3050, 'LSB')).toBe('rx_filter_band:0,-3050,-50;');
  });

  it('returns symmetric passband for AM', () => {
    expect(buildRxFilterBandCommand(50, 3050, 'AM')).toBe('rx_filter_band:0,-3050,3050;');
  });

  it('applies rx filter shift for sideband modes', () => {
    expect(buildRxFilterBandCommand(50, 3050, 'USB', 500)).toBe('rx_filter_band:0,550,3550;');
    expect(buildRxFilterBandCommand(300, 3000, 'LSB', -200)).toBe('rx_filter_band:0,-3200,-500;');
  });

  it('uses the fixed broadcast channel width for WFM', () => {
    expect(buildRxFilterBandCommand(50, 3050, 'WFM')).toBe('rx_filter_band:0,-90000,90000;');
  });
});

describe('buildTxFilterBandCommand', () => {
  it('returns tx filter for USB', () => {
    expect(buildTxFilterBandCommand(100, 2900, 'USB')).toBe('tx_filter_band:0,100,2900;');
  });
});

describe('buildTxCodecCapsCommand', () => {
  it('advertises PCM by default for the Phase 44 scaffold', () => {
    expect(buildTxCodecCapsCommand()).toBe('tx_codec_caps:0,pcm;');
  });

  it('deduplicates known codec capabilities', () => {
    expect(buildTxCodecCapsCommand(['pcm', 'opus_wb', 'pcm'])).toBe('tx_codec_caps:0,pcm,opus_wb;');
  });
});

describe('buildAnrCommands', () => {
  it('returns 4 commands', () => {
    const cmds = buildAnrCommands({ rxAnrTaps: 64, rxAnrDelay: 16, rxAnrGain: 0.0002, rxAnrLeakage: 0.00005 });
    expect(cmds).toHaveLength(4);
    expect(cmds[0]).toBe('rx_anr_taps:0,64;');
    expect(cmds[1]).toBe('rx_anr_delay:0,16;');
    expect(cmds[2]).toMatch(/^rx_anr_gain:0,0\.000200;$/);
    expect(cmds[3]).toMatch(/^rx_anr_leakage:0,0\.000050;$/);
  });
});

describe('buildAnfCommands', () => {
  it('returns 5 commands', () => {
    const cmds = buildAnfCommands({
      anfEnabled: true, rxAnfTaps: 64, rxAnfDelay: 16, rxAnfGain: 0.00012, rxAnfLeakage: 0.00008,
    });
    expect(cmds).toHaveLength(5);
    expect(cmds[0]).toBe('rx_anf:0,true;');
  });
});

describe('buildTwoToneCommands', () => {
  it('returns 5 commands', () => {
    const cmds = buildTwoToneCommands({
      txTwoToneFreq1: 700, txTwoToneFreq2: 1900, txTwoToneLevelDb: 0, txTwoToneInvertLsb: true, txTwoToneDelayMs: 0,
    });
    expect(cmds).toHaveLength(5);
    expect(cmds[0]).toBe('tx_two_tone_freq1:0,700;');
  });
});

describe('buildAllRadioPrefsCommands', () => {
  it('returns all commands for default prefs', () => {
    const prefs = normalizeRadioPrefs({});
    const cmds = buildAllRadioPrefsCommands(prefs);
    expect(cmds.length).toBeGreaterThan(30);
    expect(cmds[0]).toMatch(/^iq_samplerate:/);
    expect(cmds.some((c) => c.startsWith('modulation:'))).toBe(true);
    expect(cmds.some((c) => c.startsWith('rx_filter_band:'))).toBe(true);
    expect(cmds.some((c) => c.startsWith('tx_filter_band:'))).toBe(true);
    expect(cmds.some((c) => c.startsWith('rx_eq_band:'))).toBe(true);
    expect(cmds.some((c) => c.startsWith('tx_cfc_band:'))).toBe(true);
    expect(cmds).toContain('rx_nr2_gain_method:0,GAMMA;');
    expect(cmds).toContain('rx_nr2_npe_method:0,OSMS;');
    expect(cmds).toContain('rx_nr2_post_filter:0,true;');
    expect(cmds).toContain('rx_wbfm_deemphasis:0,NA_75US;');
    expect(cmds).toContain('tx_phase_rotator:0,false;');
    expect(cmds).toContain('tx_phase_rotator_auto:0,false;');
    expect(cmds).toContain('tx_phase_rotator_corner:0,338;');
    expect(cmds).toContain('tx_puresignal:0,false;');
    expect(cmds).toContain('tx_puresignal_auto_attenuate:0,true;');
    expect(cmds).toContain('tx_puresignal_attenuation:0,0;');
    expect(cmds.some((c) => c.startsWith('tx_two_tone:'))).toBe(true);
  });

  it('every command ends with semicolon', () => {
    const prefs = normalizeRadioPrefs({});
    const cmds = buildAllRadioPrefsCommands(prefs);
    for (const cmd of cmds) {
      expect(cmd.endsWith(';')).toBe(true);
    }
  });
});
