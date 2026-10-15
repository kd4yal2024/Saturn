import test from 'node:test';
import assert from 'node:assert/strict';
import { pcmFrame, validateRun, run } from '../scripts/ps-measurement-client.mjs';
const pin = 'a'.repeat(64);
const options = { url: 'ws://127.0.0.1:50123/tci', sourcePin: pin, durationMs: 80, frequencyHz: 7200000, confirm: true, emit() {} };
test('tagged PCM matches TCI header, finite two-tone samples and original generation', () => {
  const token = '0123456789abcdef0123456789abcdef';
  const b = pcmFrame(token, 4);
  assert.equal(b.length, 2112);
  for (const [offset, expected] of [[0, 0], [4, 48000], [8, 3], [20, 512], [24, 2], [28, 1], [32, 4], [40, 2048], [60, 0x50534731]])
    assert.equal(b.readUInt32LE(offset), expected);
  assert.equal(b.subarray(44, 60).toString('hex'), token);
  for (let i = 64; i < b.length; i += 4) assert.ok(Number.isFinite(b.readFloatLE(i)) && Math.abs(b.readFloatLE(i)) <= 0.400001);
  assert.throws(() => pcmFrame(token, -1));
  assert.throws(() => pcmFrame('bad', 0));
});
test('explicit run requires reviewed pin, bounded duration, loopback and attended load', () => {
  validateRun(options);
  for (const patch of [{confirm:false}, {sourcePin:'unverified'}, {durationMs:0}, {durationMs:1001},
    {frequencyHz:50100000}, {url:'ws://192.168.0.139:50001/tci'}, {url:'ws://127.0.0.1:50001/tci?arm=1'}])
    assert.throws(() => validateRun({...options, ...patch}));
});
class FakeSocket extends EventTarget {
  static OPEN = 1;
  static mode = 'healthy';
  static last;
  readyState = 1; bufferedAmount = 0; messages = []; frames = [];
  constructor() {
    super(); FakeSocket.last=this;
    queueMicrotask(() => {
      this.dispatchEvent(new Event('open'));
      this.text(`remote_client_role:0,${FakeSocket.mode === 'viewer' ? 'viewer' : 'operator'},1;`);
      if (FakeSocket.mode !== 'legacy') this.text(`saturn_ps_measurement_caps:0,true,1000,rx-bypass,${FakeSocket.mode === 'pin' ? 'b'.repeat(64) : pin},native,7200000;`);
      this.text(`tx_enable:0,${FakeSocket.mode !== 'inhibited'};`);
      this.text('saturn_tx_owner:shared,1,ReadyOff,1,1,tci,true,none;ready;tx_codec_accept:0,pcm;');
    });
  }
  text(data) { if (this.readyState === 1) this.dispatchEvent(new MessageEvent('message', {data})); }
  send(data) {
    if (typeof data !== 'string') { this.frames.push(data); return; }
    this.messages.push(data);
    if (data === 'drive:0,1;') queueMicrotask(() => this.text('tx_config:0,1,queued,2;tx_config:0,1,applied,2;'));
    if (data.startsWith('saturn_ps_measurement:0,start,')) {
      queueMicrotask(() => this.text('saturn_ps_measurement:0,1,queued;saturn_ps_measurement:0,1,admitted;saturn_tx_generation:psg1,0123456789abcdef0123456789abcdef,1;'));
      setTimeout(() => this.text('saturn_tx_generation:psg1,off,2;tx_fault:0,psg1,GenerationMismatch;saturn_ps_measurement:0,1,complete,3,none;'), 40);
    }
    if (data === 'saturn_ps_measurement:0,export,1;') queueMicrotask(() => this.text('saturn_ps_measurement:0,1,exported,/private/measurement-1-1.json;'));
  }
  close() { this.readyState=3; this.dispatchEvent(new Event('close')); }
}
test('fresh action streams tagged PCM, then exports only after complete without ordinary PTT', async () => {
  const original=globalThis.WebSocket; globalThis.WebSocket=FakeSocket;
  try {
    FakeSocket.mode='healthy';
    const result=await run(options);
    assert.equal(result.fault, false);
    assert.ok(FakeSocket.last.frames.length > 0);
    assert.ok(FakeSocket.last.frames.every(b => b.subarray(44,60).toString('hex') === '0123456789abcdef0123456789abcdef'));
    assert.equal(FakeSocket.last.messages.filter(s => s.startsWith('saturn_ps_measurement:0,start,')).length, 1);
    assert.ok(!FakeSocket.last.messages.some(s => s.startsWith('trx:')));
  } finally { globalThis.WebSocket=original; }
});
test('viewer, legacy, inhibited and wrong-pin sessions never receive a measurement action', async () => {
  const original=globalThis.WebSocket; globalThis.WebSocket=FakeSocket;
  try {
    for (const mode of ['viewer','legacy','inhibited','pin']) {
      FakeSocket.mode=mode;
      await assert.rejects(run(options));
      assert.equal(FakeSocket.last.frames.length, 0);
      assert.ok(!FakeSocket.last.messages.some(s => s.startsWith('saturn_ps_measurement:0,start,')));
    }
  } finally { globalThis.WebSocket=original; }
});
