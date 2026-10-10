// Wait until the owner's own telemetry shows the Bridge instance the operator was asked to start, then print what was observed.
//
//   node await_instance.mjs --sidecar PATH --out instance.json [--work-dir DIR] [--previous instance.json]
//        [--nodelay 0|1] [--rxc1 STATE] [--build SHA] [--exe-sha SHA256] [--fpga-build ID]
//        [--timeout-s 900] [--settle-s 20] [--poll-ms 1000]
//
// The collector is chosen by the environment exactly as in window.mjs (SATURN_CMP_COLLECTOR=local|ssh). Nothing here starts, stops or
// signals anything: it only reads, once per poll, through the same bounded collector. A window may start only after this exits 0.
//
// "The new instance" is not what anyone says it is; it is what the owner reports: a different process than the previous one, alive, with
// fresh cached telemetry, the TCP_NODELAY setting and RXC1 state that were asked for, the planned build and FPGA image, and all of that
// unchanged for the settle period (a Bridge that is still starting, or crash-looping, never settles).
import fs from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { collectorFromEnv } from './collector.mjs';

export const PI_AGE_MIN_MS = -50;
export const PI_AGE_MAX_MS = 3000;

/** Why this reading is not yet the wanted instance (empty when it is). */
export function problems(reading, want) {
  if (!reading || reading.error) return [`collector read failed: ${reading && reading.error}`];
  const why = [];
  if (reading.ownerAlive !== true) why.push('the owner process is not alive');
  if (reading.mainPidMatches === false) why.push("systemd's MainPID differs from the document's pid");
  const prev = want.previous;
  if (prev && reading.ownerPid === prev.pid && reading.ownerStartTicks === prev.startTicks) why.push('still the previous process');
  if (!(reading.piSourceAgeMs >= PI_AGE_MIN_MS && reading.piSourceAgeMs <= PI_AGE_MAX_MS)) why.push(`cached telemetry is not fresh (age ${reading.piSourceAgeMs} ms)`);
  if (want.nodelay !== undefined) {
    if (reading.nodelayEnabled !== want.nodelay) why.push(`TCP_NODELAY reports ${reading.nodelayEnabled}, wanted ${want.nodelay}`);
    if (reading.nodelayFailedTotal > 0) why.push(`${reading.nodelayFailedTotal} socket(s) failed to take TCP_NODELAY`);
  }
  if (want.rxc1 !== undefined && reading.rxc1Status !== want.rxc1) why.push(`RXC1 status ${JSON.stringify(reading.rxc1Status)}, wanted ${JSON.stringify(want.rxc1)}`);
  if (want.build !== undefined && reading.buildGitSha !== want.build) why.push(`build ${reading.buildGitSha}, wanted ${want.build}`);
  if (want.exeSha256 !== undefined && reading.exeSha256 !== want.exeSha256) why.push('the running executable is not the planned one');
  if (want.fpgaBuildId !== undefined && reading.fpgaBuildId !== want.fpgaBuildId) why.push(`FPGA build ${reading.fpgaBuildId}, wanted ${want.fpgaBuildId}`);
  return why;
}

export function summarize(reading, settleMs, observedAtMs) {
  return {
    pid: reading.ownerPid, startTicks: reading.ownerStartTicks, bootId: reading.bootId, exeSha256: reading.exeSha256, buildGitSha: reading.buildGitSha,
    nodelayEnabled: reading.nodelayEnabled, rxc1Status: reading.rxc1Status, fpgaBuildId: reading.fpgaBuildId,
    firmwareMajor: reading.firmwareMajor, firmwareMinor: reading.firmwareMinor, observedAtMs, settleMs,
  };
}

/**
 * Poll `read()` until a reading satisfies `want` continuously for `settleMs` with the same process, or until `timeoutMs`.
 * Returns {ok, instance|reasons, last}. Never throws on a failed read; a failed read just means "not yet".
 */
export async function awaitInstance({ read, want, timeoutMs, settleMs, pollMs, now = Date.now, sleep = (ms) => new Promise((r) => setTimeout(r, ms)), onChange = () => {} }) {
  const deadline = now() + timeoutMs;
  let stableKey = null; let stableSince = null; let last = null; let lastReasons = ['no reading yet']; let shown = '';
  for (;;) {
    last = await read();
    const reasons = problems(last, want);
    if (reasons.length === 0) {
      const key = `${last.ownerPid}:${last.ownerStartTicks}`;
      if (key !== stableKey) { stableKey = key; stableSince = now(); }
      if (now() - stableSince >= settleMs) return { ok: true, instance: summarize(last, settleMs, now()), last };
      lastReasons = [`matches, settling (${Math.round((now() - stableSince) / 1000)} s of ${Math.round(settleMs / 1000)} s)`];
    } else {
      stableKey = null; stableSince = null; lastReasons = reasons;
    }
    const text = lastReasons.join('; ');
    if (text !== shown) { shown = text; onChange(text); }
    if (now() >= deadline) return { ok: false, reasons: lastReasons, last };
    await sleep(pollMs);
  }
}

function parseArgs(argv) {
  const opts = {};
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith('--') || i + 1 >= argv.length) throw new Error(`bad argument ${JSON.stringify(argv[i])}`);
    opts[argv[i].slice(2)] = argv[i + 1];
  }
  return opts;
}

async function main(argv) {
  let o;
  try { o = parseArgs(argv); } catch (e) { console.error(`await_instance: ${e.message}`); return 2; }
  if (!o.sidecar || !o.out) { console.error('await_instance: --sidecar and --out are required'); return 2; }
  const want = {};
  if (o.nodelay !== undefined) { if (!['0', '1'].includes(o.nodelay)) { console.error('await_instance: --nodelay must be 0 or 1'); return 2; } want.nodelay = Number(o.nodelay); }
  if (o.rxc1 !== undefined) want.rxc1 = o.rxc1;
  if (o.build !== undefined) want.build = o.build;
  if (o['exe-sha'] !== undefined) want.exeSha256 = o['exe-sha'];
  if (o['fpga-build'] !== undefined) want.fpgaBuildId = o['fpga-build'];
  if (o.previous !== undefined) {
    const prev = JSON.parse(fs.readFileSync(o.previous, 'utf8'));
    const inst = prev.instance || prev;
    want.previous = { pid: inst.pid, startTicks: inst.startTicks };
  }
  const collector = collectorFromEnv(process.env, { workDir: o['work-dir'], sidecar: o.sidecar, readerPath: fileURLToPath(new URL('./owner_reader.py', import.meta.url)) });
  if (!collector) { console.error('await_instance: set SATURN_CMP_COLLECTOR=local|ssh'); return 2; }
  const started = Date.now();
  try {
    const result = await awaitInstance({
      read: () => collector.read(), want,
      timeoutMs: Number(o['timeout-s'] ?? 900) * 1000, settleMs: Number(o['settle-s'] ?? 20) * 1000, pollMs: Number(o['poll-ms'] ?? 1000),
      onChange: (text) => console.log(`[${Math.round((Date.now() - started) / 1000)} s] waiting: ${text}`),
    });
    fs.writeFileSync(o.out, JSON.stringify(result.ok ? { ok: true, want, instance: result.instance } : { ok: false, want, reasons: result.reasons, last: result.last }, null, 1));
    if (result.ok) { console.log(`observed the wanted instance: pid ${result.instance.pid}, started at tick ${result.instance.startTicks}`); return 0; }
    console.error(`await_instance: timed out; last state: ${result.reasons.join('; ')}`);
    return 1;
  } finally { await collector.close(); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = await main(process.argv.slice(2));
}
