import { describe, expect, it } from 'vitest';
import { rxWorkletHarness } from './helpers/rx-worklet';
const harness = () => rxWorkletHarness(false);

describe('RX underrun recorder', () => {
  it('does not report pre-stream silence and preserves stereo output', () => {
    const h = harness();
    h.tick();
    expect(h.processor.rxEpisode).toBeNull();
    h.packet();
    const out = h.tick();
    expect(Array.from(out[0]!)).toEqual(Array(128).fill(0.5));
    expect(Array.from(out[1]!)).toEqual(Array(128).fill(-0.5));
  });

  it('records a single missing sample and recovery with delivery metadata', () => {
    const h = harness();
    h.packet(1023);
    for (let i = 0; i < 8; i++) h.tick();
    expect(h.processor.underruns).toBe(1);
    expect(h.processor.rxEpisode.missingFrames).toBe(1);
    h.packet(); h.tick();
    const event = h.processor.rxEpisodes[0];
    expect(event.missingMs).toBeCloseTo(1000 / 48000);
    expect(event.before[0].deliveryContextDelayMs).toBeCloseTo(7);
    expect(event.recovery).toHaveLength(2);
    expect(event.before[0].left).toBeUndefined();
  });

  it('treats explicit flush as an idle boundary instead of a network underrun', () => {
    const h = harness();
    h.packet(); h.tick();
    h.processor._onMessage({ type: 'flush', reason: 'tx-mute' });
    h.tick(); h.tick();
    expect(h.processor.underruns).toBe(0);
    expect(h.processor.rxEpisode).toBeNull();
    expect(h.processor.phase).toBe('idle');
    h.packet(128); h.tick(); h.tick();
    expect(h.processor.rxEpisode.lastFlush.reason).toBe('tx-mute');
  });

  it('bounds histories and reports minimum occupancy between telemetry ticks', () => {
    const h = harness();
    for (let i = 0; i < 150; i++) h.packet(128, i);
    expect(h.processor.rxHistory).toHaveLength(96);
    expect(h.processor._queuedFrames()).toBeLessThanOrEqual(5760);
    for (let i = 0; i < 100; i++) h.tick();
    const diagnostic = h.messages[0].diagnostic;
    expect(diagnostic.queueMinFrames).toBe(0);
    expect(diagnostic.activeEpisode.before).toHaveLength(48);
    expect(diagnostic.missingFrames).toBeGreaterThan(0);
  });

  it('bounds completed events and drains them after publication', () => {
    const h = harness();
    for (let i = 0; i < 12; i++) { h.packet(128); h.tick(); h.tick(); }
    h.packet(128); h.tick();
    expect(h.processor.rxEpisodes).toHaveLength(8);
    h.processor._publishTelemetry(48000);
    expect(h.messages.at(-1).diagnostic.episodes).toHaveLength(8);
    expect(h.processor.rxEpisodes).toHaveLength(0);
  });
});
