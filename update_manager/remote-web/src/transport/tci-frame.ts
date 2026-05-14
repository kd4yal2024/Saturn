export const TCI_FRAME_HEADER_BYTES = 64;

export const enum TciStreamType {
  Iq = 0,
  AudioLeft = 1,
  AudioRight = 2,
}

export type TciBinaryFrameHeader = {
  sampleRate: number;
  floatCount: number;
  frameType: number;
  channels: number;
  sequence: number;
};

export function parseTciBinaryFrameHeader(buffer: ArrayBuffer): TciBinaryFrameHeader {
  if (buffer.byteLength < TCI_FRAME_HEADER_BYTES) {
    throw new RangeError(`TCI binary frame must be at least ${TCI_FRAME_HEADER_BYTES} bytes`);
  }

  const view = new DataView(buffer);
  return {
    sampleRate: view.getUint32(4, true),
    floatCount: view.getUint32(20, true),
    frameType: view.getUint32(24, true),
    channels: Math.max(1, view.getUint32(28, true) || 2),
    sequence: view.getUint32(32, true),
  };
}

export function framePayloadFloats(buffer: ArrayBuffer, floatCount: number): Float32Array {
  const expectedBytes = TCI_FRAME_HEADER_BYTES + floatCount * Float32Array.BYTES_PER_ELEMENT;
  if (buffer.byteLength < expectedBytes) {
    throw new RangeError(`TCI frame payload is truncated: expected ${expectedBytes} bytes, got ${buffer.byteLength}`);
  }
  return new Float32Array(buffer, TCI_FRAME_HEADER_BYTES, floatCount);
}
