import { describe, expect, it } from 'vitest';
import { buildPerfSnapshot, buildPerfSummary } from '../src/state/perf-snapshot';
import { createAppState } from '../src/state/app-state';

function makeSource() {
  const s = createAppState();
  return {
    connected: s.connected,
    connectPending: s.connectPending,
    iqStreaming: s.iqStreaming,
    audioStreaming: s.audioStreaming,
    sampleRate: s.sampleRate,
    audioSampleRate: s.audioSampleRate,
    displayZoom: s.displayZoom,
    frameRate: s.frameRate,
    frameCounter: s.frameCounter,
    iqPackets: s.iqPackets,
    fftWidth: s.fftWidth,
    audioFramesPlayed: s.audioFramesPlayed,
    audioBackpressureDrops: s.audioBackpressureDrops,
    rxWorkletDrops: s.rxWorkletDrops,
    lastFrameAt: s.lastFrameAt,
    lastSpectrumRenderAt: s.lastSpectrumRenderAt,
    waterfallSettleFrames: s.waterfallSettleFrames,
    displayCaption: s.displayCaption,
  };
}

describe('buildPerfSnapshot', () => {
  it('returns expected fields from default state', () => {
    const snap = buildPerfSnapshot(makeSource(), 1000);
    expect(snap.nowMs).toBe(1000);
    expect(snap.connected).toBe(false);
    expect(snap.sampleRate).toBe(192000);
    expect(snap.audioSampleRate).toBe(48000);
    expect(snap.displayZoom).toBe(1);
    expect(snap.iqPackets).toBe(0);
    expect(snap.fftWidth).toBe(1024);
    expect(snap.iqIdleMs).toBe(1000);
    expect(snap.wallTime).toBeTruthy();
  });

  it('computes iqIdleMs from lastFrameAt', () => {
    const source = makeSource();
    source.lastFrameAt = 900;
    const snap = buildPerfSnapshot(source, 1000);
    expect(snap.iqIdleMs).toBe(100);
  });
});

describe('buildPerfSummary', () => {
  it('returns zeros for empty snapshots', () => {
    const current = buildPerfSnapshot(makeSource(), 1000);
    const summary = buildPerfSummary([], current);
    expect(summary.sampleCount).toBe(0);
    expect(summary.startedAt).toBeNull();
    expect(summary.avgFrameRate).toBe(0);
    expect(summary.finalSnapshot).toBe(current);
  });

  it('computes stats from snapshots', () => {
    const s1 = buildPerfSnapshot(makeSource(), 1000);
    const source2 = makeSource();
    source2.frameRate = 30;
    source2.audioBackpressureDrops = 2;
    source2.rxWorkletDrops = 1;
    const s2 = buildPerfSnapshot(source2, 2000);

    const summary = buildPerfSummary([s1, s2], s2);
    expect(summary.sampleCount).toBe(2);
    expect(summary.avgFrameRate).toBe(15);
    expect(summary.maxFrameRate).toBe(30);
    expect(summary.minFrameRate).toBe(0);
    expect(summary.finalAudioBackpressureDrops).toBe(2);
    expect(summary.finalRxWorkletDrops).toBe(1);
    expect(summary.finalSnapshot).toEqual(s2);
  });
});
