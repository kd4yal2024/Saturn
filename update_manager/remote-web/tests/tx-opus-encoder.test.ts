import { describe, expect, it } from 'vitest';
import {
  TX_OPUS_ENCODER_RUNTIME_ENABLED,
  TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ,
  TX_OPUS_FRAME_DURATION_US,
  buildTxMicOpusFrame,
  createTxOpusEncoderSkeleton,
  createTxOpusFrameProducer,
  type TxOpusAudioEncoderConfig,
  type TxOpusAudioEncoderInit,
  type TxOpusEncodedAudioChunkLike,
  type TxMicOpusFrame,
  txOpusProfileForCodec,
} from '../src/audio/tx-opus-encoder';
import {
  TX_MIC_CODEC_OPUS_NB,
  TX_MIC_CODEC_OPUS_WB,
  TX_MIC_FRAME_HEADER_BYTES,
  TX_MIC_SAMPLE_TYPE_S16,
  TX_MIC_STREAM_TYPE,
} from '../src/transport/tx-uplink';

describe('Phase 44 TX Opus encoder skeleton', () => {
  it('defines narrowband and wideband Opus profiles with DTX disabled', () => {
    const nb = txOpusProfileForCodec('opus_nb');
    expect(nb.codecId).toBe(TX_MIC_CODEC_OPUS_NB);
    expect(nb.sampleRateHz).toBe(16_000);
    expect(nb.decodeOutputSampleRateHz).toBe(TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ);
    expect(nb.bitrateBps).toBe(16_000);
    expect(nb.frameDurationUs).toBe(TX_OPUS_FRAME_DURATION_US);
    expect(nb.frameSamples).toBe(320);
    expect(nb.decodedFrameSamples).toBe(960);
    expect(nb.fecEnabled).toBe(true);
    expect(nb.dtxEnabled).toBe(false);

    const wb = txOpusProfileForCodec('opus_wb');
    expect(wb.codecId).toBe(TX_MIC_CODEC_OPUS_WB);
    expect(wb.sampleRateHz).toBe(48_000);
    expect(wb.decodeOutputSampleRateHz).toBe(TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ);
    expect(wb.bitrateBps).toBe(24_000);
    expect(wb.frameSamples).toBe(960);
    expect(wb.decodedFrameSamples).toBe(960);
    expect(wb.fecEnabled).toBe(true);
    expect(wb.dtxEnabled).toBe(false);
  });

  it('keeps the runtime encoder disabled until bridge acceptance gates are ready', () => {
    const status = createTxOpusEncoderSkeleton('opus_wb', {
      AudioEncoder: function AudioEncoder() {},
    });

    expect(TX_OPUS_ENCODER_RUNTIME_ENABLED).toBe(false);
    if (status.state !== 'disabled') {
      throw new Error(`expected disabled skeleton, got ${status.state}`);
    }
    expect(status.reason).toBe('runtime_disabled');
    expect(status.profile.codec).toBe('opus_wb');
  });

  it('does not construct a browser Opus producer unless explicitly enabled', () => {
    let constructed = 0;
    class DisabledEncoder {
      constructor() {
        constructed += 1;
      }
    }

    const status = createTxOpusFrameProducer('opus_wb', {
      enabled: false,
      scope: { AudioEncoder: DisabledEncoder },
    });

    if (status.state !== 'disabled') {
      throw new Error(`expected disabled producer, got ${status.state}`);
    }
    expect(status.reason).toBe('runtime_disabled');
    expect(constructed).toBe(0);
  });

  it('builds future Opus mic frames without enabling the sender path', () => {
    const { frame, payloadBytes, codecId, decodedSampleCount } = buildTxMicOpusFrame(
      [1, 2, 255],
      77,
      'opus_wb',
    );
    const view = new DataView(frame);

    expect(frame.byteLength).toBe(TX_MIC_FRAME_HEADER_BYTES + 3);
    expect(payloadBytes).toBe(3);
    expect(codecId).toBe(TX_MIC_CODEC_OPUS_WB);
    expect(decodedSampleCount).toBe(960);
    expect(view.getUint32(4, true)).toBe(48_000);
    expect(view.getUint32(8, true)).toBe(TX_MIC_SAMPLE_TYPE_S16);
    expect(view.getUint32(20, true)).toBe(960);
    expect(view.getUint32(24, true)).toBe(TX_MIC_STREAM_TYPE);
    expect(view.getUint32(28, true)).toBe(1);
    expect(view.getUint32(32, true)).toBe(77);
    expect(view.getUint32(36, true)).toBe(TX_MIC_CODEC_OPUS_WB);
    expect(view.getUint32(40, true)).toBe(3);
    expect(view.getUint8(TX_MIC_FRAME_HEADER_BYTES)).toBe(1);
    expect(view.getUint8(TX_MIC_FRAME_HEADER_BYTES + 1)).toBe(2);
    expect(view.getUint8(TX_MIC_FRAME_HEADER_BYTES + 2)).toBe(255);
  });

  it('uses 48 kHz decoded sample count for narrowband Opus mic headers', () => {
    const { frame, decodedSampleCount } = buildTxMicOpusFrame([7, 8, 9], 88, 'opus_nb');
    const view = new DataView(frame);

    expect(decodedSampleCount).toBe(960);
    expect(view.getUint32(4, true)).toBe(16_000);
    expect(view.getUint32(20, true)).toBe(960);
    expect(view.getUint32(36, true)).toBe(TX_MIC_CODEC_OPUS_NB);
  });

  it('wraps a WebCodecs Opus chunk in a Phase 44 mic frame when explicitly enabled', () => {
    const frames: Array<{ frame: TxMicOpusFrame; sequence: number; payloadBytes: number }> = [];
    const errors: unknown[] = [];
    let configured: TxOpusAudioEncoderConfig | null = null;

    class FakeAudioEncoder {
      private readonly init: TxOpusAudioEncoderInit;

      constructor(init: TxOpusAudioEncoderInit) {
        this.init = init;
      }

      configure(config: TxOpusAudioEncoderConfig) {
        configured = config;
      }

      encode(audioData: unknown) {
        const bytes = (audioData as { encodedBytes: number[] }).encodedBytes;
        const chunk: TxOpusEncodedAudioChunkLike = {
          byteLength: bytes.length,
          duration: TX_OPUS_FRAME_DURATION_US,
          copyTo(destination: Uint8Array) {
            bytes.forEach((value, index) => {
              destination[index] = value;
            });
          },
        };
        this.init.output(chunk);
      }
    }

    const status = createTxOpusFrameProducer('opus_wb', {
      enabled: true,
      scope: { AudioEncoder: FakeAudioEncoder },
      onFrame: (frame, metadata) => {
        frames.push({ frame, sequence: metadata.sequence, payloadBytes: metadata.payloadBytes });
      },
      onError: (error) => errors.push(error),
    });

    if (status.state !== 'ready') {
      throw new Error(`expected ready producer, got ${status.state}`);
    }

    expect(configured).toEqual({
      codec: 'opus',
      sampleRate: 48_000,
      numberOfChannels: 1,
      bitrate: 24_000,
      opus: {
        frameDuration: TX_OPUS_FRAME_DURATION_US,
        useinbandfec: true,
        usedtx: false,
      },
    });

    status.producer.encode({ encodedBytes: [4, 5, 6, 7] }, 123);

    expect(errors).toEqual([]);
    expect(frames).toHaveLength(1);
    const produced = frames[0];
    if (!produced) {
      throw new Error('expected one produced Opus frame');
    }
    expect(produced.sequence).toBe(123);
    expect(produced.payloadBytes).toBe(4);
    const view = new DataView(produced.frame.frame);
    expect(view.getUint32(4, true)).toBe(48_000);
    expect(view.getUint32(20, true)).toBe(960);
    expect(view.getUint32(32, true)).toBe(123);
    expect(view.getUint32(36, true)).toBe(TX_MIC_CODEC_OPUS_WB);
    expect(view.getUint32(40, true)).toBe(4);
    expect(view.getUint8(TX_MIC_FRAME_HEADER_BYTES)).toBe(4);
    expect(view.getUint8(TX_MIC_FRAME_HEADER_BYTES + 3)).toBe(7);
  });
});
