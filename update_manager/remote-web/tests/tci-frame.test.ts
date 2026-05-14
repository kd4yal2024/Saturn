import { describe, expect, it } from 'vitest';
import { framePayloadFloats, parseTciBinaryFrameHeader, TCI_FRAME_HEADER_BYTES } from '../src/transport/tci-frame';

describe('TCI binary frames', () => {
  it('parses the Saturn 64-byte frame header', () => {
    const buffer = new ArrayBuffer(TCI_FRAME_HEADER_BYTES + 8);
    const view = new DataView(buffer);
    view.setUint32(4, 48_000, true);
    view.setUint32(20, 2, true);
    view.setUint32(24, 1, true);
    view.setUint32(28, 2, true);

    expect(parseTciBinaryFrameHeader(buffer)).toEqual({
      sampleRate: 48_000,
      floatCount: 2,
      frameType: 1,
      channels: 2,
      sequence: 0,
    });
  });

  it('returns payload floats and rejects truncated payloads', () => {
    const buffer = new ArrayBuffer(TCI_FRAME_HEADER_BYTES + 8);
    const payload = new Float32Array(buffer, TCI_FRAME_HEADER_BYTES, 2);
    payload[0] = 0.25;
    payload[1] = -0.5;

    expect(Array.from(framePayloadFloats(buffer, 2))).toEqual([0.25, -0.5]);
    expect(() => framePayloadFloats(buffer, 3)).toThrow(RangeError);
  });
});
