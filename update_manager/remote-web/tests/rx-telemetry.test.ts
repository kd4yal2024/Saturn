import { describe, expect, it } from 'vitest';
import {
  audioFramesToMilliseconds,
  formatRxLatencyDiagnostic,
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

  it('formats a paste-ready RX latency diagnostic', () => {
    const text = formatRxLatencyDiagnostic({
      capturedAtIso: '2026-07-13T11:07:20.000Z',
      connection: 'Connected',
      bridgeUrl: 'wss://192.168.0.139:8443/tci',
      networkProfile: 'LAN',
      audioProfile: 'PCM F32 48 kHz stereo / Worklet',
      bridgeRttMs: 4,
      jitterP50Ms: 1.2,
      jitterP95Ms: 8.4,
      jitterP99Ms: 21.3,
      jitterSampleCount: 512,
      audioQueueMs: 42.7,
      workletUnderruns: 3,
      workletOverflows: 1,
      audioDropEvents: 2,
      audioContextBaseLatencyMs: 10,
      audioContextOutputLatencyMs: null,
      audioFrameAgeMs: 18,
      iqFrameAgeMs: 7,
      bridgeBacklogBytes: 4096,
      browserBacklogBytes: 0,
      bridgeHighWaterBytes: 8192,
      tcpHighWaterBytes: 2048,
      connectionRecoveryMs: null,
      connectionLossCount: 0,
      audioSequenceGaps: 1,
    });

    expect(text).toContain('Saturn Remote RX Latency');
    expect(text).toContain('Profiles: LAN / PCM F32 48 kHz stereo / Worklet');
    expect(text).toContain('Packet jitter p50/p95/p99: 1.2 / 8.4 / 21.3 ms (512 samples)');
    expect(text).toContain('Worklet underruns/overflows: 3 / 1');
    expect(text).toContain('AudioContext base/output: 10.0 ms / unavailable');
    expect(text).toContain('Media backlog bridge/browser: 4096 B / 0 B');
  });
});
