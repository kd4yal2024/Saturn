import { createDefaultSettingsState } from '../settings/defaults';
import { applyRemoteSettingsToState } from '../settings/normalize';
import type { SettingsState, WsUrlEnvironment } from '../settings/types';
import { applyTciText as applyTciTextToRadio } from '../tci/apply';
import type { TciRadioState } from '../tci/state';
import { applyAudioFrame, applyIqFrame } from '../transport/rx-apply';
import { decodeRxFrame } from '../transport/rx-frame';
import type { RxTransportState } from '../transport/rx-state';
import type { RemoteController, RemoteControllerEvents, RemoteControllerState } from './types';

function createInitialRadioState(settings: SettingsState): TciRadioState {
  return {
    mode: settings.radioPrefs.mode,
    vfoA: 14200000,
    vfoB: 14200000,
    activeVfo: 'A',
    splitEnabled: false,
    dds: 14200000,
    rxAdc: settings.radioPrefs.rxAdc,
    rxAntenna: settings.radioPrefs.rxAntenna,
    rxAttenuationDb: 0,
    sampleRate: settings.radioPrefs.sampleRate,
    audioSampleRate: 48000,
    audioChannels: 2,
    rxVolumeDb: settings.radioPrefs.rxVolumeDb,
    rxSsqlEnabled: false,
    rxSsqlThreshold: 16,
    rxNoiseReductionMode: settings.radioPrefs.rxNoiseReductionMode,
    rxNoiseReductionLevel: settings.radioPrefs.rxNoiseReductionLevel,
    rxNr2GainMethod: settings.radioPrefs.rxNr2GainMethod,
    rxNr2NpeMethod: settings.radioPrefs.rxNr2NpeMethod,
    rxNr2PostFilterEnabled: settings.radioPrefs.rxNr2PostFilterEnabled,
    rxWbfmSupported: false,
    rxWbfmDeemphasis: settings.radioPrefs.rxWbfmDeemphasis,
    rxWbfmStereoDetected: false,
    rxNbMode: settings.radioPrefs.rxNbMode,
    rxNbThreshold: settings.radioPrefs.rxNbThreshold,
    rxAnrTaps: settings.radioPrefs.rxAnrTaps,
    rxAnrDelay: settings.radioPrefs.rxAnrDelay,
    rxAnrGain: settings.radioPrefs.rxAnrGain,
    rxAnrLeakage: settings.radioPrefs.rxAnrLeakage,
    anfEnabled: settings.radioPrefs.anfEnabled,
    rxAnfTaps: settings.radioPrefs.rxAnfTaps,
    rxAnfDelay: settings.radioPrefs.rxAnfDelay,
    rxAnfGain: settings.radioPrefs.rxAnfGain,
    rxAnfLeakage: settings.radioPrefs.rxAnfLeakage,
    agcMode: settings.radioPrefs.agcMode,
    agcGain: settings.radioPrefs.agcGain,
    filterLow: settings.radioPrefs.filterLow,
    filterHigh: settings.radioPrefs.filterHigh,
    rxFilterShiftHz: 0,
    txFilterLow: settings.radioPrefs.txFilterLow,
    txFilterHigh: settings.radioPrefs.txFilterHigh,
    meterDbm: null,
    txPower: null,
    txReflectedPower: null,
    swr: null,
    txAlcPeakDb: null,
    txAlcAvgDb: null,
    txAlcGainDb: null,
    txCompPeakDb: null,
    txCompAvgDb: null,
    txMicPeakDb: null,
    adcOverflowMask: 0,
    adc1Peak: 0,
    adc2Peak: 0,
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
    txDrive: settings.radioPrefs.txDrive,
    remoteTxRfEnabled: null,
    txEnabled: false,
    moxRequested: false,
    txPhase: 'rx',
    audioStreaming: false,
    rxEqEnabled: settings.radioPrefs.rxEqEnabled,
    txEqEnabled: settings.radioPrefs.txEqEnabled,
    rxEqBands: settings.radioPrefs.rxEqBands.slice(),
    txEqBands: settings.radioPrefs.txEqBands.slice(),
    cfcEnabled: settings.radioPrefs.cfcEnabled,
    cfcPrecomp: settings.radioPrefs.cfcPrecomp,
    cfcBands: settings.radioPrefs.cfcBands.slice(),
    txPhaseRotatorEnabled: settings.radioPrefs.txPhaseRotatorEnabled,
    txPhaseRotatorAuto: settings.radioPrefs.txPhaseRotatorAuto,
    txPhaseRotatorCornerHz: settings.radioPrefs.txPhaseRotatorCornerHz,
    pureSignalEnabled: settings.radioPrefs.pureSignalEnabled,
    pureSignalAutoAttenuate: settings.radioPrefs.pureSignalAutoAttenuate,
    pureSignalAttenuationDb: settings.radioPrefs.pureSignalAttenuationDb,
    pureSignalState: 'off',
    pureSignalFeedbackLevel: 0,
    pureSignalCalibrationCount: 0,
    pureSignalCorrecting: false,
    pureSignalMaxTx: 0,
    pureSignalFeedbackPackets: 0,
    pureSignalFeedbackGaps: 0,
    twoToneEnabled: settings.radioPrefs.twoToneEnabled,
    txTwoToneFreq1: settings.radioPrefs.txTwoToneFreq1,
    txTwoToneFreq2: settings.radioPrefs.txTwoToneFreq2,
    txTwoToneLevelDb: settings.radioPrefs.txTwoToneLevelDb,
    txTwoToneInvertLsb: settings.radioPrefs.txTwoToneInvertLsb,
    txTwoToneDelayMs: settings.radioPrefs.txTwoToneDelayMs,
    txNoiseGateEnabled: settings.radioPrefs.txNoiseGateEnabled,
    txNoiseGateThresholdDb: settings.radioPrefs.txNoiseGateThresholdDb,
    txDexpEnabled: settings.radioPrefs.txDexpEnabled,
    txDexpThresholdDb: settings.radioPrefs.txDexpThresholdDb,
    txDexpExpansionDb: settings.radioPrefs.txDexpExpansionDb,
    txSpeechProcessorEnabled: settings.radioPrefs.txSpeechProcessorEnabled,
    txSpeechProcessorGainDb: settings.radioPrefs.txSpeechProcessorGainDb,
    txCessbEnabled: settings.radioPrefs.txCessbEnabled,
  };
}

function createInitialTransportState(settings: SettingsState, radio: TciRadioState): RxTransportState {
  return {
    sampleRate: radio.sampleRate,
    displayZoom: settings.displayZoom,
    iqPackets: [],
    maxFftSize: 262144,
    baseFftSize: 4096,
    packetHistoryBase: 48,
    packetHistoryMax: 1024,
    moxRequested: radio.moxRequested,
    txEnabled: radio.txEnabled,
    audioStreaming: radio.audioStreaming,
    audioSampleRate: radio.audioSampleRate,
    audioFramesPlayed: 0,
    lastFrameAt: 0,
    frameCounter: 0,
    displayCaption: 'Waiting for TCI IQ frames from saturn-bridge',
  };
}

function syncFromRadio(state: RemoteControllerState): RemoteControllerState {
  state.transport.sampleRate = state.radio.sampleRate;
  state.transport.moxRequested = state.radio.moxRequested;
  state.transport.txEnabled = state.radio.txEnabled;
  state.transport.audioStreaming = state.radio.audioStreaming;
  state.transport.audioSampleRate = state.radio.audioSampleRate;
  state.transport.displayZoom = state.settings.displayZoom;
  return state;
}

export function createRemoteController(env: WsUrlEnvironment): RemoteController {
  const settings = createDefaultSettingsState();
  let radio = createInitialRadioState(settings);
  let transport = createInitialTransportState(settings, radio);

  const getState = (): RemoteControllerState => ({
    settings,
    radio,
    transport,
  });

  return {
    env,
    getState,
    applyRemoteSettings(input) {
      applyRemoteSettingsToState(input as Record<string, unknown>, settings, env);
      radio = createInitialRadioState(settings);
      transport = createInitialTransportState(settings, radio);
      return getState();
    },
    applyTciText(text) {
      const applied = applyTciTextToRadio(text, radio);
      radio = applied.state;
      const state = syncFromRadio(getState());
      const events: RemoteControllerEvents = {
        ready: applied.ready,
        displayCenterHzChanged: applied.displayCenterHzChanged,
        sampleRateChanged: applied.sampleRateChanged,
        rxVolumeChanged: applied.rxVolumeChanged,
        txReleased: applied.txReleased,
        txFault: applied.txFault,
        receivedIqLogMessage: null,
      };
      return { state, events };
    },
    applyBinaryFrame(buffer, nowMs) {
      const decoded = decodeRxFrame(buffer);
      if (!decoded) {
        return { state: getState(), accepted: false, kind: null, receivedIqLogMessage: null };
      }

      if (decoded.kind === 'iq') {
        const applied = applyIqFrame(transport, decoded, nowMs);
        transport = applied.state;
        return {
          state: getState(),
          accepted: applied.accepted,
          kind: 'iq',
          receivedIqLogMessage: applied.receivedIqLogMessage,
        };
      }

      const applied = applyAudioFrame(transport, decoded);
      transport = applied.state;
      return {
        state: getState(),
        accepted: applied.accepted,
        kind: 'audio',
        receivedIqLogMessage: null,
      };
    },
  };
}
