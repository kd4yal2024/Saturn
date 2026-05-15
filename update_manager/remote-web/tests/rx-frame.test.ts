import { describe, expect, it } from 'vitest';
import { decodeAudioFrame, decodeIqFrame, decodeRxFrame } from '../src/transport/rx-frame';
import { TCI_FRAME_HEADER_BYTES } from '../src/transport/tci-frame';

function buildFrame(
  frameType: number,
  sampleRate: number,
  channels: number,
  floats: number[],
  sequence = 0,
): ArrayBuffer {
  const buffer = new ArrayBuffer(TCI_FRAME_HEADER_BYTES + floats.length * 4);
  const view = new DataView(buffer);
  view.setUint32(4, sampleRate, true);
  view.setUint32(20, floats.length, true);
  view.setUint32(24, frameType, true);
  view.setUint32(28, channels, true);
  view.setUint32(32, sequence, true);
  new Float32Array(buffer, TCI_FRAME_HEADER_BYTES, floats.length).set(floats);
  return buffer;
}

describe('rx frame decoding', () => {
  it('decodes iq frames', () => {
    const buffer = buildFrame(0, 192000, 2, [1, 2, 3, 4]);
    const frame = decodeIqFrame(buffer);
    expect(frame?.kind).toBe('iq');
    expect(frame?.samplePairs).toBe(2);
    expect(Array.from(frame?.iq || [])).toEqual([1, 2, 3, 4]);
  });

  it('decodes audio frames and mirrors silent right channel', () => {
    const buffer = buildFrame(1, 48000, 2, [0.25, 0, -0.5, 0]);
    const frame = decodeAudioFrame(buffer);
    expect(frame?.kind).toBe('audio');
    expect(frame?.frames).toBe(2);
    expect(frame?.header.sequence).toBe(0);
    expect(frame?.mirroredMono).toBe(true);
    expect(Array.from(frame?.left || [])).toEqual([0.25, -0.5]);
    expect(Array.from(frame?.right || [])).toEqual([0.25, -0.5]);
  });

  it('keeps stereo when the right lane contains signal', () => {
    const buffer = buildFrame(1, 48000, 2, [0.25, 0.1, -0.5, -0.2]);
    const frame = decodeRxFrame(buffer);
    expect(frame?.kind).toBe('audio');
    if (frame?.kind !== 'audio') throw new Error('expected audio frame');
    expect(frame.mirroredMono).toBe(false);
    expect(Array.from(frame.right)).toEqual(
      expect.arrayContaining([
        expect.closeTo(0.1, 6),
        expect.closeTo(-0.2, 6),
      ]),
    );
  });

  it('decodes the bridge audio sequence from the header', () => {
    const buffer = buildFrame(1, 48000, 2, [0.25, 0, -0.5, 0], 19);
    const frame = decodeAudioFrame(buffer);
    expect(frame?.header.sequence).toBe(19);
  });

  it('decodes mono audio frames for low bandwidth transport', () => {
    const buffer = buildFrame(1, 12000, 1, [0.25, -0.5], 20);
    const frame = decodeAudioFrame(buffer);
    expect(frame?.header.sampleRate).toBe(12000);
    expect(frame?.header.channels).toBe(1);
    expect(frame?.mirroredMono).toBe(true);
    expect(Array.from(frame?.left || [])).toEqual([0.25, -0.5]);
    expect(Array.from(frame?.right || [])).toEqual([0.25, -0.5]);
  });
});
