import { describe, expect, it, vi } from 'vitest';
import {
  decideTxMicSend,
  txMicByteRateBytesPerSecond,
  txUplinkBufferedThresholdBytes,
} from '../src/transport/tx-uplink';

describe('TX uplink guard', () => {
  it('derives RTT-scaled thresholds from the Float32 mic byte rate', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    expect(byteRate).toBeCloseTo(204_000, 0);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 2)).toBe(81_600);
    expect(txUplinkBufferedThresholdBytes(200, byteRate, 4)).toBe(163_200);
  });

  it('marks degraded before it starts dropping mic frames', () => {
    const byteRate = txMicByteRateBytesPerSecond(48_000, 4, 256, 64);
    const decision = decideTxMicSend(90_000, 200, byteRate);
    expect(decision.degraded).toBe(true);
    expect(decision.action).toBe('send');
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
});
