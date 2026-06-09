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
export const TX_OPUS_OVERRIDE_QUERY_PARAM = 'phase44_tx_opus';
export const TX_OPUS_OVERRIDE_STORAGE_KEY = 'saturn.phase44.txOpus';

const TX_OPUS_OVERRIDE_TRUE_VALUES = new Set(['1', 'true', 'on', 'yes', 'opus']);

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

export type TxOpusAudioDataInit = {
  format: 'f32';
  sampleRate: number;
  numberOfFrames: number;
  numberOfChannels: 1;
  timestamp: number;
  data: Float32Array;
};

export type TxOpusAudioDataLike = {
  close?: () => void;
};

export type TxOpusAudioDataConstructorLike = new (
  init: TxOpusAudioDataInit,
) => TxOpusAudioDataLike;

export type TxOpusAudioDataStatus =
  | {
      state: 'ready';
      profile: TxOpusProfile;
      audioData: TxOpusAudioDataLike;
      decodedSampleCount: number;
      timestampUs: number;
    }
  | {
      state: 'unavailable';
      reason:
        | 'audio_data_missing'
        | 'sample_rate_mismatch'
        | 'empty_audio'
        | 'frame_sample_count_mismatch'
        | 'audio_data_error';
      profile: TxOpusProfile;
    };

export type TxOpusAudioEncoderConfig = {
  codec: 'opus';
  sampleRate: number;
  numberOfChannels: 1;
  bitrate: number;
  opus: {
    format: 'opus';
    signal: 'voice';
    application: 'voip';
    frameDuration: number;
    complexity: number;
    packetlossperc: number;
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

export function txOpusRuntimeOverrideEnabled(
  search: string,
  storedValue: string | null | undefined = undefined,
): boolean {
  const params = new URLSearchParams(search);
  const queryValue = params.get(TX_OPUS_OVERRIDE_QUERY_PARAM);
  if (queryValue !== null) {
    return TX_OPUS_OVERRIDE_TRUE_VALUES.has(queryValue.trim().toLowerCase());
  }
  if (storedValue !== undefined && storedValue !== null) {
    return TX_OPUS_OVERRIDE_TRUE_VALUES.has(storedValue.trim().toLowerCase());
  }
  return false;
}

export function txOpusCodecForAccepted(
  acceptedCodec: TxCodecCapability | null | undefined,
  overrideEnabled: boolean,
): TxOpusCodec | null {
  if (!overrideEnabled) return null;
  if (acceptedCodec === 'opus_wb' || acceptedCodec === 'opus_nb') return acceptedCodec;
  return null;
}

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
      format: 'opus',
      signal: 'voice',
      application: 'voip',
      frameDuration: profile.frameDurationUs,
      complexity: 5,
      packetlossperc: profile.fecEnabled ? 5 : 0,
      useinbandfec: profile.fecEnabled,
      usedtx: profile.dtxEnabled,
    },
  };
}

export function createTxOpusAudioDataFromFloat32(
  monoSamples: ArrayLike<number>,
  codec: TxOpusCodec,
  options: {
    scope?: unknown;
    sourceSampleRateHz?: number;
    timestampUs?: number;
    sequence?: number;
  } = {},
): TxOpusAudioDataStatus {
  const profile = txOpusProfileForCodec(codec);
  const sourceSampleRateHz = Math.max(1, Math.round(Number(options.sourceSampleRateHz) || TX_OPUS_WB_SAMPLE_RATE_HZ));
  if (sourceSampleRateHz !== profile.sampleRateHz) {
    return { state: 'unavailable', reason: 'sample_rate_mismatch', profile };
  }
  const sampleCount = Math.max(0, Math.floor(Number(monoSamples.length) || 0));
  if (sampleCount <= 0) {
    return { state: 'unavailable', reason: 'empty_audio', profile };
  }
  if (sampleCount !== profile.frameSamples) {
    return { state: 'unavailable', reason: 'frame_sample_count_mismatch', profile };
  }
  const AudioData = (options.scope as { AudioData?: TxOpusAudioDataConstructorLike } | null | undefined)?.AudioData;
  if (!AudioData) {
    return { state: 'unavailable', reason: 'audio_data_missing', profile };
  }

  const data = new Float32Array(sampleCount);
  for (let i = 0; i < sampleCount; i += 1) {
    const sample = Number(monoSamples[i]) || 0;
    data[i] = Math.max(-1, Math.min(1, sample));
  }
  const timestampUs = Math.max(
    0,
    Math.floor(
      Number.isFinite(options.timestampUs ?? NaN)
        ? Number(options.timestampUs)
        : (Math.max(0, Math.floor(Number(options.sequence) || 0)) * profile.frameDurationUs),
    ),
  );
  try {
    return {
      state: 'ready',
      profile,
      audioData: new AudioData({
        format: 'f32',
        sampleRate: profile.sampleRateHz,
        numberOfFrames: sampleCount,
        numberOfChannels: 1,
        timestamp: timestampUs,
        data,
      }),
      decodedSampleCount: profile.decodedFrameSamples,
      timestampUs,
    };
  } catch {
    return { state: 'unavailable', reason: 'audio_data_error', profile };
  }
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
