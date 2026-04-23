import type { AudioFrame, IqFrame } from './rx-frame';
import type { AudioApplyResult, IqApplyResult, RxTransportState } from './rx-state';

export function fftSizeForSamples(value: number, zoom: number, baseFftSize: number, maxFftSize: number): number {
  const target = Math.min(maxFftSize, baseFftSize * Math.max(1, zoom || 1));
  if (value >= target) return target;
  let size = 32;
  while ((size << 1) <= value && (size << 1) <= target) {
    size <<= 1;
  }
  return Math.max(32, size);
}

export function appendIqPacket(state: RxTransportState, iq: Float32Array): RxTransportState {
  const nextPackets = state.iqPackets.slice();
  nextPackets.push(new Float32Array(iq));
  const historyLimit = Math.min(
    state.packetHistoryMax,
    state.packetHistoryBase * Math.max(1, state.displayZoom),
  );
  while (nextPackets.length > historyLimit) {
    nextPackets.shift();
  }
  return { ...state, iqPackets: nextPackets };
}

export function buildRenderIqWindow(state: RxTransportState): Float32Array | null {
  if (state.iqPackets.length === 0) return null;
  const desiredComplexSamples = Math.min(
    state.maxFftSize,
    state.baseFftSize * Math.max(1, state.displayZoom),
  );
  const selected: Float32Array[] = [];
  let totalFloats = 0;

  for (let i = state.iqPackets.length - 1; i >= 0; i -= 1) {
    const packet = state.iqPackets[i];
    if (!packet) continue;
    selected.push(packet);
    totalFloats += packet.length;
    if ((totalFloats / 2) >= desiredComplexSamples) break;
  }

  selected.reverse();
  const combined = new Float32Array(totalFloats);
  let offset = 0;
  for (const packet of selected) {
    combined.set(packet, offset);
    offset += packet.length;
  }
  return combined;
}

export function applyIqFrame(
  state: RxTransportState,
  frame: IqFrame,
  nowMs: number,
): IqApplyResult {
  if (state.moxRequested || state.txEnabled) {
    return { state, accepted: false, sampleRateChanged: false, receivedIqLogMessage: null };
  }

  const sampleRateChanged = frame.header.sampleRate !== state.sampleRate;
  const next = appendIqPacket({
    ...state,
    sampleRate: frame.header.sampleRate,
    lastFrameAt: nowMs,
    frameCounter: state.frameCounter + 1,
    displayCaption: `Live IQ frame: ${frame.samplePairs} complex samples at ${Math.round(frame.header.sampleRate / 1000)} kHz`,
  }, frame.iq);

  const receivedIqLogMessage = nowMs - state.lastFrameAt > 2500
    ? `Received IQ frame: ${frame.samplePairs} complex samples @ ${Math.round(frame.header.sampleRate / 1000)} kHz`
    : null;

  return {
    state: next,
    accepted: true,
    sampleRateChanged,
    receivedIqLogMessage,
  };
}

export function applyAudioFrame(state: RxTransportState, frame: AudioFrame): AudioApplyResult {
  if (!state.audioStreaming) {
    return { state, accepted: false };
  }
  if (state.moxRequested || state.txEnabled) {
    return { state, accepted: false };
  }
  return {
    state: {
      ...state,
      audioSampleRate: frame.header.sampleRate,
      audioFramesPlayed: state.audioFramesPlayed + 1,
    },
    accepted: true,
  };
}
