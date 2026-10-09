// Tests for collector.mjs: the bounded client of the read-only owner reader.
//   node --test tools/browser-comparison/test_collector.mjs
// The real reader (owner_reader.py) is used for the good paths; small fake readers force the failure paths.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const MODULE = process.env.COLLECTOR_UNDER_TEST || path.join(HERE, 'collector.mjs');   // a mutated copy when mutation-testing
const { Collector, mapMetrics, collectorFromEnv, sshReaderCommand, quoteForRemoteShell } = await import(MODULE);
const READER = path.join(HERE, 'owner_reader.py');
const PYTHON = process.env.SATURN_CMP_PYTHON || 'python3';

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'collector-test-'));
const clean = (dir) => fs.rmSync(dir, { recursive: true, force: true });
const lines = (file) => fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };
const until = async (cond, ms = 2000) => { const end = Date.now() + ms; while (Date.now() < end) { if (cond()) return true; await new Promise((r) => setTimeout(r, 20)); } return cond(); };

function writeDoc(dir, { updated = Date.now(), nodelay = true, extra = {} } = {}) {
  const metrics = {
    pid: process.pid, build_git_sha: 'abc1234', iq: 1, audio: 1, connections: 1, iq_tci_frames_s: 46.9, rx_audio_frames_s: 46.9,
    display_spectrum_rows_written: 100, display_spectrum_clients: 0, audio_dropped_s: 0, tcp_outq_hwm_bytes: 10, out_hwm_bytes: 20, outbound_drops: 0, ...extra,
  };
  if (nodelay) Object.assign(metrics, { tci_nodelay_enabled: 1, tci_nodelay_confirmed_total: 2, tci_nodelay_failed_total: 0 });
  const file = path.join(dir, 'perf.json');
  fs.writeFileSync(file, JSON.stringify({ schema_version: 1, updated_at_ms: updated, source: 'saturn-bridge', backend: 'xdma', metrics }));
  return file;
}

const local = (dir, file, opts = {}) => new Collector({ command: PYTHON, args: [READER, '--path', file], sidecar: path.join(dir, 'w.collector.jsonl'), ...opts });

// A fake reader: node script whose behaviour is chosen by argv[2]. State file: argv[3] (first launch vs later launches).
const FAKE = `
import fs from 'node:fs';
import readline from 'node:readline';
const [,, mode, state, answerFile] = process.argv;
const first = !fs.existsSync(state);
fs.writeFileSync(state, 'x');
const good = answerFile ? fs.readFileSync(answerFile, 'utf8').trim() : '{}';
const rl = readline.createInterface({ input: process.stdin });
rl.on('line', (line) => {
  if (line === 'quit') process.exit(0);
  if (line !== 'read') return;
  if (mode === 'hang-first' && first) return;                                   // never answers
  if (mode === 'die-first' && first) { process.stderr.write('boom: simulated reader failure'); process.exit(3); }
  if (mode === 'slow') return setTimeout(() => console.log(good), 600);
  if (mode === 'null') return console.log('null');
  if (mode === 'string') return console.log('"hello"');
  if (mode.startsWith('strip:')) { const a = JSON.parse(good); delete a[mode.slice(6)]; return console.log(JSON.stringify(a)); }
  if (mode === 'garbage') return console.log('this is not json');
  if (mode === 'notok') return console.log(JSON.stringify({ ok: false, schema: 'saturn-owner-reader-v1', error: 'cannot read /x: gone' }));
  if (mode === 'wrongschema') return console.log(JSON.stringify({ ...JSON.parse(good), schema: 'something-else' }));
  if (mode === 'infinite') return console.log(good.replace('"piSourceAgeMs":', '"piSourceAgeMs":1e999,"x":'));
  if (mode === 'huge') return process.stdout.write('x'.repeat(100000));
  if (mode === 'unsolicited') { console.log(good); return setTimeout(() => console.log(good), 100); }
  console.log(good);
});
`;

function fakeReader(dir, mode) {
  const script = path.join(dir, 'fake_reader.mjs');
  fs.writeFileSync(script, FAKE);
  const answer = path.join(dir, 'answer.json');
  const doc = writeDoc(dir);
  fs.writeFileSync(answer, execFileSync(PYTHON, [READER, '--once', '--path', doc], { encoding: 'utf8' }));
  return { command: process.execPath, args: [script, mode, path.join(dir, `state-${mode}`), answer] };
}

test('a real reader answer becomes a bridge record with the Pi-side age, the owner identity and the Bridge-reported NODELAY evidence', async () => {
  const dir = tmp(); const c = local(dir, writeDoc(dir, { updated: Date.now() - 1500 }));
  try {
    const r = await c.read();
    assert.equal(r.error, undefined);
    assert.equal(r.iq, 1); assert.equal(r.audio, 1); assert.equal(r.rows_written, 100); assert.equal(r.outbound_drops, 0);
    assert.ok(r.piSourceAgeMs >= 1400 && r.piSourceAgeMs < 5000, r.piSourceAgeMs);
    assert.equal(r.piSourceAgeMs, r.piReadAtMs - r.piSourceUpdatedAtMs);
    assert.equal(r.piSourceUpdatedAtMs, r.updatedAtMs);
    assert.equal(r.ownerPid, process.pid); assert.equal(r.ownerAlive, true);
    assert.ok(Number.isInteger(r.ownerStartTicks)); assert.match(r.bootId, /^[0-9a-f-]{36}$/);
    assert.match(r.exeSha256, /^[0-9a-f]{64}$/); assert.match(r.documentSha256, /^[0-9a-f]{64}$/);
    assert.equal(r.buildGitSha, 'abc1234');
    assert.equal(r.nodelayEnabled, 1); assert.equal(r.nodelayConfirmedTotal, 2); assert.equal(r.nodelayFailedTotal, 0);
    assert.equal(r.collectorSeq, 1); assert.equal(r.collectorSpawn, 1);
    assert.ok(Number.isFinite(r.requestLatencyMs) && r.requestLatencyMs >= 0);
    assert.ok(Math.abs(r.collectorReceivedAtMs - Date.now()) < 5000);
  } finally { await c.close(); clean(dir); }
});

test('a Bridge that predates the NODELAY metrics yields no NODELAY fields: absent, never zero', async () => {
  const dir = tmp(); const c = local(dir, writeDoc(dir, { nodelay: false }));
  try {
    const r = await c.read();
    assert.equal(r.error, undefined);
    for (const k of ['nodelayEnabled', 'nodelayConfirmedTotal', 'nodelayFailedTotal']) assert.ok(!(k in r), `${k} must be absent`);
  } finally { await c.close(); clean(dir); }
});

test('the direct file path and the collector map the metrics with the same function', () => {
  const doc = { updated_at_ms: 5, metrics: { iq: 1, audio: 1, connections: 2, iq_tci_frames_s: 3, rx_audio_frames_s: 4, display_spectrum_rows_written: 5, display_spectrum_clients: 6, audio_dropped_s: 0.5, tcp_outq_hwm_bytes: 7, out_hwm_bytes: 8, outbound_drops: 9, pid: 1 } };
  assert.deepEqual(mapMetrics(doc), { updatedAtMs: 5, iq: 1, audio: 1, connections: 2, iq_tci_frames_s: 3, rx_audio_frames_s: 4, rows_written: 5, spectrum_clients: 6, audio_dropped_s: 0.5, tcp_outq_hwm_bytes: 7, out_hwm_bytes: 8, outbound_drops: 9 });
  assert.deepEqual(Object.keys(mapMetrics({ updated_at_ms: 5, metrics: { iq: 1 } })), ['updatedAtMs', 'iq'], 'a missing metric is omitted, not zero');
});

test('every answer is in the sidecar, with its raw line, before read() returns', async () => {
  const dir = tmp(); const file = writeDoc(dir); const c = local(dir, file);
  try {
    const sidecar = path.join(dir, 'w.collector.jsonl');
    const r1 = await c.read();
    let entries = lines(sidecar);
    assert.deepEqual(entries.map((e) => e.kind), ['start', 'spawn', 'read']);
    assert.equal(entries[0].schema, 'saturn-collector-v1');
    const raw = JSON.parse(entries[2].raw);
    assert.equal(raw.documentSha256, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), 'the raw answer carries the hash of the bytes that were read');
    assert.equal(raw.documentSha256, r1.documentSha256);
    assert.equal(entries[2].seq, r1.collectorSeq); assert.equal(entries[2].receivedAtMs, r1.collectorReceivedAtMs); assert.equal(entries[2].latencyMs, r1.requestLatencyMs);
    assert.equal(entries[2].error, null);
    await c.read(); await c.read();
    entries = lines(sidecar);
    assert.deepEqual(entries.filter((e) => e.kind === 'read').map((e) => e.seq), [1, 2, 3]);
    assert.equal(entries.filter((e) => e.kind === 'spawn').length, 1, 'one persistent reader serves the whole window');
  } finally { await c.close(); clean(dir); }
});

test('a hung reader is killed within the bound, the failure is recorded, and the next request starts a new reader', async () => {
  const dir = tmp(); const c = new Collector({ ...fakeReader(dir, 'hang-first'), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 300, spawnTimeoutMs: 300 });
  try {
    const t0 = Date.now();
    const failed = await c.read();
    assert.ok(Date.now() - t0 < 1500, `took ${Date.now() - t0} ms`);
    assert.match(failed.error, /no response within 300 ms/);
    assert.equal(failed.iq, undefined, 'a failure carries no measurement');
    assert.equal(failed.piSourceAgeMs, undefined);
    const hungPid = lines(path.join(dir, 's.jsonl')).find((e) => e.kind === 'spawn').readerPid;
    assert.ok(await until(() => !alive(hungPid)), 'the hung reader must be killed, not abandoned');
    const ok = await c.read();
    assert.equal(ok.error, undefined);
    assert.equal(ok.collectorSpawn, 2); assert.equal(ok.collectorSeq, 2);
    const reads = lines(path.join(dir, 's.jsonl')).filter((e) => e.kind === 'read');
    assert.match(reads[0].error, /no response/); assert.equal(reads[0].raw, null); assert.equal(reads[1].error, null);
    assert.deepEqual({ reads: c.summary().reads, failures: c.summary().failures, spawns: c.summary().spawns }, { reads: 2, failures: 1, spawns: 2 });
  } finally { await c.close(); clean(dir); }
});

test('a reader that dies is reported with its stderr and replaced', async () => {
  const dir = tmp(); const c = new Collector({ ...fakeReader(dir, 'die-first'), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 1000, spawnTimeoutMs: 1000 });
  try {
    const failed = await c.read();
    assert.match(failed.error, /exited \(status 3\)/);
    const entry = lines(path.join(dir, 's.jsonl')).find((e) => e.kind === 'read');
    assert.match(entry.stderrTail, /boom: simulated reader failure/);
    const ok = await c.read();
    assert.equal(ok.error, undefined); assert.equal(ok.collectorSpawn, 2);
  } finally { await c.close(); clean(dir); }
});

for (const [mode, pattern] of [['garbage', /not valid JSON/], ['notok', /reader: cannot read \/x: gone/], ['wrongschema', /unexpected reader schema/], ['infinite', /non-finite/]]) {
  test(`an unusable answer (${mode}) is a failure, never a reading`, async () => {
    const dir = tmp(); const c = new Collector({ ...fakeReader(dir, mode), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 1000, spawnTimeoutMs: 1000 });
    try {
      const r = await c.read();
      assert.match(r.error, pattern);
      assert.equal(r.iq, undefined); assert.equal(r.piSourceAgeMs, undefined); assert.equal(r.ownerPid, undefined);
      const entry = lines(path.join(dir, 's.jsonl')).find((e) => e.kind === 'read');
      assert.match(entry.error, pattern);
      assert.ok(entry.raw, 'the unusable raw line is retained as evidence');
    } finally { await c.close(); clean(dir); }
  });
}

for (const [mode, pattern] of [['null', /not an object/], ['string', /not an object/], ['strip:document', /no document with metrics/], ['strip:owner', /no owner identity/],
  ['strip:piSourceAgeMs', /no numeric piSourceAgeMs/], ['strip:piReadAtMs', /no numeric piReadAtMs/], ['strip:piSourceUpdatedAtMs', /no numeric piSourceUpdatedAtMs/]]) {
  test(`a malformed answer (${mode}) is a failure record, and read() does not throw`, async () => {
    const dir = tmp(); const c = new Collector({ ...fakeReader(dir, mode), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 1000, spawnTimeoutMs: 1000 });
    try {
      const r = await c.read();
      assert.match(r.error, pattern);
      assert.equal(r.iq, undefined); assert.equal(r.piSourceAgeMs, undefined);
    } finally { await c.close(); clean(dir); }
  });
}

test('a line that never ends is cut off at the limit', async () => {
  const dir = tmp(); const c = new Collector({ ...fakeReader(dir, 'huge'), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 2000, spawnTimeoutMs: 2000, maxLineBytes: 1000 });
  try {
    const t0 = Date.now();
    const r = await c.read();
    assert.match(r.error, /exceeds 1000 bytes/);
    assert.ok(Date.now() - t0 < 1500);
  } finally { await c.close(); clean(dir); }
});

test('output nobody asked for discards the reader, keeps an event, and the next request starts a new one', async () => {
  const dir = tmp(); const c = new Collector({ ...fakeReader(dir, 'unsolicited'), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 1000, spawnTimeoutMs: 1000 });
  try {
    const first = await c.read();
    assert.equal(first.error, undefined);
    assert.ok(await until(() => lines(path.join(dir, 's.jsonl')).some((e) => e.kind === 'event')), 'the stray output is recorded');
    const second = await c.read();
    assert.equal(second.error, undefined); assert.equal(second.collectorSpawn, 2, 'a desynchronised reader is never reused');
  } finally { await c.close(); clean(dir); }
});

test('a request that has to start the reader gets the longer bound; a running reader gets the short one; latency is measured', async () => {
  const dir = tmp(); const c = new Collector({ ...fakeReader(dir, 'slow'), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 250, spawnTimeoutMs: 3000 });
  try {
    const first = await c.read();                 // starts the reader and waits 600 ms for the answer: within the spawn bound
    assert.equal(first.error, undefined);
    assert.ok(first.requestLatencyMs >= 550 && first.requestLatencyMs < 2500, first.requestLatencyMs);
    const second = await c.read();                // the reader is running: 600 ms is beyond the 250 ms bound
    assert.match(second.error, /no response within 250 ms/);
    assert.ok(second.requestLatencyMs >= 240 && second.requestLatencyMs < 1000, second.requestLatencyMs);
    assert.equal(second.collectorSpawn, 1);
    const third = await c.read();                 // the timed-out reader was discarded, so this starts another and gets the long bound
    assert.equal(third.error, undefined); assert.equal(third.collectorSpawn, 2);
  } finally { await c.close(); clean(dir); }
});

test('a reader that cannot be started is a prompt failure, not a hang', async () => {
  const dir = tmp(); const c = new Collector({ command: path.join(dir, 'no-such-reader'), sidecar: path.join(dir, 's.jsonl'), timeoutMs: 5000, spawnTimeoutMs: 5000 });
  try {
    const t0 = Date.now();
    const r = await c.read();
    assert.match(r.error, /cannot run the reader|cannot write to the reader/);
    assert.ok(Date.now() - t0 < 2000);
    const r2 = await c.read();
    assert.ok(r2.error); assert.equal(r2.collectorSeq, 2);
  } finally { await c.close(); clean(dir); }
});

test('requests never overlap and are numbered in order', async () => {
  const dir = tmp(); const c = local(dir, writeDoc(dir));
  try {
    const rs = await Promise.all([c.read(), c.read(), c.read()]);
    assert.deepEqual(rs.map((r) => r.collectorSeq), [1, 2, 3]);
    assert.ok(rs.every((r) => r.error === undefined));
    assert.ok(rs[0].piReadAtMs <= rs[1].piReadAtMs && rs[1].piReadAtMs <= rs[2].piReadAtMs);
    assert.equal(c.summary().spawns, 1);
  } finally { await c.close(); clean(dir); }
});

test('an unreadable or stale document is reported as it is: a failure for the first, the true age for the second', async () => {
  const dir = tmp(); const file = writeDoc(dir, { updated: Date.now() - 86_400_000 }); const c = local(dir, file);
  try {
    const stale = await c.read();
    assert.equal(stale.error, undefined);
    assert.ok(stale.piSourceAgeMs > 86_000_000, 'a day-old document is not refreshed');
    fs.rmSync(file);
    const gone = await c.read();
    assert.match(gone.error, /cannot read/); assert.equal(gone.piSourceAgeMs, undefined);
  } finally { await c.close(); clean(dir); }
});

test('a sidecar from an earlier run is never extended', () => {
  const dir = tmp();
  try {
    const sidecar = path.join(dir, 's.jsonl');
    fs.writeFileSync(sidecar, '{"kind":"start"}\n');
    assert.throws(() => new Collector({ command: 'true', sidecar }), /EEXIST/);
    assert.throws(() => new Collector({ command: 'true' }), /sidecar/);
    assert.throws(() => new Collector({ sidecar: path.join(dir, 'x') }), /command/);
  } finally { clean(dir); }
});

test('close() stops the reader', async () => {
  const dir = tmp(); const c = local(dir, writeDoc(dir));
  await c.read();
  const pid = lines(path.join(dir, 'w.collector.jsonl')).find((e) => e.kind === 'spawn').readerPid;
  assert.ok(alive(pid));
  await c.close();
  assert.ok(await until(() => !alive(pid)), 'the reader process must be gone after close()');
  clean(dir);
});

test('the ssh form carries the reader as one quoted program argument that survives a POSIX shell, options and all', () => {
  const dir = tmp();
  try {
    const source = fs.readFileSync(READER, 'utf8');
    const file = writeDoc(dir);
    const spaced = path.join(dir, "it's here"); fs.mkdirSync(spaced); const file2 = writeDoc(spaced);
    const { command, args } = sshReaderCommand({ host: 'pi@192.0.2.10', readerSource: source, path: file2, service: 'saturn-bridge' });
    assert.equal(command, 'ssh');
    assert.deepEqual(args.slice(0, 2), ['-T', '-o']);
    const remote = args[args.length - 1];
    assert.equal(args[args.length - 2], 'pi@192.0.2.10');
    // What the remote shell would run, run here instead (a local sh stands in for the remote one).
    const out = execFileSync('sh', ['-c', `${remote} --once`], { encoding: 'utf8' });
    const answer = JSON.parse(out);
    assert.equal(answer.ok, true); assert.equal(answer.path, file2);
    assert.ok(file);
    assert.equal(quoteForRemoteShell("a'b"), "'a'\\''b'");
  } finally { clean(dir); }
});

test('an ssh host that could be read as an option, or is empty, is refused', () => {
  for (const host of ['-oProxyCommand=evil', '', undefined, 'pi host', 'pi;rm']) {
    assert.throws(() => sshReaderCommand({ host, readerSource: 'print(1)' }), /ssh host/, String(host));
  }
  assert.doesNotThrow(() => sshReaderCommand({ host: 'user@pi.local', readerSource: 'print(1)' }));
});

test('the environment selects the collector: nothing, local, ssh, or an error', async () => {
  const dir = tmp();
  try {
    const sidecar = path.join(dir, 's.jsonl');
    assert.equal(collectorFromEnv({}, { workDir: dir, sidecar, readerPath: READER }), null);
    assert.throws(() => collectorFromEnv({ SATURN_CMP_COLLECTOR: 'carrier-pigeon' }, { workDir: dir, sidecar, readerPath: READER }), /"local" or "ssh"/);
    assert.throws(() => collectorFromEnv({ SATURN_CMP_COLLECTOR: 'ssh' }, { workDir: dir, sidecar, readerPath: READER }), /ssh host/);
    writeDoc(dir);
    const c = collectorFromEnv({ SATURN_CMP_COLLECTOR: 'local', SATURN_CMP_PYTHON: PYTHON }, { workDir: dir, sidecar, readerPath: READER });
    const r = await c.read();
    assert.equal(r.error, undefined); assert.equal(r.ownerPid, process.pid);
    await c.close();
  } finally { clean(dir); }
});
