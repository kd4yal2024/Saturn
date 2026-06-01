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
    rxFilterShiftHz: 0,
    txFilterLow: 50,
    txFilterHigh: 3050,
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
    safetyQueueDepthOverflowCount: 0,
    txUplinkDegraded: false,
    txMicDroppedCount: 0,
    txUplinkBufferedBytes: 0,
    txUplinkBufferedHwmBytes: 0,
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
    remoteClientRole: null,
    remoteClientId: null,
    txDrive: 10,
    remoteTxRfEnabled: null,
    txEnabled: false,
    moxRequested: false,
    txPhase: 'rx',
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
    txNoiseGateEnabled: true,
    txNoiseGateThresholdDb: -30,
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

  it('tracks bridge RTT from saturn pong replies', () => {
    const sentAt = Math.max(0, performance.now() - 12);
    const result = applyTciText(`saturn_pong:probe,${sentAt};`, createState());
    expect(result.state.bridgeRttMs).not.toBeNull();
    expect(result.state.bridgeRttMs ?? 0).toBeGreaterThanOrEqual(0);
    expect(result.state.bridgeRttAt).toBeGreaterThanOrEqual(sentAt);
  });

  it('tracks bridge backpressure telemetry', () => {
    const result = applyTciText(
      'remote_backpressure:0,1,2,3,4,5,6,7,8,9,10,11,12,13000,14;',
      createState(),
    );
    expect(result.state.backpressureSafetyP99Us).toBe(3);
    expect(result.state.backpressureControlP99Us).toBe(6);
    expect(result.state.displayReplacedPerSec).toBe(7);
    expect(result.state.displayDroppedPerSec).toBe(8);
    expect(result.state.bridgeAudioDroppedPerSec).toBe(9);
    expect(result.state.bridgeAudioSeqGapCount).toBe(10);
    expect(result.state.audioSeqGapCount).toBe(0);
    expect(result.state.audioPanicDrainCount).toBe(11);
    expect(result.state.sendBlockedMs).toBe(12);
    expect(result.state.outboundHighWatermarkBytes).toBe(13000);
    expect(result.state.safetyQueueDepthOverflowCount).toBe(14);
  });

  it('tracks bridge TX uplink telemetry', () => {
    const result = applyTciText(
      'remote_tx_uplink:0,true,42,32000,64000,1234,2,180,3,4,5;',
      createState(),
    );
    expect(result.state.txUplinkDegraded).toBe(true);
    expect(result.state.txMicDroppedCount).toBe(42);
    expect(result.state.txUplinkBufferedBytes).toBe(32000);
    expect(result.state.txUplinkBufferedHwmBytes).toBe(64000);
    expect(result.state.txMicLastArrivedSeq).toBe(1234);
    expect(result.state.txMicSeqGapCount).toBe(2);
    expect(result.state.txMicAgeMs).toBe(180);
    expect(result.state.txCodecDecodeErrorCount).toBe(3);
    expect(result.state.txCodecStaleDropCount).toBe(4);
    expect(result.state.txCodecReleaseFlushCount).toBe(5);
  });

  it('keeps legacy bridge TX uplink telemetry compatible', () => {
    const result = applyTciText(
      'remote_tx_uplink:0,true,42,32000,64000,1234,2,180;',
      createState(),
    );

    expect(result.state.txMicAgeMs).toBe(180);
    expect(result.state.txCodecDecodeErrorCount).toBe(0);
    expect(result.state.txCodecStaleDropCount).toBe(0);
    expect(result.state.txCodecReleaseFlushCount).toBe(0);
  });

  it('tracks Phase 44 TX codec negotiation replies', () => {
    const accepted = applyTciText('tx_codec_accept:0,pcm;', createState());
    expect(accepted.state.txCodecRequested).toBe('pcm');
    expect(accepted.state.txCodecAccepted).toBe('pcm');
    expect(accepted.state.txCodecNegotiatedAt).toBeGreaterThan(0);
    expect(accepted.state.txCodecRejectReason).toBeNull();

    const rejected = applyTciText('tx_codec_reject:0,opus_wb,unsupported;', accepted.state);
    expect(rejected.state.txCodecRequested).toBe('opus_wb');
    expect(rejected.state.txCodecAccepted).toBeNull();
    expect(rejected.state.txCodecNegotiatedAt).toBe(0);
    expect(rejected.state.txCodecRejectReason).toBe('unsupported');
  });

  it('tracks the bridge RF enable gate', () => {
    const disabled = applyTciText('remote_tx_rf_enabled:0,false;', createState());
    expect(disabled.state.remoteTxRfEnabled).toBe(false);

    const enabled = applyTciText('tx_rf_enabled:0,true;', disabled.state);
    expect(enabled.state.remoteTxRfEnabled).toBe(true);
  });

  it('tracks the backend client role', () => {
    const operator = applyTciText('remote_client_role:0,operator,42;', createState());
    expect(operator.state.remoteClientRole).toBe('operator');
    expect(operator.state.remoteClientId).toBe('42');

    const viewer = applyTciText('client_role:viewer,standby;', operator.state);
    expect(viewer.state.remoteClientRole).toBe('viewer');
    expect(viewer.state.remoteClientId).toBe('standby');
  });

  it('converts signed rx/tx passbands into UI cuts', () => {
    const result = applyTciText('modulation:0,LSB;rx_filter_band:0,-2900,-100;tx_filter_band:0,-3000,-300;', createState());
    expect(result.state.mode).toBe('LSB');
    expect(result.state.filterLow).toBe(100);
    expect(result.state.filterHigh).toBe(2900);
    expect(result.state.rxFilterShiftHz).toBe(0);
    expect(result.state.txFilterLow).toBe(300);
    expect(result.state.txFilterHigh).toBe(3000);
  });

  it('preserves shifted rx passbands separately from UI cut width', () => {
    const result = applyTciText('rx_filter_band:0,550,3550;', createState());
    expect(result.state.filterLow).toBe(50);
    expect(result.state.filterHigh).toBe(3050);
    expect(result.state.rxFilterShiftHz).toBe(500);
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

  it('keeps local mox armed when bridge reports not keyed before RF is active', () => {
    const initial = createState();
    initial.txEnabled = false;
    initial.moxRequested = true;
    const result = applyTciText('trx:0,false;', initial);
    expect(result.txReleased).toBe(false);
    expect(result.state.txEnabled).toBe(false);
    expect(result.state.moxRequested).toBe(true);
  });

  it('tx_state:armed sets moxRequested and txPhase', () => {
    const result = applyTciText('tx_state:0,armed;', createState());
    expect(result.state.txPhase).toBe('armed');
    expect(result.state.moxRequested).toBe(true);
    expect(result.state.txEnabled).toBe(false);
    expect(result.txReleased).toBe(false);
  });

  it('tx_state:keyed sets txEnabled and moxRequested', () => {
    const result = applyTciText('tx_state:0,keyed;', createState());
    expect(result.state.txPhase).toBe('keyed');
    expect(result.state.moxRequested).toBe(true);
    expect(result.state.txEnabled).toBe(true);
    expect(result.txReleased).toBe(false);
  });

  it('tx_state:rx from keyed triggers txReleased', () => {
    const initial = createState();
    initial.txPhase = 'keyed';
    initial.txEnabled = true;
    initial.moxRequested = true;
    const result = applyTciText('tx_state:0,rx;', initial);
    expect(result.state.txPhase).toBe('rx');
    expect(result.state.txEnabled).toBe(false);
    expect(result.state.moxRequested).toBe(false);
    expect(result.txReleased).toBe(true);
  });

  it('tx_state:rx from armed clears mox without txReleased', () => {
    const initial = createState();
    initial.txPhase = 'armed';
    initial.moxRequested = true;
    const result = applyTciText('tx_state:0,rx;', initial);
    expect(result.state.txPhase).toBe('rx');
    expect(result.state.moxRequested).toBe(false);
    expect(result.txReleased).toBe(false);
  });

  it('tracks bridge TX faults and forces local TX state back to RX', () => {
    const initial = createState();
    initial.txPhase = 'keyed';
    initial.txEnabled = true;
    initial.moxRequested = true;
    const result = applyTciText('tx_fault:0,power_trip,126.3,110.0;', initial);
    expect(result.txFault).toBe('Power trip 126.3 W > 110.0 W');
    expect(result.state.txFaultReason).toBe('Power trip 126.3 W > 110.0 W');
    expect(result.state.txPhase).toBe('rx');
    expect(result.state.txEnabled).toBe(false);
    expect(result.state.moxRequested).toBe(false);
    expect(result.txReleased).toBe(true);
  });

  it('describes bridge uplink-late TX faults', () => {
    const initial = createState();
    initial.txPhase = 'keyed';
    initial.txEnabled = true;
    initial.moxRequested = true;
    const result = applyTciText('tx_fault:0,uplink_late,280,250;', initial);
    expect(result.txFault).toBe('Uplink late 280 ms > 250 ms');
    expect(result.state.txFaultReason).toBe('Uplink late 280 ms > 250 ms');
    expect(result.state.txPhase).toBe('rx');
    expect(result.txReleased).toBe(true);
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
