import { TCI_FRAME_HEADER_BYTES, TciStreamType } from '../transport/tci-frame';

export const RX_OPUS_SAMPLE_RATE = 48_000;
export const RX_OPUS_FRAME_DURATION_US = 20_000;
export type RxAudioCodec = 'pcm' | 'opus';

export type RxOpusPacket = {
  sampleRate: number;
  channels: 1 | 2;
  sequence: number;
  payload: Uint8Array;
  frames: number;
};

// Type 17 reuses the TCI header, but offset 20 counts bytes instead of floats.
export function parseRxOpusPacket(buffer: ArrayBuffer): RxOpusPacket | null {
  if (buffer.byteLength < TCI_FRAME_HEADER_BYTES) return null;
  const view = new DataView(buffer);
  const sampleRate = view.getUint32(4, true);
  const durationMs = view.getUint32(8, true);
  const flags = view.getUint32(12, true);
  const samplesPerChannel = view.getUint32(16, true);
  const payloadBytes = view.getUint32(20, true);
  const frameType = view.getUint32(24, true);
  const channels = view.getUint32(28, true);
  if (frameType !== TciStreamType.AudioOpus || sampleRate !== RX_OPUS_SAMPLE_RATE ||
      durationMs !== 20 || flags !== 0 || samplesPerChannel !== 960 ||
      (channels !== 1 && channels !== 2) || payloadBytes < 1 || payloadBytes > 4096 ||
      buffer.byteLength !== TCI_FRAME_HEADER_BYTES + payloadBytes) return null;
  return {
    sampleRate,
    channels,
    sequence: view.getUint32(32, true),
    payload: new Uint8Array(buffer, TCI_FRAME_HEADER_BYTES, payloadBytes),
    frames: samplesPerChannel,
  };
}

type DecoderProbe = {
  isConfigSupported(config: {codec: string; sampleRate: number; numberOfChannels: number}): Promise<{supported: boolean}>;
};

export async function probeRxOpusDecoder(decoder: DecoderProbe | null | undefined): Promise<boolean> {
  if (!decoder || typeof decoder.isConfigSupported !== 'function') return false;
  try {
    for (const numberOfChannels of [1, 2]) {
      const result = await decoder.isConfigSupported({
        codec: 'opus', sampleRate: RX_OPUS_SAMPLE_RATE, numberOfChannels,
      });
      if (!result.supported) return false;
    }
    return true;
  } catch {
    return false;
  }
}

export type RxAudioCodecSession = {
  readonly accepted: RxAudioCodec;
  readonly requested: RxAudioCodec;
  readonly failed: boolean;
  requestOpus(): boolean;
  acceptEcho(codec: RxAudioCodec): boolean;
  fallback(): boolean;
  reset(): void;
};

export function createRxAudioCodecSession(): RxAudioCodecSession {
  let accepted: RxAudioCodec = 'pcm';
  let requested: RxAudioCodec = 'pcm';
  let failed = false;
  return {
    get accepted() { return accepted; },
    get requested() { return requested; },
    get failed() { return failed; },
    requestOpus() {
      if (failed) return false;
      requested = 'opus';
      return true;
    },
    acceptEcho(codec) {
      if (codec === 'opus' && (failed || requested !== 'opus')) return false;
      const changed = accepted !== codec;
      accepted = codec;
      return changed;
    },
    fallback() {
      if (failed) return false;
      failed = true;
      requested = 'pcm';
      accepted = 'pcm';
      return true;
    },
    reset() {
      accepted = 'pcm';
      requested = 'pcm';
      failed = false;
    },
  };
}

export type RxDecodedAudio = {
  sampleRate: number;
  numberOfChannels: number;
  numberOfFrames: number;
  copyTo(destination: Float32Array, options: {planeIndex: number; format: 'f32-planar'}): void;
};

export function copyRxOpusAudio(data: RxDecodedAudio): {left: Float32Array; right: Float32Array; frames: number; sampleRate: number} | null {
  if (data.sampleRate !== RX_OPUS_SAMPLE_RATE || data.numberOfFrames < 1 ||
      data.numberOfFrames > 5760 || (data.numberOfChannels !== 1 && data.numberOfChannels !== 2)) return null;
  const left = new Float32Array(data.numberOfFrames);
  const right = new Float32Array(data.numberOfFrames);
  data.copyTo(left, {planeIndex: 0, format: 'f32-planar'});
  if (data.numberOfChannels === 2) data.copyTo(right, {planeIndex: 1, format: 'f32-planar'});
  else right.set(left);
  return {left, right, frames: data.numberOfFrames, sampleRate: data.sampleRate};
}

// A single corrupt packet is a dropped 20 ms frame, not a reason to abandon the
// codec. A run of them means the wire contract is not what this browser thinks
// it is, so fall back to PCM. Falling back is latched until the socket reopens.
export const RX_OPUS_MALFORMED_FALLBACK_LIMIT = 5;
// The decoder pending map is keyed by packet timestamp; entries leave it when
// the decoder emits output. A map this deep means the decoder stalled, which is
// a resync (drop what is pending and keep Opus), not a contract mismatch.
export const RX_OPUS_BACKLOG_RESYNC_LIMIT = 32;

export function rxOpusMalformedAction(
  consecutiveMalformed: number,
  limit = RX_OPUS_MALFORMED_FALLBACK_LIMIT,
): 'drop' | 'fallback' {
  return consecutiveMalformed >= limit ? 'fallback' : 'drop';
}

export function rxOpusBacklogAction(
  pendingEntries: number,
  limit = RX_OPUS_BACKLOG_RESYNC_LIMIT,
): 'none' | 'resync' {
  return pendingEntries > limit ? 'resync' : 'none';
}
