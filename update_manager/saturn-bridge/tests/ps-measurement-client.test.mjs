import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
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
    {commandedWatts:0}, {commandedWatts:6}, {commandedWatts:1.5}, {commandedWatts:NaN},
    {frequencyHz:50100000}, {url:'ws://192.168.0.139:50001/tci'}, {url:'ws://127.0.0.1:50001/tci?arm=1'}])
    assert.throws(() => validateRun({...options, ...patch}));
  validateRun({...options, commandedWatts:5});
  validateRun({...options, summaryOnly:true, durationMs:45000, commandedWatts:5});
  assert.throws(() => validateRun({...options, summaryOnly:true, durationMs:45001}));
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
      if (FakeSocket.mode !== 'legacy') this.text(`saturn_ps_measurement_caps:0,true,${FakeSocket.mode === 'summary' ? '45000' : '1000'},rx-bypass,${FakeSocket.mode === 'pin' ? 'b'.repeat(64) : pin},native,7200000,${FakeSocket.mode === 'summary' ? 'summary-only' : 'raw'};`);
      this.text(`tx_enable:0,${FakeSocket.mode !== 'inhibited'};`);
      this.text('saturn_tx_owner:shared,1,ReadyOff,1,1,tci,true,none;ready;tx_codec_accept:0,pcm;');
    });
  }
  text(data) { if (this.readyState === 1) this.dispatchEvent(new MessageEvent('message', {data})); }
  send(data) {
    if (typeof data !== 'string') { this.frames.push(data); return; }
    this.messages.push(data);
    if (/^drive:0,[1-5];$/.test(data)) queueMicrotask(() => this.text('tx_config:0,1,queued,2;tx_config:0,1,applied,2;'));
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
    assert.ok(FakeSocket.last.messages.includes('drive:0,1;'));
    assert.ok(FakeSocket.last.frames.length > 0);
    assert.ok(FakeSocket.last.frames.every(b => b.subarray(44,60).toString('hex') === '0123456789abcdef0123456789abcdef'));
    assert.equal(FakeSocket.last.messages.filter(s => s.startsWith('saturn_ps_measurement:0,start,')).length, 1);
    assert.ok(!FakeSocket.last.messages.some(s => s.startsWith('trx:')));
  } finally { globalThis.WebSocket=original; }
});
test('explicit 5 W remains bounded and reports commanded rather than measured power', async () => {
  const original=globalThis.WebSocket; globalThis.WebSocket=FakeSocket;
  const evidence=[];
  try {
    FakeSocket.mode='healthy';
    const result=await run({...options, commandedWatts:5, emit: record => evidence.push(record)});
    assert.equal(result.fault, false);
    assert.ok(FakeSocket.last.messages.includes('drive:0,5;'));
    assert.ok(!FakeSocket.last.messages.includes('drive:0,1;'));
    const request=evidence.find(record => record.event === 'fresh-measurement-request');
    assert.equal(request.commandedWatts, 5);
    assert.equal(request.externalCouplingDb, null);
    assert.equal(request.rfImdQualified, false);
    assert.equal(FakeSocket.last.messages.filter(s => s.startsWith('saturn_ps_measurement:0,start,')).length, 1);
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
test('45-second summary stimulus is explicit and cannot silently replace raw capture', async () => {
  const original=globalThis.WebSocket; globalThis.WebSocket=FakeSocket;
  const evidence=[];
  try {
    FakeSocket.mode='summary';
    await assert.rejects(run(options));
    assert.ok(!FakeSocket.last.messages.some(s => s.startsWith('saturn_ps_measurement:0,start,')));
    FakeSocket.mode='healthy';
    await assert.rejects(run({...options, summaryOnly:true, durationMs:45000}));
    assert.ok(!FakeSocket.last.messages.some(s => s.startsWith('saturn_ps_measurement:0,start,')));
    FakeSocket.mode='summary';
    const result=await run({...options, summaryOnly:true, durationMs:45000, commandedWatts:5, emit:r=>evidence.push(r)});
    assert.equal(result.fault,false);
    assert.equal(result.rawFeedbackRequested,false);
    assert.ok(FakeSocket.last.messages.includes('saturn_ps_measurement:0,start,45000;'));
    assert.equal(evidence.find(r=>r.event==='fresh-measurement-request').rawFeedbackRequested,false);
  } finally {globalThis.WebSocket=original;}
});

class Clock {
  now = 0; next = 0; timers = new Map(); callbacks = [];
  setTimeout = (callback, delay) => {
    const id = ++this.next;
    this.timers.set(id, { due: this.now + delay, callback });
    this.callbacks.push(callback);
    return id;
  };
  clearTimeout = id => this.timers.delete(id);
  advance(ms) {
    const end = this.now + ms;
    for (;;) {
      const next = [...this.timers].sort((a,b) => a[1].due - b[1].due || a[0] - b[0])[0];
      if (!next || next[1].due > end) break;
      this.now = next[1].due; this.timers.delete(next[0]); next[1].callback();
    }
    this.now = end;
  }
}
class ControlledSocket extends EventTarget {
  static OPEN = 1; static last;
  readyState = 1; bufferedAmount = 0; messages = []; frames = [];
  constructor() { super(); ControlledSocket.last = this; }
  text(data) { if (this.readyState === 1) this.dispatchEvent(new MessageEvent('message', {data})); }
  send(data) { (typeof data === 'string' ? this.messages : this.frames).push(data); }
  close() { this.readyState = 3; this.dispatchEvent(new Event('close')); }
}
function controlledRun(patch = {}) {
  const clock = new Clock(), signals = new EventEmitter(), evidence = [];
  const promise = run({...options, ...patch, emit:r => evidence.push(r)}, {
    WebSocket:ControlledSocket, setTimeout:clock.setTimeout, clearTimeout:clock.clearTimeout,
    now:() => clock.now, signals,
  });
  const socket = ControlledSocket.last;
  socket.dispatchEvent(new Event('open'));
  socket.text(`remote_client_role:0,operator,1;saturn_ps_measurement_caps:0,true,${patch.summaryOnly ? '45000' : '1000'},rx-bypass,${pin},native,7200000,${patch.summaryOnly ? 'summary-only' : 'raw'};`);
  socket.text('tx_enable:0,true;saturn_tx_owner:shared,1,ReadyOff,1,1,tci,true,none;ready;tx_codec_accept:0,pcm;');
  socket.text('tx_config:0,1,queued,2;tx_config:0,1,applied,2;');
  socket.text('saturn_ps_measurement:0,1,queued;saturn_ps_measurement:0,1,admitted;saturn_tx_generation:psg1,0123456789abcdef0123456789abcdef,1;');
  return {clock, signals, evidence, socket, promise};
}
const stoppedCommands = s => s.messages.filter(m => m === 'saturn_ps_measurement:0,stop;');
const exportCommands = s => s.messages.filter(m => m === 'saturn_ps_measurement:0,export,1;');
const complete = (s, fault = 'none', count = '3') => s.text(`saturn_tx_generation:psg1,off,2;saturn_ps_measurement:0,1,complete,${count},${fault};`);
const exported = s => s.text('saturn_ps_measurement:0,1,exported,/private/measurement-1-1.json;');

test('normal duration stop awaits delayed Bridge cleanup/export without inventing a fault', async () => {
  const {clock, socket, promise, evidence} = controlledRun({summaryOnly:true, durationMs:45000});
  clock.advance(45000);
  socket.text('saturn_tx_generation:psg1,off,2;tx_fault:0,psg1,GenerationMismatch;');
  clock.advance(200);
  assert.equal(stoppedCommands(socket).length, 1);
  assert.equal(exportCommands(socket).length, 0);
  assert.equal(socket.readyState, 1, 'duration stop must not close before cleanup evidence');
  const frames = socket.frames.length;
  assert.ok(frames > 4000, 'exercise the actual 45-second tagged PCM cadence');
  clock.advance(3000);
  assert.equal(socket.frames.length, frames);
  complete(socket);
  assert.equal(exportCommands(socket).length, 1);
  exported(socket);
  const result = await promise;
  assert.equal(result.fault, false);
  assert.equal(result.stopReason, 'local duration bound');
  assert.equal(evidence.find(r => r.event === 'stop-requested').fault, false);
  assert.equal(clock.timers.size, 0);
});
test('a genuine fault after duration stop stays failed even when cleanup/export succeed', async () => {
  const {clock, socket, promise} = controlledRun();
  const rejected = assert.rejects(promise, /diagnostic fault; no automatic retry/);
  clock.advance(280);
  socket.text('tx_fault:0,psg1,QueueOverrun;');
  complete(socket, 'QueueOverrun'); exported(socket);
  await rejected;
  assert.equal(stoppedCommands(socket).length, 1);
  assert.equal(socket.messages.filter(m => m.startsWith('saturn_ps_measurement:0,start,')).length, 1);
});
test('missing completion remains bounded and cannot become success from an unsolicited export', async () => {
  const {clock, socket, promise} = controlledRun();
  const rejected = assert.rejects(promise, /Cleanup\/export not verified/);
  clock.advance(280); exported(socket);
  assert.equal(socket.readyState, 1);
  clock.advance(14999);
  assert.equal(socket.readyState, 1);
  clock.advance(1);
  await rejected;
  assert.equal(socket.readyState, 3);
  assert.equal(exportCommands(socket).length, 0);
  assert.equal(stoppedCommands(socket).length, 1);
  assert.equal(clock.timers.size, 0);
});
test('operator release stops once, rejects an interrupted run, and cannot resume on stale callbacks', async () => {
  const {clock, socket, signals, promise} = controlledRun();
  const rejected = assert.rejects(promise, /diagnostic fault; no automatic retry/);
  clock.advance(20);
  const pendingPcm = [...clock.timers.values()].filter(t => t.due < 100).map(t => t.callback);
  assert.ok(pendingPcm.length > 0);
  signals.emit('SIGINT'); signals.emit('SIGTERM');
  const frames = socket.frames.length;
  socket.text('saturn_tx_generation:psg1,0123456789abcdef0123456789abcdef,1;');
  for (const callback of pendingPcm) callback();
  clock.advance(100);
  assert.equal(socket.frames.length, frames);
  complete(socket); exported(socket);
  await rejected;
  assert.equal(stoppedCommands(socket).length, 1);
});
test('stale generation and canceled PCM callbacks cannot refresh duration or restart output', async () => {
  const {clock, socket, promise} = controlledRun();
  clock.advance(270);
  const pendingPcm = [...clock.timers.values()].filter(t => t.due < 280).map(t => t.callback);
  assert.ok(pendingPcm.length > 0);
  clock.advance(10);
  const frames = socket.frames.length;
  for (const callback of pendingPcm) callback();
  socket.text('saturn_tx_generation:psg1,0123456789abcdef0123456789abcdef,1;');
  socket.text('saturn_ps_measurement:0,99,queued;saturn_ps_measurement:0,99,complete,3,none;');
  clock.advance(2000);
  assert.equal(socket.frames.length, frames);
  assert.equal(exportCommands(socket).length, 0);
  complete(socket); complete(socket); exported(socket);
  assert.equal((await promise).fault, false);
  assert.equal(exportCommands(socket).length, 1);
  assert.equal(stoppedCommands(socket).length, 1);
});
test('completion with zero or malformed capture counts remains a failed diagnostic', async () => {
  for (const count of ['0', 'NaN', '-1', '1.5']) {
    const {socket, promise} = controlledRun();
    const rejected = assert.rejects(promise, /diagnostic fault; no automatic retry/);
    complete(socket, 'none', count); exported(socket);
    await rejected;
  }
});
test('obsolete phase deadlines cannot fault a normal stop or extend the current bound', async () => {
  const {clock, socket, promise} = controlledRun();
  const [oldTransactionDeadline, oldMeasurementDeadline] = clock.callbacks;
  clock.advance(280);
  const currentCleanupDeadline = [...clock.timers.values()][0].callback;
  oldTransactionDeadline(); oldMeasurementDeadline();
  assert.equal(stoppedCommands(socket).length, 1);
  assert.equal(socket.readyState, 1);
  complete(socket);
  currentCleanupDeadline();
  assert.equal(socket.readyState, 1);
  exported(socket);
  assert.equal((await promise).fault, false);
});
test('actual client scheduling fault stops before duration and cannot be cleared by a clean receipt', async () => {
  const {clock, socket, promise} = controlledRun();
  const rejected = assert.rejects(promise, /diagnostic fault; no automatic retry/);
  socket.bufferedAmount = 8193;
  clock.advance(11);
  assert.equal(stoppedCommands(socket).length, 1);
  const frames = socket.frames.length;
  clock.advance(100);
  assert.equal(socket.frames.length, frames);
  complete(socket); exported(socket);
  await rejected;
});
test('completed cleanup without export still times out rather than reporting success', async () => {
  const {clock, socket, promise} = controlledRun();
  const rejected = assert.rejects(promise, /Cleanup\/export not verified/);
  complete(socket);
  clock.advance(14999);
  assert.equal(socket.readyState, 1);
  clock.advance(1);
  await rejected;
  assert.equal(socket.readyState, 3);
  assert.equal(clock.timers.size, 0);
});
