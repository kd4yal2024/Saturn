import { describe, expect, it, vi } from 'vitest';
import {
  decideTxMicSend,
  txMicByteRateBytesPerSecond,
  txUplinkBufferedThresholdBytes,
} from '../src/transport/tx-uplink';

describe('TX uplink guard', () => {
  it('derives RTT-scaled thresholds from the s16 mic byte rate', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 2, 256, 64);
    expect(byteRate).toBeCloseTo(108_000, 0);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 2)).toBe(43_200);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 4)).toBe(86_400);
  });

  it('marks degraded before it starts dropping mic frames', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 2, 256, 64);
    const decision = decideTxMicSend(50_000, 200, byteRate);
    expect(decision.degraded).toBe(true);
    expect(decision.action).toBe('send');
  });

  it('drops before the caller commits bytes to WebSocket.send', () => {
    const send = vi.fn();
    const byteRate = txMicByteRateBytesPerSecond(48_000, 2, 256, 64);
    const decision = decideTxMicSend(100_000, 200, byteRate);

    if (decision.action === 'send') {
      send(new ArrayBuffer(64));
    }

    expect(decision.degraded).toBe(true);
    expect(decision.action).toBe('drop');
    expect(send).not.toHaveBeenCalled();
  });
});
