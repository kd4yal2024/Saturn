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
// Safety net only: an age-pruned queue should never reach this. If it does, the
// decoder stalled and a resync (drop what is pending, keep Opus) is warranted.
export const RX_OPUS_BACKLOG_RESYNC_LIMIT = 64;
// Undecoded packets older than this are dropped instead of waiting forever.
export const RX_OPUS_PENDING_MAX_AGE_MS = 1000;
// If the decoder has consumed this many packets without producing one frame of
// audio, it is not going to recover: hand the session back to PCM.
export const RX_OPUS_STALL_FRAMES = 100;

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

/** A stalled decoder must not hold the session silent indefinitely. */
export function rxOpusStalled(
  opusFrames: number,
  decodedFrames: number,
  limit = RX_OPUS_STALL_FRAMES,
): boolean {
  return decodedFrames <= 0 && opusFrames >= limit;
}

export type RxOpusPendingEntry<T> = { value: T; timestamp: number; arrivedAt: number };

/**
 * Matches decoder output to submitted packets **in order**.
 *
 * WebCodecs normally echoes `EncodedAudioChunk.timestamp` on the output
 * `AudioData`, but Chrome's Opus decoder was observed renumbering timestamps for
 * a subset of packets (live LAN measurement: 1045 outputs for 1046 decodes, with
 * 42 input timestamps never appearing on any output). Timestamp-keyed matching
 * therefore leaked entries forever and tripped the backlog guard, dropping real
 * audio on a timer. Opus decodes one frame at a time in sequence, so consuming
 * the oldest pending entry is the correct pairing; timestamp disagreement is
 * counted for diagnostics instead of breaking the map.
 */
export type RxOpusPendingQueue<T> = {
  submit(timestamp: number, arrivedAt: number, value: T): void;
  take(timestamp: number): T | null;
  clear(): void;
  readonly size: number;
  readonly orphanOutputs: number;
  readonly prunedEntries: number;
  readonly timestampRewrites: number;
};

export function createRxOpusPendingQueue<T>(
  options: { maxAgeMs?: number } = {},
): RxOpusPendingQueue<T> {
  const maxAgeMs = options.maxAgeMs ?? RX_OPUS_PENDING_MAX_AGE_MS;
  const entries: RxOpusPendingEntry<T>[] = [];
  let orphanOutputs = 0;
  let prunedEntries = 0;
  let timestampRewrites = 0;
  return {
    submit(timestamp, arrivedAt, value) {
      while (entries.length > 0 && arrivedAt - (entries[0] as RxOpusPendingEntry<T>).arrivedAt > maxAgeMs) {
        entries.shift();
        prunedEntries += 1;
      }
      entries.push({value, timestamp, arrivedAt});
    },
    take(timestamp) {
      if (entries.length === 0) {
        orphanOutputs += 1;
        return null;
      }
      const entry = entries.shift() as RxOpusPendingEntry<T>;
      if (entry.timestamp !== timestamp) timestampRewrites += 1;
      return entry.value;
    },
    clear() { entries.length = 0; },
    get size() { return entries.length; },
    get orphanOutputs() { return orphanOutputs; },
    get prunedEntries() { return prunedEntries; },
    get timestampRewrites() { return timestampRewrites; },
  };
}
