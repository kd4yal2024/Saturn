import { describe, expect, it } from 'vitest';
import { buildPerfSnapshot, buildPerfSummary } from '../src/state/perf-snapshot';
import { createAppState } from '../src/state/app-state';

function makeSource() {
  return createAppState();
}

describe('buildPerfSnapshot', () => {
  it('returns expected fields from default state', () => {
    const snap = buildPerfSnapshot(makeSource(), 1000);
    expect(snap.nowMs).toBe(1000);
    expect(snap.connected).toBe(false);
    expect(snap.sampleRate).toBe(192000);
    expect(snap.audioSampleRate).toBe(48000);
    expect(snap.audioChannels).toBe(2);
    expect(snap.networkProfile).toBe('lan');
    expect(snap.audioProfile).toContain('48 kHz stereo');
    expect(snap.displayZoom).toBe(1);
    expect(snap.iqPackets).toBe(0);
    expect(snap.fftWidth).toBe(1024);
    expect(snap.bridgeRttMs).toBeNull();
    expect(snap.backpressureSafetyP99Us).toBe(0);
    expect(snap.backpressureControlP99Us).toBe(0);
    expect(snap.outboundHighWatermarkBytes).toBe(0);
    expect(snap.browserMainLagP99Ms).toBe(0);
    expect(snap.browserRafIntervalP99Ms).toBe(0);
    expect(snap.txWorkletToMainP99Ms).toBe(0);
    expect(snap.txMainToSendP99Ms).toBe(0);
    expect(snap.txWsSendP99Ms).toBe(0);
    expect(snap.txTimingFrameCount).toBe(0);
    expect(snap.phase40DisplayProfile).toBe('');
    expect(snap.iqIdleMs).toBe(1000);
    expect(snap.audioIdleMs).toBe(1000);
    expect(snap.rxAudioJitterP99Ms).toBe(0);
    expect(snap.rxWorkletQueuedMs).toBe(0);
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
    source2.rxWorkletQueuedMs = 24;
    source2.rxWorkletUnderruns = 2;
    source2.rxWorkletOverflows = 1;
    source2.audioContextBaseLatencyMs = 5.5;
    source2.audioContextOutputLatencyMs = 11.25;
    source2.lastAudioFrameAt = 1980;
    source2.rxAudioJitterSamples = [1, 2, 12.5];
    source2.rxAudioJitterP50Ms = 2;
    source2.rxAudioJitterP95Ms = 12.5;
    source2.rxAudioJitterP99Ms = 12.5;
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
    source2.bridgeOutboundQueuedBytes = 4096;
    source2.bridgeTcpOutqHighWatermarkBytes = 8192;
    source2.wsMediaBacklogBytes = 512;
    source2.connectionLossCount = 1;
    source2.connectionRecoveryMs = 900;
    source2.safetyQueueDepthOverflowCount = 0;
    source2.browserMainLagP99Ms = 18.5;
    source2.browserRafIntervalP99Ms = 34.25;
    source2.txWorkletToMainP99Ms = 42.5;
    source2.txMainToSendP99Ms = 9.75;
    source2.txWsSendP99Ms = 1.25;
    source2.txTimingFrameCount = 187;
    source2.txTimingDroppedFrameCount = 2;
    source2.phase40DisplayProfile = 'D-text-only';
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
    expect(summary.finalRxWorkletUnderruns).toBe(2);
    expect(summary.finalRxWorkletOverflows).toBe(1);
    expect(summary.maxRxWorkletQueuedMs).toBe(24);
    expect(summary.maxRxAudioJitterP99Ms).toBe(12.5);
    expect(summary.maxWsMediaBacklogBytes).toBe(512);
    expect(summary.maxBridgeOutboundQueuedBytes).toBe(4096);
    expect(summary.maxConnectionRecoveryMs).toBe(900);
    expect(summary.maxBrowserMainLagP99Ms).toBe(18.5);
    expect(summary.maxBrowserRafIntervalP99Ms).toBe(34.25);
    expect(summary.maxTxWorkletToMainP99Ms).toBe(42.5);
    expect(summary.maxTxMainToSendP99Ms).toBe(9.75);
    expect(summary.maxTxWsSendP99Ms).toBe(1.25);
    expect(summary.totalTxTimingFrames).toBe(187);
    expect(summary.totalTxTimingDroppedFrames).toBe(2);
    expect(summary.phase40DisplayProfile).toBe('D-text-only');
    expect(summary.finalSnapshot).toEqual(s2);
  });
});
