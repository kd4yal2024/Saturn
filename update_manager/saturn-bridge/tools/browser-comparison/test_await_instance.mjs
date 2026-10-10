// Tests for await_instance.mjs: the evidence, not the operator's word, says when the wanted Bridge instance is running.
//   node --test tools/browser-comparison/test_await_instance.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const MODULE = process.env.AWAIT_UNDER_TEST || path.join(HERE, 'await_instance.mjs');   // a mutated copy when mutation-testing
const { problems, awaitInstance, summarize } = await import(MODULE);
const PYTHON = process.env.SATURN_CMP_PYTHON || 'python3';

const good = (over = {}) => ({
  ownerAlive: true, mainPidMatches: null, ownerPid: 200, ownerStartTicks: 2000, bootId: 'boot', exeSha256: 'e'.repeat(64), buildGitSha: 'f'.repeat(40),
  piSourceAgeMs: 400, nodelayEnabled: 1, nodelayFailedTotal: 0, rxc1Status: 'valid', fpgaBuildId: '53460004', firmwareMajor: 1, firmwareMinor: 31, ...over,
});
const want = { previous: { pid: 100, startTicks: 1000 }, nodelay: 1, rxc1: 'valid', build: 'f'.repeat(40), exeSha256: 'e'.repeat(64), fpgaBuildId: '53460004' };

test('a reading that is the wanted instance has no problems', () => {
  assert.deepEqual(problems(good(), want), []);
  assert.deepEqual(problems(good(), {}), [], 'with nothing asked for, only liveness and freshness matter');
});

test('each way a reading can fall short is named', () => {
  const cases = [
    [{ error: 'no response within 2000 ms' }, /collector read failed: no response/],
    [good({ ownerAlive: false }), /not alive/],
    [good({ mainPidMatches: false }), /MainPID/],
    [good({ ownerPid: 100, ownerStartTicks: 1000 }), /still the previous process/],
    [good({ piSourceAgeMs: 9000 }), /not fresh/],
    [good({ piSourceAgeMs: -500 }), /not fresh/],
    [good({ piSourceAgeMs: undefined }), /not fresh/],
    [good({ nodelayEnabled: 0 }), /TCP_NODELAY reports 0, wanted 1/],
    [good({ nodelayEnabled: undefined }), /TCP_NODELAY reports undefined/],
    [good({ nodelayFailedTotal: 2 }), /2 socket\(s\) failed/],
    [good({ rxc1Status: 'unarmed' }), /RXC1 status "unarmed", wanted "valid"/],
    [good({ rxc1Status: undefined }), /RXC1 status undefined/],
    [good({ buildGitSha: 'a'.repeat(40) }), /build a+, wanted f+/],
    [good({ exeSha256: 'd'.repeat(64) }), /not the planned one/],
    [good({ fpgaBuildId: '53460003' }), /FPGA build 53460003, wanted 53460004/],
  ];
  for (const [reading, pattern] of cases) {
    const why = problems(reading, want);
    assert.ok(why.some((w) => pattern.test(w)), `${JSON.stringify(reading)} -> ${JSON.stringify(why)}`);
  }
});

test('the previous process is recognised by pid AND start time: a reused pid with a new start time is a new process', () => {
  assert.deepEqual(problems(good({ ownerPid: 100, ownerStartTicks: 1500 }), want), []);
  assert.deepEqual(problems(good({ ownerPid: 150, ownerStartTicks: 1000 }), want), []);
});

// a scripted collector on a virtual clock
function scripted(readings, step = 1000) {
  let t = 0; let i = 0;
  return { now: () => t, sleep: async (ms) => { t += ms; }, read: async () => readings[Math.min(i++, readings.length - 1)], reads: () => i, pollMs: step };
}

test('it waits through errors, the old process, and a half-configured start, then succeeds only after the settle period', async () => {
  const s = scripted([{ error: 'no response' }, good({ ownerPid: 100, ownerStartTicks: 1000 }), good({ nodelayEnabled: 0 }), good({ rxc1Status: 'unavailable' }), good()]);
  const seen = [];
  const r = await awaitInstance({ read: s.read, want, timeoutMs: 120000, settleMs: 5000, pollMs: s.pollMs, now: s.now, sleep: s.sleep, onChange: (t) => seen.push(t) });
  assert.equal(r.ok, true);
  assert.equal(r.instance.pid, 200); assert.equal(r.instance.startTicks, 2000); assert.equal(r.instance.nodelayEnabled, 1); assert.equal(r.instance.rxc1Status, 'valid');
  assert.ok(s.now() >= 4000 + 5000, `settled for at least 5 s after the first good reading (virtual time ${s.now()})`);
  assert.ok(seen.some((t) => /collector read failed/.test(t)) && seen.some((t) => /still the previous process/.test(t)) && seen.some((t) => /settling/.test(t)));
});

test('a process that changes while settling restarts the settle period (a crash-looping Bridge never settles)', async () => {
  const flip = [good({ ownerPid: 200, ownerStartTicks: 2000 }), good({ ownerPid: 200, ownerStartTicks: 2000 }), good({ ownerPid: 300, ownerStartTicks: 3000 })];
  const s = scripted([...flip, ...Array(10).fill(good({ ownerPid: 300, ownerStartTicks: 3000 }))]);
  const r = await awaitInstance({ read: s.read, want, timeoutMs: 120000, settleMs: 5000, pollMs: s.pollMs, now: s.now, sleep: s.sleep });
  assert.equal(r.ok, true);
  assert.equal(r.instance.pid, 300, 'the settled instance is the last one, not the first that matched');
  assert.ok(s.now() >= 2000 + 5000, 'the settle period restarted when the process changed');
  const loop = scripted(Array.from({ length: 200 }, (_, k) => good({ ownerPid: 200 + k, ownerStartTicks: 2000 + k })));
  const never = await awaitInstance({ read: loop.read, want, timeoutMs: 30000, settleMs: 5000, pollMs: loop.pollMs, now: loop.now, sleep: loop.sleep });
  assert.equal(never.ok, false, 'a Bridge that restarts every second never settles');
});

test('a pid that stays while its start time changes (a fast restart with a reused pid), or a start time that stays while the pid changes, restarts the settle period', async () => {
  for (const [label, second] of [['start time changed', good({ ownerPid: 200, ownerStartTicks: 2500 })], ['pid changed', good({ ownerPid: 250, ownerStartTicks: 2000 })]]) {
    const s = scripted([good(), good(), second, ...Array(10).fill(second)]);
    const r = await awaitInstance({ read: s.read, want, timeoutMs: 120000, settleMs: 5000, pollMs: s.pollMs, now: s.now, sleep: s.sleep });
    assert.equal(r.ok, true, label);
    assert.equal(r.instance.startTicks, second.ownerStartTicks, label);
    assert.ok(s.now() >= 2000 + 5000, `${label}: the settle period restarted at the change (virtual time ${s.now()})`);
  }
});

test('a reading that goes bad during settling restarts it', async () => {
  const s = scripted([good(), good(), good({ rxc1Status: 'unavailable' }), good(), good(), good(), good(), good(), good(), good()]);
  const r = await awaitInstance({ read: s.read, want, timeoutMs: 120000, settleMs: 5000, pollMs: s.pollMs, now: s.now, sleep: s.sleep });
  assert.equal(r.ok, true);
  assert.ok(s.now() >= 3000 + 5000, `settled only after the bad reading: virtual time ${s.now()}`);
});

test('it gives up at the timeout and says what it last saw', async () => {
  const s = scripted([good({ nodelayEnabled: 0 })]);
  const r = await awaitInstance({ read: s.read, want, timeoutMs: 10000, settleMs: 5000, pollMs: s.pollMs, now: s.now, sleep: s.sleep });
  assert.equal(r.ok, false);
  assert.match(r.reasons.join(';'), /TCP_NODELAY reports 0, wanted 1/);
  assert.ok(s.now() >= 10000 && s.now() < 12000);
});

test('summarize carries the identity of the settled instance', () => {
  assert.deepEqual(summarize(good(), 5000, 77), { pid: 200, startTicks: 2000, bootId: 'boot', exeSha256: 'e'.repeat(64), buildGitSha: 'f'.repeat(40), nodelayEnabled: 1, rxc1Status: 'valid',
    fpgaBuildId: '53460004', firmwareMajor: 1, firmwareMinor: 31, observedAtMs: 77, settleMs: 5000 });
});

// ---- end to end: the real reader, the real CLI, and two real processes standing in for the old and the new Bridge
function writeDoc(file, pid, nodelay, rxc1) {
  const metrics = { pid, build_git_sha: 'f'.repeat(40), iq: 1, audio: 1, connections: 1, iq_tci_frames_s: 0, rx_audio_frames_s: 46.9, display_spectrum_rows_written: 0,
    display_spectrum_clients: 0, audio_dropped_s: 0, tcp_outq_hwm_bytes: 0, out_hwm_bytes: 0, outbound_drops: 0, date_code_hex: '53460004', firmware_major: 1, firmware_minor: 31,
    tci_nodelay_enabled: nodelay, tci_nodelay_confirmed_total: 1, tci_nodelay_failed_total: 0, rx_counter_v31: { schema: 'rxc1-v1', status: rxc1, host_acquisition_failures: 0, ddc: [] } };
  const tmp = `${file}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify({ schema_version: 1, updated_at_ms: Date.now(), source: 'saturn-bridge', backend: 'xdma', metrics }));
  fs.renameSync(tmp, file);
}
const cli = (args, env) => spawn(process.execPath, [MODULE, ...args], { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
const done = (child) => new Promise((resolve) => { let out = ''; let err = ''; child.stdout.on('data', (d) => { out += d; }); child.stderr.on('data', (d) => { err += d; }); child.on('exit', (code) => resolve({ code, out, err })); });

test('end to end: the CLI waits for a different, settled, correctly configured process and records it', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'await-test-'));
  const old = spawn('sleep', ['60']); const next = spawn('sleep', ['60']);
  const keep = setInterval(() => {}, 1000);
  try {
    await new Promise((r) => setTimeout(r, 200));
    const file = path.join(dir, 'perf.json');
    writeDoc(file, old.pid, 1, 'valid');
    const refresher = setInterval(() => writeDoc(file, current.pid, current.nodelay, current.rxc1), 200);
    const current = { pid: old.pid, nodelay: 1, rxc1: 'valid' };
    // first the previous instance
    const prevOut = path.join(dir, 'previous.json');
    const first = await done(cli(['--sidecar', path.join(dir, 'a.jsonl'), '--out', prevOut, '--work-dir', dir, '--nodelay', '1', '--rxc1', 'valid', '--settle-s', '1', '--poll-ms', '100', '--timeout-s', '20'], { SATURN_CMP_COLLECTOR: 'local', SATURN_CMP_PYTHON: PYTHON }));
    assert.equal(first.code, 0, first.err + first.out);
    const prev = JSON.parse(fs.readFileSync(prevOut, 'utf8'));
    assert.equal(prev.instance.pid, old.pid);
    // then the operator "restarts": the new process comes up with the wrong setting, then with RXC1 unavailable, then right
    const waiting = done(cli(['--sidecar', path.join(dir, 'b.jsonl'), '--out', path.join(dir, 'next.json'), '--work-dir', dir, '--previous', prevOut, '--nodelay', '0', '--rxc1', 'valid',
      '--settle-s', '1', '--poll-ms', '100', '--timeout-s', '30'], { SATURN_CMP_COLLECTOR: 'local', SATURN_CMP_PYTHON: PYTHON }));
    await new Promise((r) => setTimeout(r, 800)); Object.assign(current, { pid: next.pid, nodelay: 1, rxc1: 'valid' });         // wrong NODELAY
    await new Promise((r) => setTimeout(r, 800)); Object.assign(current, { nodelay: 0, rxc1: 'unavailable' });                   // RXC1 not up yet
    await new Promise((r) => setTimeout(r, 800)); Object.assign(current, { rxc1: 'valid' });                                      // now right
    const second = await waiting;
    clearInterval(refresher);
    assert.equal(second.code, 0, second.err + second.out);
    assert.match(second.out, /waiting: .*still the previous process/s);
    assert.match(second.out, /waiting: .*TCP_NODELAY reports 1, wanted 0/s);
    assert.match(second.out, /waiting: .*RXC1 status "unavailable"/s);
    const next_ = JSON.parse(fs.readFileSync(path.join(dir, 'next.json'), 'utf8'));
    assert.equal(next_.ok, true); assert.equal(next_.instance.pid, next.pid); assert.equal(next_.instance.nodelayEnabled, 0); assert.equal(next_.instance.rxc1Status, 'valid');
    assert.notEqual(next_.instance.pid, prev.instance.pid);           // (the two stand-ins may share a start tick; pid and start time are compared together)
    assert.ok(Number.isInteger(next_.instance.startTicks));
    assert.match(next_.instance.exeSha256, /^[0-9a-f]{64}$/);
  } finally { clearInterval(keep); old.kill(); next.kill(); fs.rmSync(dir, { recursive: true, force: true }); }
});

test('end to end: the previous instance, even if it otherwise matches, is never accepted as the new one', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'await-test-'));
  const proc = spawn('sleep', ['60']);
  try {
    await new Promise((r) => setTimeout(r, 200));
    const file = path.join(dir, 'perf.json');
    const refresher = setInterval(() => writeDoc(file, proc.pid, 1, 'valid'), 200);
    writeDoc(file, proc.pid, 1, 'valid');
    const env = { SATURN_CMP_COLLECTOR: 'local', SATURN_CMP_PYTHON: PYTHON };
    const first = await done(cli(['--sidecar', path.join(dir, 'p1.jsonl'), '--out', path.join(dir, 'previous.json'), '--work-dir', dir, '--nodelay', '1', '--settle-s', '1', '--poll-ms', '100', '--timeout-s', '20'], env));
    assert.equal(first.code, 0, first.err);
    const again = await done(cli(['--sidecar', path.join(dir, 'p2.jsonl'), '--out', path.join(dir, 'again.json'), '--work-dir', dir, '--previous', path.join(dir, 'previous.json'), '--nodelay', '1', '--settle-s', '1', '--poll-ms', '100', '--timeout-s', '2'], env));
    clearInterval(refresher);
    assert.equal(again.code, 1, 'the same process is not a restart');
    assert.match(JSON.parse(fs.readFileSync(path.join(dir, 'again.json'), 'utf8')).reasons.join(';'), /still the previous process/);
  } finally { proc.kill(); fs.rmSync(dir, { recursive: true, force: true }); }
});

test('end to end: a Bridge that never reaches the wanted state times out with exit 1 and the reasons recorded', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'await-test-'));
  const proc = spawn('sleep', ['60']);
  try {
    await new Promise((r) => setTimeout(r, 200));
    const file = path.join(dir, 'perf.json');
    const refresher = setInterval(() => writeDoc(file, proc.pid, 1, 'unarmed'), 200);
    writeDoc(file, proc.pid, 1, 'unarmed');
    const r = await done(cli(['--sidecar', path.join(dir, 'c.jsonl'), '--out', path.join(dir, 'x.json'), '--work-dir', dir, '--rxc1', 'valid', '--settle-s', '1', '--poll-ms', '100', '--timeout-s', '2'], { SATURN_CMP_COLLECTOR: 'local', SATURN_CMP_PYTHON: PYTHON }));
    clearInterval(refresher);
    assert.equal(r.code, 1);
    assert.match(r.err, /timed out/);
    const out = JSON.parse(fs.readFileSync(path.join(dir, 'x.json'), 'utf8'));
    assert.equal(out.ok, false); assert.match(out.reasons.join(';'), /RXC1 status "unarmed", wanted "valid"/);
  } finally { proc.kill(); fs.rmSync(dir, { recursive: true, force: true }); }
});

test('usage errors exit 2 and write nothing', () => {
  const run = (args, env = {}) => spawnSync(process.execPath, [MODULE, ...args], { env: { ...process.env, ...env }, encoding: 'utf8' });
  assert.equal(run([]).status, 2);
  assert.equal(run(['--sidecar', '/tmp/x.jsonl', '--out', '/tmp/x.json', '--nodelay', '2'], { SATURN_CMP_COLLECTOR: 'local' }).status, 2);
  assert.equal(run(['--bogus']).status, 2);
  const none = run(['--sidecar', path.join(os.tmpdir(), `nocol-${process.pid}.jsonl`), '--out', path.join(os.tmpdir(), `nocol-${process.pid}.json`)], { SATURN_CMP_COLLECTOR: '' });
  assert.equal(none.status, 2); assert.match(none.stderr, /SATURN_CMP_COLLECTOR/);
});
