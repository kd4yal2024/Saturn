import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

// Execute the shipped processor, not a second implementation of its queue.
const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
const constants: Record<string, number> = {
  RX_MSG_QUEUE_MAX_PACKETS: 8, CTRL_READ_IDX: 0, CTRL_WRITE_IDX: 1,
  CTRL_GAIN_IDX: 4, CTRL_MUTED_IDX: 5, TX_MIC_BLOCK_SAMPLES: 1024,
};
const source = template.split('const WORKLET_SOURCE = `')[1]!.split('`;')[0]!
  .replace(/\$\{([^}]+)\}/g, (_, key: string) => String(constants[key]));

function harness() {
  const messages: any[] = [];
  let Processor: any;
  const context = {
    sampleRate: 48000, currentTime: 0,
    AudioWorkletProcessor: class {
      port = { postMessage: (message: any) => messages.push(structuredClone(message)) };
    },
    registerProcessor: (name: string, ctor: any) => {
      if (name === 'saturn-rx-player') Processor = ctor;
    },
  };
  runInNewContext(source, context);
  const processor = new Processor();
  const tick = () => {
    const out = [new Float32Array(128), new Float32Array(128)];
    processor.process([], [out]);
    context.currentTime += 128 / 48000;
    return out;
  };
  const packet = (frames = 1024, id = 1) => processor._onMessage({
    type: 'audio', left: new Float32Array(frames).fill(0.5),
    right: new Float32Array(frames).fill(-0.5),
    diagnostic: { id, sentContextMs: context.currentTime * 1000 - 7 },
  });
  return { processor, messages, tick, packet, context };
}

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

  it('retains explicit flush context without changing underrun counting', () => {
    const h = harness();
    h.packet(); h.tick();
    h.processor._onMessage({ type: 'flush', reason: 'tx-mute' });
    h.tick(); h.tick();
    expect(h.processor.underruns).toBe(1);
    expect(h.processor.rxEpisode.missingFrames).toBe(256);
    expect(h.processor.rxEpisode.lastFlush.reason).toBe('tx-mute');
  });

  it('bounds histories and reports minimum occupancy between telemetry ticks', () => {
    const h = harness();
    for (let i = 0; i < 150; i++) h.packet(128, i);
    expect(h.processor.rxHistory).toHaveLength(96);
    expect(h.processor.msgQueue).toHaveLength(8);
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
