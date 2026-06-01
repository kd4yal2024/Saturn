import { describe, expect, it, vi } from 'vitest';
import {
  TX_MIC_BYTES_PER_SAMPLE_S16,
  TX_MIC_CODEC_OPUS_NB,
  TX_MIC_CODEC_OPUS_WB,
  TX_MIC_CODEC_PCM,
  TX_MIC_FRAME_HEADER_BYTES,
  TX_MIC_SAMPLE_RATE_HZ,
  TX_MIC_SAMPLE_TYPE_S16,
  TX_MIC_STREAM_TYPE,
  TX_UPLINK_HARD_CAP_BYTES,
  TX_UPLINK_SOFT_CAP_BYTES,
  buildTxMicPcmS16Frame,
  decideTxMicSend,
  detectTxCodecCapabilities,
  txMicByteRateBytesPerSecond,
  txUplinkBufferedThresholdBytes,
} from '../src/transport/tx-uplink';
import { TX_MIC_BLOCK_SAMPLES } from '../src/audio/constants';

describe('TX uplink guard', () => {
  it('derives RTT-scaled thresholds from the Float32 mic byte rate', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    expect(byteRate).toBeCloseTo(204_000, 0);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 2)).toBe(81_600);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 4)).toBe(163_200);
  });

  it('derives the lower byte rate used by s16 mic frames', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 2, 256, 64);
    expect(byteRate).toBeCloseTo(108_000, 0);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 2)).toBe(43_200);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 4)).toBe(86_400);
  });

  it('derives the current coalesced s16 mic byte rate', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 2, TX_MIC_BLOCK_SAMPLES, 64);
    expect(byteRate).toBeCloseTo(99_000, 0);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 2)).toBe(39_600);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 4)).toBe(79_200);
  });

  it('sends and stays clear when bufferedAmount is well below the hard cap', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    const decision = decideTxMicSend(8_000, 200, byteRate);
    expect(decision.action).toBe('send');
    expect(decision.degraded).toBe(false);
    expect(decision.hardCapEngaged).toBe(false);
  });

  it('drops before the caller commits bytes to WebSocket.send', () => {
    const send = vi.fn();
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    const decision = decideTxMicSend(200_000, 200, byteRate);

    if (decision.action === 'send') {
      send(new ArrayBuffer(64));
    }

    expect(decision.degraded).toBe(true);
    expect(decision.action).toBe('drop');
    expect(send).not.toHaveBeenCalled();
  });

  it('soft cap reports degraded without dropping mic frames', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    const decision = decideTxMicSend(TX_UPLINK_SOFT_CAP_BYTES + 1, 200, byteRate);

    expect(decision.softCapEngaged).toBe(true);
    expect(decision.hardCapEngaged).toBe(false);
    expect(decision.degraded).toBe(true);
    expect(decision.action).toBe('send');
  });

  it('does not let low RTT undercut the absolute PCM drop cap', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 2, TX_MIC_BLOCK_SAMPLES, 64);
    const decision = decideTxMicSend(16_896, 40, byteRate);

    expect(decision.dropThresholdBytes).toBeLessThan(16_896);
    expect(decision.softCapEngaged).toBe(false);
    expect(decision.hardCapEngaged).toBe(false);
    expect(decision.degraded).toBe(false);
    expect(decision.action).toBe('send');
  });

  it('hard cap drops even when the RTT-scaled drop threshold would still send', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    // RTT-scaled drop threshold at 200ms = 163_200 bytes; hard cap is lower.
    // Pick a value above the hard cap but well below the RTT drop threshold
    // so we prove the hard cap is the active constraint.
    const decision = decideTxMicSend(TX_UPLINK_HARD_CAP_BYTES + 1, 200, byteRate);
    expect(decision.softCapEngaged).toBe(true);
    expect(decision.hardCapEngaged).toBe(true);
    expect(decision.action).toBe('drop');
    expect(decision.degraded).toBe(true);
  });

  it('hard cap fires independent of RTT (cannot be tuned away)', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    // Even with absurdly large RTT (which would normally widen the cap), the
    // hard cap remains in force. This is the non-negotiable safety floor.
    const decision = decideTxMicSend(TX_UPLINK_HARD_CAP_BYTES + 1, 10_000, byteRate);
    expect(decision.hardCapEngaged).toBe(true);
    expect(decision.action).toBe('drop');
  });

  it('builds PCM mic frames with Phase 44 codec header fields', () => {
    const { frame, peak, payloadBytes } = buildTxMicPcmS16Frame([0.25, -0.5, 2], 123);
    const view = new DataView(frame);

    expect(frame.byteLength).toBe(TX_MIC_FRAME_HEADER_BYTES + 3 * TX_MIC_BYTES_PER_SAMPLE_S16);
    expect(payloadBytes).toBe(3 * TX_MIC_BYTES_PER_SAMPLE_S16);
    expect(peak).toBe(1);
    expect(view.getUint32(4, true)).toBe(TX_MIC_SAMPLE_RATE_HZ);
    expect(view.getUint32(8, true)).toBe(TX_MIC_SAMPLE_TYPE_S16);
    expect(view.getUint32(20, true)).toBe(3);
    expect(view.getUint32(24, true)).toBe(TX_MIC_STREAM_TYPE);
    expect(view.getUint32(28, true)).toBe(1);
    expect(view.getUint32(32, true)).toBe(123);
    expect(view.getUint32(36, true)).toBe(TX_MIC_CODEC_PCM);
    expect(view.getUint32(40, true)).toBe(payloadBytes);
    expect(view.getInt16(TX_MIC_FRAME_HEADER_BYTES, true)).toBe(8192);
    expect(view.getInt16(TX_MIC_FRAME_HEADER_BYTES + 2, true)).toBe(-16384);
    expect(view.getInt16(TX_MIC_FRAME_HEADER_BYTES + 4, true)).toBe(32767);
  });

  it('defines reserved Phase 44 Opus codec ids without using them yet', () => {
    expect(TX_MIC_CODEC_PCM).toBe(0);
    expect(TX_MIC_CODEC_OPUS_NB).toBe(1);
    expect(TX_MIC_CODEC_OPUS_WB).toBe(2);
  });

  it('detects PCM-only TX codec support when WebCodecs AudioEncoder is unavailable', async () => {
    const detection = await detectTxCodecCapabilities({});
    expect(detection.detected).toEqual(['pcm']);
    expect(detection.advertised).toEqual(['pcm']);
    expect(detection.webCodecsAudioEncoder).toBe(false);
    expect(detection.opusNb).toBe(false);
    expect(detection.opusWb).toBe(false);
  });

  it('detects browser Opus support while still advertising PCM for this scaffold', async () => {
    const calls: Record<string, unknown>[] = [];
    const detection = await detectTxCodecCapabilities({
      AudioEncoder: {
        isConfigSupported: vi.fn(async (config: Record<string, unknown>) => {
          calls.push(config);
          return { supported: config.codec === 'opus' };
        }),
      },
    });

    expect(calls).toHaveLength(2);
    expect(calls.map((config) => config.sampleRate)).toEqual([16_000, 48_000]);
    expect(detection.detected).toEqual(['pcm', 'opus_nb', 'opus_wb']);
    expect(detection.advertised).toEqual(['pcm']);
    expect(detection.webCodecsAudioEncoder).toBe(true);
    expect(detection.opusNb).toBe(true);
    expect(detection.opusWb).toBe(true);
  });
});
