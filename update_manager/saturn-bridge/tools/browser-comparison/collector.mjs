// Collector of the acquisition owner's cached telemetry, through the read-only reader (owner_reader.py).
//
// One long-lived reader process per window answers "read" with one line of JSON (see owner_reader.py). This client
//   - bounds every request (a hung reader is killed and respawned, never waited on),
//   - appends every answer, and every failure, to a sidecar file (OUT.collector.jsonl) the moment it arrives, so the
//     raw evidence survives a timeout, a crash or a closed browser,
//   - maps an answer into the `bridge` record of a sample with the SAME function for every arm and every source,
//   - turns any failure into an error record. It never substitutes zeros, never refreshes a timestamp and never
//     retries within a sample.
// The reader only reads: perf.json and /proc. Nothing here writes to, or starts anything on, the machine it reads.
import { spawn } from 'node:child_process';
import { performance } from 'node:perf_hooks';
import fs from 'node:fs';

export const READER_SCHEMA = 'saturn-owner-reader-v1';
export const SIDECAR_SCHEMA = 'saturn-collector-v1';
export const TIMEOUT_MS = 2000;          // one request, once the reader is running
export const SPAWN_TIMEOUT_MS = 10000;   // a request that also had to start the reader (an SSH session needs longer)
export const MAX_LINE_BYTES = 4 * 1024 * 1024;
const STDERR_TAIL = 1000;

// perf.json metric names -> sample.bridge field names. The one place this mapping lives (window.mjs uses it for the
// direct file path too), so the collector and the file path cannot drift apart. The checker verifies the mapping
// against the raw document retained in the sidecar.
export const METRIC_FIELDS = {
  iq: 'iq', audio: 'audio', connections: 'connections', iq_tci_frames_s: 'iq_tci_frames_s', rx_audio_frames_s: 'rx_audio_frames_s',
  rows_written: 'display_spectrum_rows_written', spectrum_clients: 'display_spectrum_clients', audio_dropped_s: 'audio_dropped_s',
  tcp_outq_hwm_bytes: 'tcp_outq_hwm_bytes', out_hwm_bytes: 'out_hwm_bytes', outbound_drops: 'outbound_drops',
};
// The Bridge's own TCP_NODELAY evidence: absent on a Bridge that predates it, and then absent here (never zero).
export const NODELAY_FIELDS = {
  nodelayEnabled: 'tci_nodelay_enabled', nodelayConfirmedTotal: 'tci_nodelay_confirmed_total', nodelayFailedTotal: 'tci_nodelay_failed_total',
};

// The FPGA image the owner reports, carried with every reading (it decides what RXC1 can mean).
export const IDENTITY_FIELDS = { fpgaBuildId: 'date_code_hex', firmwareMajor: 'firmware_major', firmwareMinor: 'firmware_minor' };

function drop(obj) {
  for (const k of Object.keys(obj)) if (obj[k] === undefined) delete obj[k];
  return obj;
}

/** The bridge fields that come from the cached document alone (the original direct-file path uses exactly these). */
export function mapMetrics(document) {
  const m = document.metrics;
  const out = { updatedAtMs: document.updated_at_ms };
  for (const [field, metric] of Object.entries(METRIC_FIELDS)) out[field] = m[metric];
  return drop(out);
}

export function findNonFinite(value, path = '$', found = []) {
  if (found.length >= 5) return found;
  if (typeof value === 'number' && !Number.isFinite(value)) found.push(path);
  else if (Array.isArray(value)) value.forEach((v, i) => findNonFinite(v, `${path}[${i}]`, found));
  else if (value && typeof value === 'object') for (const [k, v] of Object.entries(value)) findNonFinite(v, `${path}.${k}`, found);
  return found;
}

/** Why an answer cannot be used, or null. */
export function answerProblem(answer) {
  if (!answer || typeof answer !== 'object' || Array.isArray(answer)) return 'the reader answer is not an object';
  if (answer.schema !== READER_SCHEMA) return `unexpected reader schema ${JSON.stringify(answer.schema)}`;
  if (answer.ok !== true) return `reader: ${answer.error || 'the cached document is unavailable'}`;
  const bad = findNonFinite(answer);
  if (bad.length) return `non-finite numbers in the reader answer at ${bad}`;
  const d = answer.document;
  if (!d || typeof d !== 'object' || !d.metrics || typeof d.metrics !== 'object') return 'the reader answer has no document with metrics';
  for (const k of ['piReadAtMs', 'piSourceUpdatedAtMs', 'piSourceAgeMs']) {
    if (typeof answer[k] !== 'number') return `the reader answer has no numeric ${k}`;
  }
  if (!answer.owner || typeof answer.owner !== 'object') return 'the reader answer has no owner identity';
  return null;
}

/** A sample.bridge record from one good answer plus the collector's own receipt. */
export function mapReading(answer, receipt) {
  const o = answer.owner;
  const m = answer.document.metrics;
  const out = mapMetrics(answer.document);
  Object.assign(out, {
    piSourceAgeMs: answer.piSourceAgeMs, piSourceUpdatedAtMs: answer.piSourceUpdatedAtMs, piReadAtMs: answer.piReadAtMs,
    documentSha256: answer.documentSha256,
    ownerPid: o.pid, ownerStartTicks: o.startTicks, ownerAlive: o.alive, bootId: o.bootId, exeSha256: o.exeSha256,
    exeError: o.exeError, mainPidMatches: o.mainPidMatches, buildGitSha: m.build_git_sha,
    collectorSeq: receipt.seq, collectorSpawn: receipt.spawn, collectorReceivedAtMs: receipt.receivedAtMs, requestLatencyMs: receipt.latencyMs,
  });
  for (const [field, metric] of Object.entries(NODELAY_FIELDS)) out[field] = m[metric];
  for (const [field, metric] of Object.entries(IDENTITY_FIELDS)) out[field] = m[metric];
  // RXC1 polling state as the owner itself publishes it (metrics.rx_counter_v31): absent on a Bridge without it, never defaulted.
  const rx = m.rx_counter_v31;
  if (rx && typeof rx === 'object') {
    out.rxc1Status = rx.status;
    out.rxc1HostAcquisitionFailures = rx.host_acquisition_failures;
  }
  return drop(out);
}

/** A failure is an error record. It carries the collector's receipt and nothing that looks like a measurement. */
export function mapFailure(message, receipt) {
  return { error: message, collectorSeq: receipt.seq, collectorSpawn: receipt.spawn, collectorReceivedAtMs: receipt.receivedAtMs, requestLatencyMs: receipt.latencyMs };
}

export class Collector {
  #child = null; #buf = ''; #waiter = null; #stderr = ''; #tail = Promise.resolve();
  #seq = 0; #spawns = 0; #failures = 0; #reads = 0;

  constructor({ command, args = [], sidecar, timeoutMs = TIMEOUT_MS, spawnTimeoutMs = SPAWN_TIMEOUT_MS, maxLineBytes = MAX_LINE_BYTES, clock = Date.now, env = process.env, summary = {} }) {
    if (!command) throw new Error('collector: no reader command');
    if (!sidecar) throw new Error('collector: a sidecar path is required (raw answers must be retained)');
    Object.assign(this, { command, args, sidecar, timeoutMs, spawnTimeoutMs, maxLineBytes, clock, env });
    // 'wx': a sidecar from an earlier run must never be extended (its sequence numbers would collide with this run's).
    fs.writeFileSync(sidecar, JSON.stringify({ kind: 'start', schema: SIDECAR_SCHEMA, startedAtMs: clock(), command: [command, ...args].map((a) => String(a).slice(0, 200)), timeoutMs, spawnTimeoutMs, ...summary }) + '\n', { flag: 'wx' });
  }

  append(entry) {
    fs.appendFileSync(this.sidecar, JSON.stringify(entry) + '\n');
  }

  summary() {
    return { schema: SIDECAR_SCHEMA, reads: this.#reads, failures: this.#failures, spawns: this.#spawns, timeoutMs: this.timeoutMs };
  }

  /** One bounded request. Never throws: the result is a mapped reading or an error record. Requests never overlap. */
  read() {
    const run = this.#tail.then(() => this.#once());
    this.#tail = run.then(() => undefined);
    return run;
  }

  #spawn() {
    this.#spawns += 1;
    const spawnNo = this.#spawns;
    const child = spawn(this.command, this.args, { stdio: ['pipe', 'pipe', 'pipe'], env: this.env });
    this.#child = child; this.#buf = ''; this.#stderr = '';
    const mine = () => this.#child === child;
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      if (!mine()) return;
      this.#buf += chunk;
      const nl = this.#buf.indexOf('\n');
      if (nl < 0) {
        if (this.#buf.length > this.maxLineBytes) this.#fail(new Error(`reader line exceeds ${this.maxLineBytes} bytes`), child);
        return;
      }
      const line = this.#buf.slice(0, nl);
      const rest = this.#buf.slice(nl + 1);
      this.#buf = rest;
      if (!this.#waiter || rest.length) {
        // Output nobody asked for: the request/response pairing can no longer be trusted. Discard the reader.
        this.#fail(new Error('the reader produced output that was not a response to a request'), child);
        return;
      }
      const waiter = this.#waiter; this.#waiter = null;
      waiter.resolve({ line, receivedAtMs: this.clock(), at: performance.now() });
    });
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', (chunk) => { if (mine()) this.#stderr = (this.#stderr + chunk).slice(-STDERR_TAIL); });
    child.stdin.on('error', (error) => { if (mine()) this.#fail(new Error(`cannot write to the reader: ${error.message}`), child); });
    child.on('error', (error) => { if (mine()) this.#fail(new Error(`cannot run the reader: ${error.message}`), child); });
    child.on('exit', (code, signal) => { if (mine()) this.#fail(new Error(`the reader exited (${signal ? `signal ${signal}` : `status ${code}`})`), child); });
    this.append({ kind: 'spawn', spawn: spawnNo, readerPid: child.pid ?? null, atMs: this.clock() });
    return child;
  }

  /** Reject the pending request (if any) and discard this reader; the next request starts a new one. */
  #fail(error, child) {
    if (this.#child === child) this.#child = null;
    try { child.kill('SIGKILL'); } catch (_) { /* already gone */ }
    const waiter = this.#waiter; this.#waiter = null;
    if (waiter) waiter.reject(error);
    else this.append({ kind: 'event', atMs: this.clock(), readerPid: child.pid ?? null, event: error.message });   // between requests: kept, and the next request starts a new reader
  }

  async #once() {
    this.#seq += 1;
    const seq = this.#seq;
    this.#reads += 1;
    const sentAtMs = this.clock();
    const t0 = performance.now();
    let spawned = false;
    let line = null; let receivedAtMs = null; let error = null; let tEnd = null;
    try {
      if (!this.#child) { this.#spawn(); spawned = true; }
      const child = this.#child;
      const timeoutMs = spawned ? this.spawnTimeoutMs : this.timeoutMs;
      const answer = new Promise((resolve, reject) => { this.#waiter = { resolve, reject }; });
      let timer;
      const timed = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`no response within ${timeoutMs} ms`)), timeoutMs); });
      child.stdin.write('read\n');
      try {
        const got = await Promise.race([answer, timed]);
        line = got.line; receivedAtMs = got.receivedAtMs; tEnd = got.at;
      } finally { clearTimeout(timer); }
    } catch (e) {
      error = String((e && e.message) || e);
      if (this.#child) this.#fail(new Error(error), this.#child);   // a hung or broken reader is killed, never reused
      this.#waiter = null;
    }
    if (receivedAtMs === null) { receivedAtMs = this.clock(); tEnd = performance.now(); }
    const receipt = { seq, spawn: this.#spawns, receivedAtMs, latencyMs: Math.max(0, Math.round(tEnd - t0)) };
    let answer = null;
    if (!error) {
      try { answer = JSON.parse(line); } catch (e) { error = `the reader answer is not valid JSON: ${e.message}`; }
      if (!error) error = answerProblem(answer);
    }
    const entry = { kind: 'read', seq, spawn: this.#spawns, spawned, sentAtMs, receivedAtMs, latencyMs: receipt.latencyMs, error, raw: line };
    if (error && this.#stderr) entry.stderrTail = this.#stderr;
    try {
      this.append(entry);
    } catch (e) {
      error = `cannot retain the raw answer in ${this.sidecar}: ${e.message}`;
    }
    if (error) {
      this.#failures += 1;
      return mapFailure(error, receipt);
    }
    return mapReading(answer, receipt);
  }

  async close() {
    await this.#tail;
    const child = this.#child;
    this.#child = null;
    if (!child) return;
    try { child.stdin.end('quit\n'); } catch (_) { /* gone */ }
    await new Promise((resolve) => {
      const timer = setTimeout(() => { try { child.kill('SIGKILL'); } catch (_) { /* gone */ } resolve(); }, 1000);
      child.once('exit', () => { clearTimeout(timer); resolve(); });
      if (child.exitCode !== null || child.signalCode !== null) { clearTimeout(timer); resolve(); }
    });
  }
}

/** A single-quoted word for a remote POSIX shell. */
export function quoteForRemoteShell(text) {
  return `'${String(text).replace(/'/g, `'\\''`)}'`;
}

/** ssh HOST python3 -c 'READER SOURCE' --path P: the reader travels as a program argument and installs nothing. */
export function sshReaderCommand({ host, readerSource, path, service, python = 'python3', ssh = 'ssh' }) {
  if (!/^[A-Za-z0-9][A-Za-z0-9._@-]*$/.test(host || '')) throw new Error(`collector: unacceptable ssh host ${JSON.stringify(host)}`);
  const remote = [python, '-c', quoteForRemoteShell(readerSource)];
  if (path) remote.push('--path', quoteForRemoteShell(path));
  if (service) remote.push('--service', quoteForRemoteShell(service));
  return { command: ssh, args: ['-T', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5', '-o', 'ServerAliveInterval=2', '-o', 'ServerAliveCountMax=2', host, remote.join(' ')] };
}

/**
 * The collector the environment asks for, or null for the original direct file read.
 *   SATURN_CMP_COLLECTOR=local   read WORKDIR/perf.json through the reader on this machine (the rehearsal)
 *   SATURN_CMP_COLLECTOR=ssh     read the owner's cached document on the Pi: needs SATURN_CMP_COLLECTOR_HOST,
 *                                optional SATURN_CMP_COLLECTOR_PATH and SATURN_CMP_COLLECTOR_SERVICE
 */
export function collectorFromEnv(env, { workDir, sidecar, readerPath }) {
  const mode = env.SATURN_CMP_COLLECTOR;
  if (!mode) return null;
  const python = env.SATURN_CMP_PYTHON || 'python3';
  if (mode === 'local') return new Collector({ command: python, args: [readerPath, '--path', `${workDir}/perf.json`], sidecar, summary: { source: 'local' } });
  if (mode === 'ssh') {
    const { command, args } = sshReaderCommand({
      host: env.SATURN_CMP_COLLECTOR_HOST, readerSource: fs.readFileSync(readerPath, 'utf8'),
      path: env.SATURN_CMP_COLLECTOR_PATH, service: env.SATURN_CMP_COLLECTOR_SERVICE, python,
    });
    return new Collector({ command, args, sidecar, summary: { source: 'ssh', host: env.SATURN_CMP_COLLECTOR_HOST, path: env.SATURN_CMP_COLLECTOR_PATH || null } });
  }
  throw new Error(`collector: SATURN_CMP_COLLECTOR must be "local" or "ssh", not ${JSON.stringify(mode)}`);
}
