import { describe, expect, it } from 'vitest';
import {
  audioFramesToMilliseconds,
  percentileMs,
  rxAudioArrivalJitterMs,
  summarizeRxAudioJitter,
} from '../src/audio/rx-telemetry';

describe('RX audio telemetry', () => {
  it('computes nearest-rank latency percentiles', () => {
    const values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 20];
    expect(percentileMs(values, 50)).toBe(5);
    expect(percentileMs(values, 95)).toBe(20);
    expect(percentileMs(values, 99)).toBe(20);
    expect(summarizeRxAudioJitter(values)).toEqual({
      p50Ms: 5,
      p95Ms: 20,
      p99Ms: 20,
      sampleCount: 10,
    });
  });

  it('measures arrival error relative to packet duration', () => {
    expect(rxAudioArrivalJitterMs(1022, 1000, 20)).toBe(2);
    expect(rxAudioArrivalJitterMs(1018, 1000, 20)).toBe(2);
    expect(rxAudioArrivalJitterMs(1000, 0, 20)).toBeNull();
  });

  it('converts worklet queue frames to milliseconds', () => {
    expect(audioFramesToMilliseconds(1152, 48000)).toBe(24);
    expect(audioFramesToMilliseconds(0, 48000)).toBe(0);
    expect(audioFramesToMilliseconds(128, 0)).toBe(0);
  });
});
