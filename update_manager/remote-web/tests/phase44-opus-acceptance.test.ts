import { describe, expect, it } from 'vitest';
import {
  TX_OPUS_FRAME_DURATION_US,
  createTxOpusAudioDataFromFloat32,
  createTxOpusFrameProducer,
  txOpusCodecForAccepted,
  txOpusRuntimeOverrideEnabled,
  type TxMicOpusFrame,
  type TxOpusAudioDataInit,
  type TxOpusAudioEncoderConfig,
  type TxOpusAudioEncoderInit,
  type TxOpusEncodedAudioChunkLike,
  type TxOpusFrameMetadata,
} from '../src/audio/tx-opus-encoder';
import { buildTxCodecCapsCommand } from '../src/tci/commands';
import { applyTciText } from '../src/tci/apply';
import { createAppState } from '../src/state/app-state';
import type { TciRadioState } from '../src/tci/state';
import {
  TX_MIC_CODEC_OPUS_WB,
  TX_MIC_FRAME_HEADER_BYTES,
  TX_MIC_SAMPLE_TYPE_S16,
  TX_MIC_STREAM_TYPE,
  detectTxCodecCapabilities,
} from '../src/transport/tx-uplink';

describe('Phase 44 disabled-by-default Opus acceptance harness', () => {
  it('keeps browser TX codec advertising PCM-only without the explicit override', async () => {
    const detection = await detectTxCodecCapabilities({
      AudioEncoder: {
        isConfigSupported: async (config: Record<string, unknown>) => {
          return { supported: config.codec === 'opus' };
        },
      },
    });

    expect(txOpusRuntimeOverrideEnabled('')).toBe(false);
    expect(detection.detected).toEqual(['pcm', 'opus_nb', 'opus_wb']);
    expect(detection.advertised).toEqual(['pcm']);
    expect(buildTxCodecCapsCommand(detection.advertised)).toBe('tx_codec_caps:0,pcm;');
  });

  it('advertises only wideband Opus when the local Phase 44 override is enabled', async () => {
    const detection = await detectTxCodecCapabilities(
      {
        AudioEncoder: {
          isConfigSupported: async (config: Record<string, unknown>) => {
            return { supported: config.codec === 'opus' };
          },
        },
      },
      { advertiseOpus: txOpusRuntimeOverrideEnabled('?phase44_tx_opus=1') },
    );

    expect(detection.detected).toEqual(['pcm', 'opus_nb', 'opus_wb']);
    expect(detection.advertised).toEqual(['pcm', 'opus_wb']);
    expect(buildTxCodecCapsCommand(detection.advertised)).toBe('tx_codec_caps:0,pcm,opus_wb;');
  });

  it('builds a bridge-ready 20 ms Opus wideband mic frame after bridge acceptance', () => {
    const accepted = applyTciText(
      'tx_codec_accept:0,opus_wb;',
      createAppState() as unknown as TciRadioState,
    );
    expect(accepted.state.txCodecAccepted).toBe('opus_wb');
    expect(txOpusCodecForAccepted(accepted.state.txCodecAccepted, true)).toBe('opus_wb');

    const encodedPayload = new Uint8Array([0x48, 0x11, 0x22, 0x33, 0x44]);
    const frames: Array<{ frame: TxMicOpusFrame; metadata: TxOpusFrameMetadata }> = [];
    const errors: unknown[] = [];
    let audioDataInit: TxOpusAudioDataInit | null = null;
    let configured: TxOpusAudioEncoderConfig | null = null;

    class FakeAudioData {
      constructor(init: TxOpusAudioDataInit) {
        audioDataInit = init;
      }
    }

    class FakeAudioEncoder {
      private readonly init: TxOpusAudioEncoderInit;

      constructor(init: TxOpusAudioEncoderInit) {
        this.init = init;
      }

      configure(config: TxOpusAudioEncoderConfig): void {
        configured = config;
      }

      encode(): void {
        const chunk: TxOpusEncodedAudioChunkLike = {
          byteLength: encodedPayload.byteLength,
          duration: TX_OPUS_FRAME_DURATION_US,
          copyTo(destination: Uint8Array) {
            destination.set(encodedPayload);
          },
        };
        this.init.output(chunk);
      }

      close(): void {
        // Test double only; real WebCodecs encoder releases browser resources.
      }
    }

    const samples = new Float32Array(960);
    for (let i = 0; i < samples.length; i += 1) {
      samples[i] = Math.sin((i / samples.length) * Math.PI * 2) * 0.2;
    }

    const audioData = createTxOpusAudioDataFromFloat32(samples, 'opus_wb', {
      scope: { AudioData: FakeAudioData },
      sourceSampleRateHz: 48_000,
      sequence: 120,
    });
    if (audioData.state !== 'ready') {
      throw new Error(`expected AudioData, got ${audioData.state}`);
    }

    const producer = createTxOpusFrameProducer('opus_wb', {
      enabled: true,
      scope: { AudioEncoder: FakeAudioEncoder },
      onFrame: (frame, metadata) => frames.push({ frame, metadata }),
      onError: (error) => errors.push(error),
    });
    if (producer.state !== 'ready') {
      throw new Error(`expected Opus producer, got ${producer.state}`);
    }

    producer.producer.encode(audioData.audioData, 120, audioData.decodedSampleCount);
    producer.producer.close();

    expect(errors).toEqual([]);
    expect(audioDataInit).toMatchObject({
      format: 'f32',
      sampleRate: 48_000,
      numberOfFrames: 960,
      numberOfChannels: 1,
      timestamp: 120 * TX_OPUS_FRAME_DURATION_US,
    });
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

    expect(frames).toHaveLength(1);
    const produced = frames[0];
    if (!produced) {
      throw new Error('expected one produced Opus frame');
    }

    expect(produced.metadata).toMatchObject({
      codec: 'opus_wb',
      sequence: 120,
      payloadBytes: encodedPayload.byteLength,
      decodedSampleCount: 960,
      chunkDurationUs: TX_OPUS_FRAME_DURATION_US,
    });

    const view = new DataView(produced.frame.frame);
    expect(produced.frame.frame.byteLength).toBe(TX_MIC_FRAME_HEADER_BYTES + encodedPayload.byteLength);
    expect(view.getUint32(4, true)).toBe(48_000);
    expect(view.getUint32(8, true)).toBe(TX_MIC_SAMPLE_TYPE_S16);
    expect(view.getUint32(20, true)).toBe(960);
    expect(view.getUint32(24, true)).toBe(TX_MIC_STREAM_TYPE);
    expect(view.getUint32(28, true)).toBe(1);
    expect(view.getUint32(32, true)).toBe(120);
    expect(view.getUint32(36, true)).toBe(TX_MIC_CODEC_OPUS_WB);
    expect(view.getUint32(40, true)).toBe(encodedPayload.byteLength);
    expect(new Uint8Array(produced.frame.frame, TX_MIC_FRAME_HEADER_BYTES)).toEqual(encodedPayload);
  });
});
