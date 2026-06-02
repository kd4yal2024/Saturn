import {
  TX_MIC_CODEC_OPUS_NB,
  TX_MIC_CODEC_OPUS_WB,
  TX_MIC_FRAME_HEADER_BYTES,
  TX_MIC_SAMPLE_TYPE_S16,
  TX_MIC_STREAM_TYPE,
  type TxCodecCapability,
} from '../transport/tx-uplink';

export const TX_OPUS_ENCODER_RUNTIME_ENABLED = false;
export const TX_OPUS_FRAME_DURATION_US = 20_000;
export const TX_OPUS_NB_SAMPLE_RATE_HZ = 16_000;
export const TX_OPUS_WB_SAMPLE_RATE_HZ = 48_000;
export const TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ = 48_000;
export const TX_OPUS_NB_BITRATE_BPS = 16_000;
export const TX_OPUS_WB_BITRATE_BPS = 24_000;
export const TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES =
  (TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ * TX_OPUS_FRAME_DURATION_US) / 1_000_000;

export type TxOpusCodec = Extract<TxCodecCapability, 'opus_nb' | 'opus_wb'>;

export type TxOpusProfile = {
  codec: TxOpusCodec;
  codecId: number;
  sampleRateHz: number;
  decodeOutputSampleRateHz: number;
  bitrateBps: number;
  frameDurationUs: number;
  frameSamples: number;
  decodedFrameSamples: number;
  fecEnabled: boolean;
  dtxEnabled: boolean;
};

export type TxOpusEncoderStatus =
  | { state: 'disabled'; reason: 'runtime_disabled'; profile: TxOpusProfile }
  | {
      state: 'unavailable';
      reason: 'audio_encoder_missing' | 'encoder_backend_missing';
      profile: TxOpusProfile;
    }
  | {
      state: 'ready';
      profile: TxOpusProfile;
      producer: TxOpusFrameProducer;
      config: TxOpusAudioEncoderConfig;
    };

export type TxMicOpusFrame = {
  frame: ArrayBuffer;
  payloadBytes: number;
  codecId: number;
  decodedSampleCount: number;
};

export type TxOpusAudioEncoderConfig = {
  codec: 'opus';
  sampleRate: number;
  numberOfChannels: 1;
  bitrate: number;
  opus: {
    frameDuration: number;
    useinbandfec: boolean;
    usedtx: boolean;
  };
};

export type TxOpusEncodedAudioChunkLike = {
  byteLength: number;
  duration?: number | null;
  copyTo(destination: Uint8Array): void;
};

export type TxOpusAudioEncoderLike = {
  configure(config: TxOpusAudioEncoderConfig): void;
  encode(audioData: unknown): void;
  flush?: () => Promise<void>;
  close?: () => void;
};

export type TxOpusAudioEncoderInit = {
  output: (chunk: TxOpusEncodedAudioChunkLike) => void;
  error: (error: unknown) => void;
};

export type TxOpusAudioEncoderConstructorLike = new (
  init: TxOpusAudioEncoderInit,
) => TxOpusAudioEncoderLike;

export type TxOpusFrameProducerOptions = {
  enabled?: boolean;
  scope?: unknown;
  onFrame?: (frame: TxMicOpusFrame, metadata: TxOpusFrameMetadata) => void;
  onError?: (error: unknown) => void;
};

export type TxOpusFrameMetadata = {
  codec: TxOpusCodec;
  sequence: number;
  payloadBytes: number;
  decodedSampleCount: number;
  chunkDurationUs: number | null;
};

type PendingEncode = {
  sequence: number;
  decodedSampleCount: number;
};

export function txOpusProfileForCodec(codec: TxOpusCodec): TxOpusProfile {
  if (codec === 'opus_nb') {
    return {
      codec,
      codecId: TX_MIC_CODEC_OPUS_NB,
      sampleRateHz: TX_OPUS_NB_SAMPLE_RATE_HZ,
      decodeOutputSampleRateHz: TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ,
      bitrateBps: TX_OPUS_NB_BITRATE_BPS,
      frameDurationUs: TX_OPUS_FRAME_DURATION_US,
      frameSamples: (TX_OPUS_NB_SAMPLE_RATE_HZ * TX_OPUS_FRAME_DURATION_US) / 1_000_000,
      decodedFrameSamples: TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES,
      fecEnabled: true,
      dtxEnabled: false,
    };
  }
  return {
    codec,
    codecId: TX_MIC_CODEC_OPUS_WB,
    sampleRateHz: TX_OPUS_WB_SAMPLE_RATE_HZ,
    decodeOutputSampleRateHz: TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ,
    bitrateBps: TX_OPUS_WB_BITRATE_BPS,
    frameDurationUs: TX_OPUS_FRAME_DURATION_US,
    frameSamples: (TX_OPUS_WB_SAMPLE_RATE_HZ * TX_OPUS_FRAME_DURATION_US) / 1_000_000,
    decodedFrameSamples: TX_OPUS_DECODE_OUTPUT_FRAME_SAMPLES,
    fecEnabled: true,
    dtxEnabled: false,
  };
}

export function txOpusAudioEncoderConfig(profile: TxOpusProfile): TxOpusAudioEncoderConfig {
  return {
    codec: 'opus',
    sampleRate: profile.sampleRateHz,
    numberOfChannels: 1,
    bitrate: profile.bitrateBps,
    opus: {
      frameDuration: profile.frameDurationUs,
      useinbandfec: profile.fecEnabled,
      usedtx: profile.dtxEnabled,
    },
  };
}

export function createTxOpusEncoderSkeleton(
  codec: TxOpusCodec,
  scope: unknown = globalThis,
): TxOpusEncoderStatus {
  const profile = txOpusProfileForCodec(codec);
  if (!TX_OPUS_ENCODER_RUNTIME_ENABLED) {
    return { state: 'disabled', reason: 'runtime_disabled', profile };
  }
  const encoder = (scope as { AudioEncoder?: unknown } | null | undefined)?.AudioEncoder;
  if (!encoder) {
    return { state: 'unavailable', reason: 'audio_encoder_missing', profile };
  }
  return { state: 'unavailable', reason: 'encoder_backend_missing', profile };
}

export class TxOpusFrameProducer {
  private readonly profile: TxOpusProfile;
  private readonly encoder: TxOpusAudioEncoderLike;
  private readonly onFrame: (frame: TxMicOpusFrame, metadata: TxOpusFrameMetadata) => void;
  private readonly onError: (error: unknown) => void;
  private readonly pending: PendingEncode[] = [];

  constructor(
    profile: TxOpusProfile,
    encoder: TxOpusAudioEncoderLike,
    onFrame: (frame: TxMicOpusFrame, metadata: TxOpusFrameMetadata) => void,
    onError: (error: unknown) => void,
  ) {
    this.profile = profile;
    this.encoder = encoder;
    this.onFrame = onFrame;
    this.onError = onError;
  }

  encode(audioData: unknown, sequence: number, decodedSampleCount = this.profile.decodedFrameSamples): void {
    this.pending.push({
      sequence: sequence >>> 0,
      decodedSampleCount: Math.max(1, Math.floor(Number(decodedSampleCount) || this.profile.decodedFrameSamples)),
    });
    try {
      this.encoder.encode(audioData);
    } catch (error) {
      this.pending.pop();
      this.onError(error);
    }
  }

  async flush(): Promise<void> {
    if (typeof this.encoder.flush === 'function') {
      await this.encoder.flush();
    }
  }

  close(): void {
    this.pending.length = 0;
    if (typeof this.encoder.close === 'function') {
      this.encoder.close();
    }
  }

  handleEncodedChunk(chunk: TxOpusEncodedAudioChunkLike): void {
    const pending = this.pending.shift() ?? {
      sequence: 0,
      decodedSampleCount: this.profile.decodedFrameSamples,
    };
    const byteLength = Math.max(0, Math.floor(Number(chunk.byteLength) || 0));
    const payload = new Uint8Array(byteLength);
    try {
      chunk.copyTo(payload);
    } catch (error) {
      this.onError(error);
      return;
    }
    const built = buildTxMicOpusFrame(
      payload,
      pending.sequence,
      this.profile.codec,
      pending.decodedSampleCount,
    );
    this.onFrame(built, {
      codec: this.profile.codec,
      sequence: pending.sequence,
      payloadBytes: built.payloadBytes,
      decodedSampleCount: built.decodedSampleCount,
      chunkDurationUs: Number.isFinite(chunk.duration ?? NaN) ? Math.max(0, Number(chunk.duration)) : null,
    });
  }
}

export function createTxOpusFrameProducer(
  codec: TxOpusCodec,
  options: TxOpusFrameProducerOptions = {},
): TxOpusEncoderStatus {
  const profile = txOpusProfileForCodec(codec);
  if (options.enabled !== true) {
    return { state: 'disabled', reason: 'runtime_disabled', profile };
  }
  const Encoder = (options.scope as { AudioEncoder?: TxOpusAudioEncoderConstructorLike } | null | undefined)
    ?.AudioEncoder;
  if (!Encoder) {
    return { state: 'unavailable', reason: 'audio_encoder_missing', profile };
  }

  const config = txOpusAudioEncoderConfig(profile);
  let producer: TxOpusFrameProducer | null = null;
  let encoder: TxOpusAudioEncoderLike;
  try {
    encoder = new Encoder({
      output: (chunk) => producer?.handleEncodedChunk(chunk),
      error: (error) => {
        options.onError?.(error);
      },
    });
    encoder.configure(config);
  } catch (error) {
    options.onError?.(error);
    return { state: 'unavailable', reason: 'encoder_backend_missing', profile };
  }
  producer = new TxOpusFrameProducer(
    profile,
    encoder,
    options.onFrame ?? (() => {}),
    options.onError ?? (() => {}),
  );
  return { state: 'ready', profile, producer, config };
}

export function buildTxMicOpusFrame(
  payload: ArrayLike<number>,
  sequence: number,
  codec: TxOpusCodec,
  decodedSampleCount?: number,
): TxMicOpusFrame {
  const profile = txOpusProfileForCodec(codec);
  const payloadBytes = Math.max(0, Math.floor(Number(payload.length) || 0));
  const sampleCount = Math.max(
    1,
    Math.floor(Number(decodedSampleCount) || profile.decodedFrameSamples),
  );
  const frame = new ArrayBuffer(TX_MIC_FRAME_HEADER_BYTES + payloadBytes);
  const view = new DataView(frame);
  view.setUint32(0, 0, true);
  view.setUint32(4, profile.sampleRateHz, true);
  // The bridge uses codec_id to identify Opus. sample_type remains s16 so old
  // PCM paths never accidentally reinterpret compressed payload as float data.
  view.setUint32(8, TX_MIC_SAMPLE_TYPE_S16, true);
  view.setUint32(20, sampleCount, true);
  view.setUint32(24, TX_MIC_STREAM_TYPE, true);
  view.setUint32(28, 1, true);
  view.setUint32(32, sequence >>> 0, true);
  view.setUint32(36, profile.codecId, true);
  view.setUint32(40, payloadBytes >>> 0, true);

  for (let i = 0; i < payloadBytes; i += 1) {
    view.setUint8(
      TX_MIC_FRAME_HEADER_BYTES + i,
      Math.max(0, Math.min(255, Math.round(Number(payload[i]) || 0))),
    );
  }

  return {
    frame,
    payloadBytes,
    codecId: profile.codecId,
    decodedSampleCount: sampleCount,
  };
}
