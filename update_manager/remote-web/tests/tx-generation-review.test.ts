import { describe, expect, it } from 'vitest';
import { TxGenerationReviewClient, tagPsg1, PSG1_MAGIC,
  type CaptureFactory, type FreshCapture } from '../src/transport/tx-generation-review';
const ONE = '00112233445566778899aabbccddeeff';
const TWO = 'ffeeddccbbaa99887766554433221100';
const offer = (token = ONE, serial = 1) => `saturn_tx_generation:psg1,${token},${serial};`;
class Encoder {
  static instances: Encoder[] = [];
  closed = false;
  encoded = 0;
  emitted = 0;
  flushed: (() => void) | null = null;
  constructor(readonly init: { output(chunk: unknown): void; error(error: unknown): void }) {
    Encoder.instances.push(this);
  }
  configure() {}
  encode() { this.encoded++; }
  flush() { return new Promise<void>(resolve => {
    if (this.emitted === this.encoded) resolve(); else this.flushed = resolve;
  }); }
  close() { this.closed = true; }
  emit(duration = 20_000) {
    this.init.output({ byteLength: 3, duration, copyTo(bytes: Uint8Array) { bytes.set([1, 2, 3]); } });
    this.emitted++;
    if (this.emitted === this.encoded) { this.flushed?.(); this.flushed = null; }
  }
}
class Data { constructor(_: unknown) {} close() {} }
function harness(codec: 'pcm' | 'opus_wb' = 'pcm', enabled = true, factory?: CaptureFactory) {
  Encoder.instances = [];
  let now = 0;
  const callbacks: Array<(s: Float32Array, oldest: number) => void> = [];
  const sent: ArrayBuffer[] = []; const events: string[] = [];
  const capture: CaptureFactory = factory ?? (async (token, callback) => {
    callbacks.push(callback);
    return { token, historyResetAtMs: now, close() { events.push('source-close'); } };
  });
  const client = new TxGenerationReviewClient({ codec, enabled, scope: { AudioEncoder: Encoder, AudioData: Data },
    now: () => now, capture,
    media(frame) { sent.push(frame); }, control(text) { events.push(text); } });
  return { client, sent, events, callbacks, setNow(value: number) { now = value; },
    input(index = 0, oldest = now) { callbacks[index]!(new Float32Array(codec === 'pcm' ? 512 : 960).fill(0.3), oldest); } };
}
describe('offline PSG1 remote client', () => {
  it('never selects production by default or without explicit review opt-in', async () => {
    const h = harness('pcm', false);
    expect(await h.client.generation(offer())).toBe(false);
    expect(h.callbacks).toHaveLength(0);
  });
  it('tags a copy without changing the old frame and uses exact header bytes', () => {
    const frame = new ArrayBuffer(64);
    const tagged = tagPsg1(frame, ONE);
    expect(new DataView(tagged).getUint32(60, true)).toBe(PSG1_MAGIC);
    expect([...new Uint8Array(tagged).slice(44, 60)]).toEqual(Array.from({ length: 16 }, (_, i) => parseInt(ONE.slice(i*2, i*2+2), 16)));
    expect(new Uint8Array(frame).every(b => b === 0)).toBe(true);
  });
  it('requires acknowledged replacement before accepting capture samples', async () => {
    let resolve!: (value: FreshCapture) => void;
    let callback!: (s: Float32Array, oldest: number) => void;
    const h = harness('pcm', true, (_, c) => { callback = c; return new Promise(r => { resolve = r; }); });
    const pending = h.client.generation(offer());
    callback(new Float32Array(512), 0);
    expect(h.sent).toHaveLength(0);
    resolve({ token: ONE, historyResetAtMs: 0, close() {} });
    expect(await pending).toBe(true);
    callback(new Float32Array(512).fill(.3), 0);
    expect(h.sent).toHaveLength(1);
  });
  it('rejects a source reset acknowledgement from before the requested boundary', async () => {
    const h = harness('pcm', true, async token => ({ token, historyResetAtMs: 0, close() {} }));
    h.setNow(10);
    expect(await h.client.generation(offer())).toBe(false);
    expect(h.events).toContain('trx:0,false;');
  });
  it('rejects retained input from before a fresh source even after new arrivals', async () => {
    const h = harness(); h.setNow(20); await h.client.generation(offer());
    h.setNow(40); h.input(0, 19);
    expect(h.sent).toHaveLength(0); expect(h.events[0]).toBe('trx:0,false;');
  });
  it('release invalidates first, sends unkey before cleanup, and refuses old callbacks', async () => {
    const h = harness(); await h.client.generation(offer()); h.input();
    h.client.release(); h.input();
    expect(h.sent).toHaveLength(1);
    expect(h.events).toEqual(['trx:0,false;', 'source-close']);
    await h.client.generation(offer(TWO, 2)); h.input(0); h.input(1);
    expect(h.sent).toHaveLength(2);
    expect(new Uint8Array(h.sent[1]!).slice(44, 60)).toEqual(new Uint8Array(tagPsg1(new ArrayBuffer(64), TWO)).slice(44, 60));
  });
  it('late source replacement cannot revive an old generation', async () => {
    const resolvers: Array<(s: FreshCapture) => void> = []; const closed: string[] = [];
    const h = harness('pcm', true, () => new Promise(r => resolvers.push(r)));
    const first = h.client.generation(offer()); const second = h.client.generation(offer(TWO, 2));
    resolvers[1]!({ token: TWO, historyResetAtMs: 0, close() { closed.push(TWO); } }); await second;
    resolvers[0]!({ token: ONE, historyResetAtMs: 0, close() { closed.push(ONE); } });
    expect(await first).toBe(false); expect(closed).toEqual([ONE]);
  });
  it('cannot revive retired offers or let a delayed off cancel the new generation', async () => {
    const h = harness(); await h.client.generation(offer()); h.client.release();
    expect(await h.client.generation(offer())).toBe(false);
    await h.client.generation(offer(TWO, 2));
    expect(await h.client.generation('saturn_tx_generation:psg1,off,1;')).toBe(false);
    h.input(1); expect(h.sent).toHaveLength(1);
  });
  it('recreates real producer/encoder instances every four inputs and orders asynchronous output', async () => {
    const h = harness('opus_wb'); await h.client.generation(offer());
    for (let i = 0; i < 8; i++) { h.setNow(i*20); h.input(); }
    expect(Encoder.instances).toHaveLength(2);
    Encoder.instances[1]!.emit(); expect(h.sent).toHaveLength(0);
    for (let i = 0; i < 4; i++) Encoder.instances[0]!.emit();
    expect(h.sent).toHaveLength(5);
    for (let i = 1; i < 4; i++) Encoder.instances[1]!.emit();
    await Promise.resolve();
    await Promise.resolve();
    expect(h.sent.map(f => new DataView(f).getUint32(32, true))).toEqual([0,1,2,3,4,5,6,7]);
    expect(Encoder.instances.every(e => e.closed)).toBe(true);
    Encoder.instances[0]!.emit(); expect(h.sent).toHaveLength(8); // no orphan sequence-zero ghost
    Encoder.instances[0]!.init.error('late error from closed bank');
    expect(h.events).toEqual([]);
  });
  it('retains oldest bank age across encoder delay and stops before stale output is sent', async () => {
    const h = harness('opus_wb'); await h.client.generation(offer()); h.input();
    h.setNow(100); h.input(); h.setNow(151); Encoder.instances[0]!.emit();
    expect(h.sent).toHaveLength(0); expect(h.events[0]).toBe('trx:0,false;');
  });
  it('fails closed on the measured Chrome four-input / five-output flush behavior', async () => {
    const h = harness('opus_wb'); await h.client.generation(offer());
    for (let i = 0; i < 4; i++) { h.setNow(i * 20); h.input(); }
    const bank = Encoder.instances[0]!;
    // All outputs are delivered before flush's promise continuations run,
    // matching the real Chrome experiment, not a packet-count assumption.
    for (let i = 0; i < 5; i++) bank.emit();
    await Promise.resolve(); await Promise.resolve();
    expect(h.sent).toHaveLength(4);
    expect(h.events[0]).toBe('trx:0,false;');
    expect(bank.closed).toBe(true);
    bank.emit(); h.input(); expect(h.sent).toHaveLength(4);
    await h.client.generation(offer(TWO, 2)); h.input(1);
    Encoder.instances[1]!.emit(); expect(h.sent).toHaveLength(5);
    expect(new DataView(h.sent[4]!).getUint32(32, true)).toBe(0);
  });
  it('old encoder callbacks cannot be restamped after re-arm', async () => {
    const h = harness('opus_wb'); await h.client.generation(offer()); h.input();
    const old = Encoder.instances[0]!;
    h.client.release(); await h.client.generation(offer(TWO, 2)); h.input(1);
    old.emit(); expect(h.sent).toHaveLength(0);
    Encoder.instances[1]!.emit(); expect(h.sent).toHaveLength(1);
    expect(new DataView(h.sent[0]!).getUint32(32, true)).toBe(0);
  });
  it('bounds queued banks and faults an unsolicited encoder output', async () => {
    const h = harness('opus_wb'); await h.client.generation(offer());
    for (let i = 0; i < 9; i++) h.input();
    expect(h.events[0]).toBe('trx:0,false;'); expect(h.sent).toHaveLength(0);
    const other = harness('opus_wb'); await other.client.generation(offer()); other.input();
    Encoder.instances[0]!.emit(); Encoder.instances[0]!.emit();
    expect(other.sent).toHaveLength(1); expect(other.events[0]).toBe('trx:0,false;');
  });
});
