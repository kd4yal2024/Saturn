import { framePayloadFloats, parseTciBinaryFrameHeader, TciStreamType, type TciBinaryFrameHeader } from './tci-frame';

export type IqFrame = {
  kind: 'iq';
  header: TciBinaryFrameHeader;
  iq: Float32Array;
  samplePairs: number;
};

export type AudioFrame = {
  kind: 'audio';
  header: TciBinaryFrameHeader;
  frames: number;
  left: Float32Array;
  right: Float32Array;
  mirroredMono: boolean;
};

export type DecodedRxFrame = IqFrame | AudioFrame;

export function classifyFrame(header: TciBinaryFrameHeader): 'audio' | 'iq' {
  return header.frameType === TciStreamType.AudioLeft ? 'audio' : 'iq';
}

export function decodeIqFrame(buffer: ArrayBuffer): IqFrame | null {
  const header = parseTciBinaryFrameHeader(buffer);
  if (!header.floatCount) return null;
  const iq = framePayloadFloats(buffer, header.floatCount);
  return {
    kind: 'iq',
    header,
    iq,
    samplePairs: Math.floor(iq.length / 2),
  };
}

export function decodeAudioFrame(buffer: ArrayBuffer): AudioFrame | null {
  const header = parseTciBinaryFrameHeader(buffer);
  if (!header.floatCount) return null;

  const channels = Math.max(1, header.channels);
  const floats = framePayloadFloats(buffer, header.floatCount);
  const frames = Math.floor(floats.length / channels);
  if (frames < 1) return null;

  const left = new Float32Array(frames);
  const right = new Float32Array(frames);

  let mirroredMono = channels < 2;
  if (!mirroredMono && channels >= 2) {
    mirroredMono = true;
    const probeFrames = Math.min(frames, 128);
    for (let i = 0; i < probeFrames; i += 1) {
      if (Math.abs(floats[i * channels + 1] ?? 0) > 1e-6) {
        mirroredMono = false;
        break;
      }
    }
  }

  for (let i = 0; i < frames; i += 1) {
    const leftSample = floats[i * channels] ?? 0;
    const rightSample = mirroredMono
      ? leftSample
      : (channels > 1 ? (floats[i * channels + 1] ?? leftSample) : leftSample);
    left[i] = leftSample;
    right[i] = rightSample;
  }

  return {
    kind: 'audio',
    header,
    frames,
    left,
    right,
    mirroredMono,
  };
}

export function decodeRxFrame(buffer: ArrayBuffer): DecodedRxFrame | null {
  const header = parseTciBinaryFrameHeader(buffer);
  return classifyFrame(header) === 'audio' ? decodeAudioFrame(buffer) : decodeIqFrame(buffer);
}
