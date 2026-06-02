import { describe, expect, it } from 'vitest';
import {
  TX_OPUS_ENCODER_RUNTIME_ENABLED,
  TX_OPUS_DECODE_OUTPUT_SAMPLE_RATE_HZ,
  TX_OPUS_FRAME_DURATION_US,
  buildTxMicOpusFrame,
  createTxOpusEncoderSkeleton,
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
    expect(status.state).toBe('disabled');
    expect(status.reason).toBe('runtime_disabled');
    expect(status.profile.codec).toBe('opus_wb');
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
});
