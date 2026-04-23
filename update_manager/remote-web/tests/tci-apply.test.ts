import { describe, expect, it } from 'vitest';
import { applyTciText } from '../src/tci/apply';
import type { TciRadioState } from '../src/tci/state';

function createState(): TciRadioState {
  return {
    mode: 'USB',
    vfoA: 14200000,
    vfoB: 14200000,
    dds: 14200000,
    rxAdc: 0,
    rxAntenna: 1,
    sampleRate: 192000,
    audioSampleRate: 48000,
    rxVolumeDb: -10,
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
    meterDbm: null,
    txPower: null,
    swr: null,
    txDrive: 100,
    txEnabled: false,
    moxRequested: false,
    audioStreaming: false,
    rxEqEnabled: false,
    txEqEnabled: false,
    rxEqBands: new Array(11).fill(0),
    txEqBands: new Array(11).fill(0),
    cfcEnabled: false,
    cfcPrecomp: 0,
    cfcBands: new Array(11).fill(0),
    twoToneEnabled: false,
    txTwoToneFreq1: 700,
    txTwoToneFreq2: 1900,
    txTwoToneLevelDb: 0,
    txTwoToneInvertLsb: true,
    txTwoToneDelayMs: 0,
  };
}

describe('applyTciText', () => {
  it('marks ready and updates vfo/dds', () => {
    const result = applyTciText('ready;vfo:0,0,7100000;vfo:0,1,7200000;dds:0,7150000;', createState());
    expect(result.ready).toBe(true);
    expect(result.state.vfoA).toBe(7100000);
    expect(result.state.vfoB).toBe(7200000);
    expect(result.state.dds).toBe(7150000);
    expect(result.displayCenterHzChanged).toBe(true);
  });

  it('applies RX DSP and meter fields', () => {
    const result = applyTciText(
      'modulation:0,DIGL;rx_nr_mode:0,EMNR;rx_nb:0,2;rx_agc:0,4;rx_smeter:0,0,-91.2;tx_power:0,35;swr:0,1.8;',
      createState(),
    );
    expect(result.state.mode).toBe('DIGL');
    expect(result.state.rxNoiseReductionMode).toBe('NR2');
    expect(result.state.rxNbMode).toBe('NB2');
    expect(result.state.agcMode).toBe('FAST');
    expect(result.state.meterDbm).toBe(-91.2);
    expect(result.state.txPower).toBe(35);
    expect(result.state.swr).toBe(1.8);
  });

  it('converts signed rx/tx passbands into UI cuts', () => {
    const result = applyTciText('modulation:0,LSB;rx_filter_band:0,-2900,-100;tx_filter_band:0,-3000,-300;', createState());
    expect(result.state.mode).toBe('LSB');
    expect(result.state.filterLow).toBe(100);
    expect(result.state.filterHigh).toBe(2900);
    expect(result.state.txFilterLow).toBe(300);
    expect(result.state.txFilterHigh).toBe(3000);
  });

  it('tracks tx release and audio streaming changes', () => {
    const initial = createState();
    initial.txEnabled = true;
    initial.moxRequested = true;
    const result = applyTciText('trx:0,false;audio_start;audio_samplerate:48000;', initial);
    expect(result.txReleased).toBe(true);
    expect(result.state.txEnabled).toBe(false);
    expect(result.state.moxRequested).toBe(false);
    expect(result.state.audioStreaming).toBe(true);
    expect(result.state.audioSampleRate).toBe(48000);
  });

  it('updates eq and two-tone controls', () => {
    const result = applyTciText(
      'rx_eq_enable:1;rx_eq_band:0,3,6.7;tx_eq_enable:true;tx_eq_band:0,4,-2.2;tx_two_tone:true;tx_two_tone_freq1:0,850;tx_two_tone_level_db:0,-6;',
      createState(),
    );
    expect(result.state.rxEqEnabled).toBe(true);
    expect(result.state.rxEqBands[3]).toBe(7);
    expect(result.state.txEqEnabled).toBe(true);
    expect(result.state.txEqBands[4]).toBe(-2);
    expect(result.state.twoToneEnabled).toBe(true);
    expect(result.state.txTwoToneFreq1).toBe(850);
    expect(result.state.txTwoToneLevelDb).toBe(-6);
  });
});
