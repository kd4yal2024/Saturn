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
    bridgeRttMs: s.bridgeRttMs,
    backpressureSafetyP50Us: s.backpressureSafetyP50Us,
    backpressureSafetyP95Us: s.backpressureSafetyP95Us,
    backpressureSafetyP99Us: s.backpressureSafetyP99Us,
    backpressureControlP50Us: s.backpressureControlP50Us,
    backpressureControlP95Us: s.backpressureControlP95Us,
    backpressureControlP99Us: s.backpressureControlP99Us,
    displayReplacedPerSec: s.displayReplacedPerSec,
    displayDroppedPerSec: s.displayDroppedPerSec,
    bridgeAudioDroppedPerSec: s.bridgeAudioDroppedPerSec,
    bridgeAudioSeqGapCount: s.bridgeAudioSeqGapCount,
    audioSeqGapCount: s.audioSeqGapCount,
    audioPanicDrainCount: s.audioPanicDrainCount,
    sendBlockedMs: s.sendBlockedMs,
    outboundHighWatermarkBytes: s.outboundHighWatermarkBytes,
    safetyQueueDepthOverflowCount: s.safetyQueueDepthOverflowCount,
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
    expect(snap.bridgeRttMs).toBeNull();
    expect(snap.backpressureSafetyP99Us).toBe(0);
    expect(snap.backpressureControlP99Us).toBe(0);
    expect(snap.outboundHighWatermarkBytes).toBe(0);
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
    source2.bridgeRttMs = 23;
    source2.backpressureSafetyP99Us = 4000;
    source2.backpressureControlP99Us = 9000;
    source2.displayReplacedPerSec = 5;
    source2.displayDroppedPerSec = 1;
    source2.bridgeAudioDroppedPerSec = 2;
    source2.bridgeAudioSeqGapCount = 3;
    source2.audioSeqGapCount = 4;
    source2.audioPanicDrainCount = 1;
    source2.sendBlockedMs = 8;
    source2.outboundHighWatermarkBytes = 12000;
    source2.safetyQueueDepthOverflowCount = 0;
    const s2 = buildPerfSnapshot(source2, 2000);

    const summary = buildPerfSummary([s1, s2], s2);
    expect(summary.sampleCount).toBe(2);
    expect(summary.avgFrameRate).toBe(15);
    expect(summary.maxFrameRate).toBe(30);
    expect(summary.minFrameRate).toBe(0);
    expect(summary.maxBridgeRttMs).toBe(23);
    expect(summary.maxBackpressureSafetyP99Us).toBe(4000);
    expect(summary.maxBackpressureControlP99Us).toBe(9000);
    expect(summary.totalDisplayReplaced).toBe(5);
    expect(summary.totalDisplayDropped).toBe(1);
    expect(summary.totalBridgeAudioDropped).toBe(2);
    expect(summary.finalBridgeAudioSeqGapCount).toBe(3);
    expect(summary.finalAudioSeqGapCount).toBe(4);
    expect(summary.totalAudioPanicDrainCount).toBe(1);
    expect(summary.totalSendBlockedMs).toBe(8);
    expect(summary.maxOutboundHighWatermarkBytes).toBe(12000);
    expect(summary.totalSafetyQueueDepthOverflowCount).toBe(0);
    expect(summary.finalAudioBackpressureDrops).toBe(2);
    expect(summary.finalRxWorkletDrops).toBe(1);
    expect(summary.finalSnapshot).toEqual(s2);
  });
});
