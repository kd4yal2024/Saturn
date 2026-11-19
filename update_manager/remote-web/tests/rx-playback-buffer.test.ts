import { describe, expect, it } from 'vitest';
import { rxWorkletHarness } from './helpers/rx-worklet';

/** Arrival schedule and audio rendering use separate deterministic clocks. */
function replay(bounded: boolean, arrivals: number[], totalMs: number) {
  const h = rxWorkletHarness(bounded); let at = 0, audibleAt: number | null = null, maxQueue = 0;
  while (h.context.currentTime * 1000 < totalMs) {
    while (at < arrivals.length && arrivals[at]! <= h.context.currentTime * 1000 + 0.001) h.packet(960, at++);
    const before = h.context.currentTime * 1000;
    maxQueue = Math.max(maxQueue, h.processor._queuedFrames());
    const out = h.tick();
    if (audibleAt === null && out[0]!.some(n => n !== 0)) audibleAt = before;
    expect(h.processor._queuedFrames()).toBeLessThanOrEqual(5760);
  }
  return { ...h, audibleAt, maxQueue };
}

describe('bounded RX playback of actual shipped worklet', () => {
  it('primes 48 ms without calling startup silence a network underrun', () => {
    const h = rxWorkletHarness(); h.tick(); h.packet(960);
    for (let i = 0; i < 8; ++i) expect(h.tick()[0]!.every(x => x === 0)).toBe(true);
    expect(h.processor.underruns).toBe(0); expect(h.processor.rxMissingFrames).toBe(0);
    h.packet(960); expect(h.tick()[0]![0]).toBe(0);
    h.packet(960); const out = h.tick();
    expect(out[0]![0]).toBe(0.5); expect(out[1]![0]).toBe(-0.5);
    expect(h.processor.phase).toBe('playing');
  });

  it('absorbs repeatable paired-packet bursts that underrun immediate playback', () => {
    const arrivals = Array.from({ length: 100 }, (_, i) => i === 0 ? 0 : Math.ceil(i / 2) * 40);
    const immediate = replay(false, arrivals, 1900), buffered = replay(true, arrivals, 1900);
    expect(immediate.processor.rxMissingFrames).toBeGreaterThan(0);
    expect(buffered.processor.rxMissingFrames).toBe(0);
    expect(buffered.processor.underruns).toBe(0);
    expect(buffered.audibleAt).toBeGreaterThanOrEqual(40);
    expect(buffered.audibleAt).toBeLessThanOrEqual(60);
  });

  it('does not hide a long outage and recovers with one bounded re-prime', () => {
    const arrivals = Array.from({ length: 80 }, (_, i) => i * 20 + (i >= 20 ? 180 : 0));
    const h = replay(true, arrivals, 1650);
    expect(h.processor.underruns).toBe(1); expect(h.processor.rebuffers).toBe(1);
    expect(h.processor.targetMs).toBe(64); expect(h.processor.phase).toBe('playing');
    expect(h.processor.rxMissingFrames).toBeGreaterThan(0);
    expect(h.messages.flatMap(m => m.diagnostic.episodes).some((e: any) => e.bufferingFrames > 0)).toBe(true);
  });

  it('caps burst backlog by frames, retains the newest stereo samples and reports trims', () => {
    const h = rxWorkletHarness();
    h.samples(Float32Array.from({ length: 8000 }, (_, i) => i), Float32Array.from({ length: 8000 }, (_, i) => -i));
    expect(h.processor._queuedFrames()).toBe(5760);
    expect(h.processor.trimmedFrames).toBe(2240);
    const out = h.tick(); expect(out[0]![0]).toBe(2240); expect(out[1]![0]).toBe(-2240);
    for (let i = 0; i < 100; i++) h.packet(960);
    expect(h.processor._queuedFrames()).toBe(5760);
    expect(h.processor.rxHistory).toHaveLength(96);
  });

  it('flushes old samples and re-primes without counting an intentional stop as jitter', () => {
    const h = rxWorkletHarness(); h.packet(3000); h.tick();
    h.processor._onMessage({ type: 'flush', reason: 'band-recall' });
    for (let i = 0; i < 10; i++) h.tick();
    expect(h.processor.underruns).toBe(0); expect(h.processor._queuedFrames()).toBe(0);
    expect(h.processor.phase).toBe('idle');
    h.samples(new Float32Array(2304).fill(0.25), new Float32Array(2304).fill(-0.25));
    const out = h.tick(); expect(out[0]![0]).toBe(0.25); expect(out[1]![0]).toBe(-0.25);
  });

  it('caps adaptation at 80 ms and decreases only after stable playback', () => {
    const h = rxWorkletHarness();
    for (let over = 0; over < 4; over++) {
      h.packet(4000); for (let i = 0; i < 40; i++) h.tick();
    }
    expect(h.processor.targetMs).toBe(80);
    h.packet(4000); h.tick();
    for (let i = 0; i < 11400; i++) { h.packet(128); h.tick(); }
    expect(h.processor.targetMs).toBe(76);
    expect(h.processor._queuedFrames()).toBeLessThanOrEqual(5760);
  });

  it('uses browser sample rate for bounded capacity and priming', () => {
    const h = rxWorkletHarness(true, 44100); h.packet(2117); const out = h.tick();
    expect(out[0]![0]).toBe(0.5); expect(h.processor.maxQueueFrames).toBe(5292);
  });

  it('keeps an immediate-playback comparison path while retaining the queue cap', () => {
    const h = rxWorkletHarness(false); h.packet(128); expect(h.tick()[0]![0]).toBe(0.5);
    h.tick(); expect(h.processor.underruns).toBe(1); expect(h.processor.targetMs).toBe(0);
  });

  it('primes the optional SAB path after initial publication and an intentional flush', () => {
    const h = rxWorkletHarness(), ring = new Float32Array(new SharedArrayBuffer(4096 * 2 * 4));
    const ctrl = new Uint32Array(new SharedArrayBuffer(32)); new Float32Array(ctrl.buffer)[4] = 1;
    h.processor._onMessage({ type: 'init-sab', ringBuf: ring.buffer, ctrlBuf: ctrl.buffer });
    for (let i = 0; i < 3000; i++) { ring[i * 2] = 0.5; ring[i * 2 + 1] = -0.5; }
    Atomics.store(ctrl, 1, 960); expect(h.tick()[0]![0]).toBe(0);
    Atomics.store(ctrl, 1, 3000); expect(h.tick()[0]![0]).toBe(0.5);
    Atomics.store(ctrl, 0, Atomics.load(ctrl, 1));
    h.processor._onMessage({ type: 'flush', reason: 'source-change', clearSab: false });
    expect(h.processor._queuedFrames()).toBe(0);
    Atomics.store(ctrl, 0, 0); Atomics.store(ctrl, 1, 3000);
    expect(h.tick()[1]![0]).toBe(-0.5); expect(h.processor.underruns).toBe(0);
  });

  it('exports reserve, recovery and intentional-hold metadata without PCM', () => {
    const h = replay(true, [0, 40, 40, 80, 80], 250);
    const telemetry = h.messages.at(-1);
    expect(telemetry.playout.targetMs).toBeLessThanOrEqual(80);
    expect(telemetry.playout.maxQueueMs).toBe(120);
    expect(telemetry.diagnostic.activeEpisode.before[0].left).toBeUndefined();
  });
});
