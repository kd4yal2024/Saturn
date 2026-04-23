export type RxTransportState = {
  sampleRate: number;
  displayZoom: number;
  iqPackets: Float32Array[];
  maxFftSize: number;
  baseFftSize: number;
  packetHistoryBase: number;
  packetHistoryMax: number;
  moxRequested: boolean;
  txEnabled: boolean;
  audioStreaming: boolean;
  audioSampleRate: number;
  audioFramesPlayed: number;
  lastFrameAt: number;
  frameCounter: number;
  displayCaption: string;
};

export type IqApplyResult = {
  state: RxTransportState;
  accepted: boolean;
  sampleRateChanged: boolean;
  receivedIqLogMessage: string | null;
};

export type AudioApplyResult = {
  state: RxTransportState;
  accepted: boolean;
};
