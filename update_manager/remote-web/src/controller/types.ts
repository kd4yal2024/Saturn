import type { SettingsState, WsUrlEnvironment } from '../settings/types';
import type { TciRadioState } from '../tci/state';
import type { RxTransportState } from '../transport/rx-state';

export type RemoteControllerState = {
  settings: SettingsState;
  radio: TciRadioState;
  transport: RxTransportState;
};

export type RemoteControllerEvents = {
  ready: boolean;
  displayCenterHzChanged: boolean;
  sampleRateChanged: boolean;
  rxVolumeChanged: boolean;
  txReleased: boolean;
  receivedIqLogMessage: string | null;
};

export type RemoteController = {
  readonly env: WsUrlEnvironment;
  getState: () => RemoteControllerState;
  applyRemoteSettings: (input: unknown) => RemoteControllerState;
  applyTciText: (text: string) => { state: RemoteControllerState; events: RemoteControllerEvents };
  applyBinaryFrame: (buffer: ArrayBuffer, nowMs: number) => { state: RemoteControllerState; accepted: boolean; kind: 'iq' | 'audio' | null; receivedIqLogMessage: string | null };
};
