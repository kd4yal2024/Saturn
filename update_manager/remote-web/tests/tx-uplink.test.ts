import { describe, expect, it, vi } from 'vitest';
import {
  TX_UPLINK_HARD_CAP_BYTES,
  decideTxMicSend,
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

  it('hard cap drops even when the RTT-scaled drop threshold would still send', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    // RTT-scaled drop threshold at 200ms = 163_200 bytes; hard cap is lower.
    // Pick a value above the hard cap but well below the RTT drop threshold
    // so we prove the hard cap is the active constraint.
    const decision = decideTxMicSend(TX_UPLINK_HARD_CAP_BYTES + 1, 200, byteRate);
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
});
