import { describe, it, expect } from 'vitest';
import { resampleChannel, prepareAudioForPlayback, ringBufferFreeFrames } from '../src/audio/resample';
import {
  RX_AUDIO_FRAME_FLOATS,
  RX_RING_FRAMES,
  CTRL_READ_IDX,
  CTRL_WRITE_IDX,
} from '../src/audio/constants';

describe('resampleChannel', () => {
  it('returns empty output for empty input', () => {
    const result = resampleChannel(new Float32Array(0), 48000, 44100, 0);
    expect(result.length).toBe(0);
  });

  it('passes through when rates match', () => {
    const input = new Float32Array([0.1, 0.2, 0.3, 0.4]);
    const result = resampleChannel(input, 48000, 48000, 4);
    expect(result.length).toBe(4);
    expect(result[0]).toBeCloseTo(0.1);
    expect(result[3]).toBeCloseTo(0.4);
  });

  it('upsamples correctly', () => {
    const input = new Float32Array([0.0, 1.0]);
    const result = resampleChannel(input, 1, 2, 4);
    expect(result.length).toBe(4);
    expect(result[0]).toBeCloseTo(0.0);
    expect(result[3]).toBeCloseTo(1.0);
  });

  it('downsamples correctly', () => {
    const input = new Float32Array([0.0, 0.5, 1.0, 0.5]);
    const result = resampleChannel(input, 2, 1, 2);
    expect(result.length).toBe(2);
  });
});

describe('prepareAudioForPlayback', () => {
  it('returns original data when rates match', () => {
    const left = new Float32Array([0.1, 0.2]);
    const right = new Float32Array([0.3, 0.4]);
    const result = prepareAudioForPlayback(left, right, 2, 48000, 48000);
    expect(result.left).toBe(left);
    expect(result.right).toBe(right);
    expect(result.frames).toBe(2);
    expect(result.sampleRate).toBe(48000);
  });

  it('resamples when rates differ', () => {
    const left = new Float32Array(480);
    const right = new Float32Array(480);
    const result = prepareAudioForPlayback(left, right, 480, 48000, 44100);
    expect(result.frames).toBe(441);
    expect(result.sampleRate).toBe(44100);
    expect(result.left.length).toBe(441);
  });
});

describe('ringBufferFreeFrames', () => {
  it('returns capacity-1 when empty', () => {
    expect(ringBufferFreeFrames(0, 0, 4096)).toBe(4095);
  });

  it('returns 0 when full', () => {
    expect(ringBufferFreeFrames(0, 4095, 4096)).toBe(0);
  });

  it('handles wrap-around', () => {
    expect(ringBufferFreeFrames(3000, 1000, 4096)).toBe(1999);
  });
});

describe('audio constants', () => {
  it('has expected values', () => {
    expect(RX_AUDIO_FRAME_FLOATS).toBe(2048);
    expect(RX_RING_FRAMES).toBe(4096);
    expect(CTRL_READ_IDX).toBe(0);
    expect(CTRL_WRITE_IDX).toBe(1);
  });
});
