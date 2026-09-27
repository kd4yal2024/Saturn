#!/usr/bin/env node
// Live browser-side RX Opus probe.
//
// Runs the real template and bundle in headless Chrome against a *real* bridge,
// traces WebCodecs decode input versus output timestamps, and prints the
// browser's own codec snapshot. Use it to verify the RX audio path on hardware
// without the authenticated proxy: forward the bridge's loopback TCI port over
// SSH and point this probe at the tunnel.
//
//   ssh -N -L 127.0.0.1:15001:127.0.0.1:50001 pi@<radio> &
//   node scripts/live-rx-opus-probe.mjs --ws ws://127.0.0.1:15001/ --mode lan --seconds 20
//
// The probe listens only (RX audio and display); it never keys the radio and
// never changes radio state. It does add one RX audio listener while it runs.
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { appendFileSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const remoteWebRoot = resolve(scriptDir, '..');
const updateManagerRoot = resolve(remoteWebRoot, '..');

function option(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

// --template/--bundle let this verify the *deployed* assets rather than a local
// build, e.g. after copying them off the radio.
const templatePath = resolve(option('template',
  resolve(updateManagerRoot, 'templates/saturn-remote-next.html')));
const bundlePath = resolve(option('bundle',
  resolve(remoteWebRoot, 'dist/saturn-remote-next.js')));
const mode = option('mode', 'lan');
const seconds = Number(option('seconds', '20'));
const wsUrl = option('ws', 'ws://127.0.0.1:15001/');
// Extra page query, e.g. --query "&rx_audio_codec=pcm" for the PCM control arm.
const extraQuery = option('query', '');
// Soak mode: keep sampling for --soak-seconds instead of one final snapshot.
const soakSeconds = Number(option('soak-seconds', '0'));
const sampleSeconds = Number(option('sample-seconds', '60'));
const outFile = option('out', '');
const chrome = process.env.CHROMIUM || process.env.CHROME || 'google-chrome';
// Debug port first: the profile directory is unique per (mode, port) so a second
// probe cannot collide with a running one on Chrome's profile lock.
const debugPort = Number(option('debug-port', mode === 'lan' ? '9334' : '9333'));
const profileDir = join('/tmp', `live-rx-opus-${mode}-${debugPort}`);
mkdirSync(profileDir, { recursive: true });

// Installed in <head>: forces the tunnel endpoint, disables split transport
// (the bridge's loopback TCI is a single lane) and wraps AudioDecoder so every
// decode timestamp and every output timestamp is recorded.
const HEAD = `
  <script>
  (() => {
    localStorage.setItem('saturn.remote.splitTransport', 'off');
    localStorage.setItem('saturn.remote.streamMode', ${JSON.stringify(mode)});
    const RealSocket = window.WebSocket;
    window.WebSocket = class extends RealSocket {
      constructor(url, protocols) { super(${JSON.stringify(wsUrl)}, protocols); }
    };
    const trace = { decodes: [], outputs: [], creates: 0, errors: [],
      decodeCount: 0, outputCount: 0, mismatchCount: 0, pendingTs: [],
      lastOutputAt: null, maxGapMs: 0 };
    window.__opusTrace = trace;
    const Decoder = window.AudioDecoder;
    if (typeof Decoder === 'function') {
      window.AudioDecoder = class extends Decoder {
        constructor(init) {
          trace.creates += 1;
          super({
            output: (data) => {
              const at = performance.now();
              if (trace.lastOutputAt !== null) {
                const gap = at - trace.lastOutputAt;
                if (gap > trace.maxGapMs) trace.maxGapMs = gap;
              }
              trace.lastOutputAt = at;
              trace.outputCount += 1;
              trace.outputs.push({ at: performance.now(), ts: data.timestamp,
                frames: data.numberOfFrames, ch: data.numberOfChannels, sr: data.sampleRate });
              const expected = trace.pendingTs.shift();
              if (expected !== data.timestamp) trace.mismatchCount += 1;
              init.output(data);
            },
            error: (error) => {
              trace.errors.push({ at: performance.now(),
                message: String((error && error.message) || error) });
              init.error(error);
            },
          });
        }
        decode(chunk) {
          trace.decodes.push({ at: performance.now(), ts: chunk.timestamp,
            bytes: chunk.byteLength });
          trace.decodeCount += 1;
          trace.pendingTs.push(chunk.timestamp);
          return super.decode(chunk);
        }
      };
    }
  })();
  </script>
`;

// Installed at the end of <body>: connects, starts RX audio once the toggle is
// usable, then waits for real audio to flow.
const DRIVER = `
  <script>
  window.__probe = (async () => {
    const started = performance.now();
    const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    while (typeof window.saturnConnectBridgeDiagnostic !== 'function'
        && performance.now() - started < 8000) {
      await wait(100);
    }
    window.saturnConnectBridgeDiagnostic();
    let clicked = false;
    while (performance.now() - started < 20000) {
      const toggle = document.getElementById('rx-audio-toggle-btn');
      if (!clicked && toggle && !toggle.disabled
          && toggle.getAttribute('aria-pressed') !== 'true') {
        clicked = true;
        toggle.click();
      }
      if (clicked && window.__opusTrace && window.__opusTrace.decodes.length > 40) break;
      await wait(200);
    }
    return { clicked, atMs: performance.now() };
  })();
  </script>
`;

const template = readFileSync(templatePath, 'utf8');
const withHead = template.replace('<head>', `<head>\n${HEAD}`);
const bodyEnd = withHead.lastIndexOf('</body>');
const page = `${withHead.slice(0, bodyEnd)}${DRIVER}${withHead.slice(bodyEnd)}`;

const server = createServer((request, response) => {
  if ((request.url || '').includes('remote-next.js')) {
    response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8' });
    response.end(readFileSync(bundlePath));
    return;
  }
  response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
  response.end(page);
});
await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen));
const port = server.address().port;
const browser = spawn(chrome, [
  '--headless=new', '--disable-gpu', '--disable-dev-shm-usage', '--no-sandbox',
  '--mute-audio', '--autoplay-policy=no-user-gesture-required', '--no-first-run',
  '--disable-background-networking', '--disable-breakpad', '--disable-crash-reporter',
  '--disable-extensions', '--hide-scrollbars', '--window-size=1280,800',
  `--user-data-dir=${profileDir}`, `--remote-debugging-port=${debugPort}`,
  `http://127.0.0.1:${port}/remote-next?transport=legacy${extraQuery}`,
], { stdio: ['ignore', 'pipe', 'pipe'] });

const sleep = (ms) => new Promise((resolveWait) => setTimeout(resolveWait, ms));
let target = null;
for (let attempt = 0; attempt < 60 && !target; attempt += 1) {
  await sleep(500);
  try {
    const list = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
    target = list.find((entry) => entry.type === 'page' && entry.webSocketDebuggerUrl);
  } catch {}
}
function shutdown(code) {
  try { browser.kill('SIGTERM'); } catch {}
  server.close();
  process.exit(code);
}
if (!target) {
  console.error('live-rx-opus-probe: no Chrome debug target');
  shutdown(1);
}

const cdp = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolveOpen, rejectOpen) => {
  cdp.onopen = resolveOpen;
  cdp.onerror = () => rejectOpen(new Error('could not attach to Chrome'));
});
let nextId = 1;
const pendingCalls = new Map();
cdp.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pendingCalls.has(message.id)) {
    pendingCalls.get(message.id)(message);
    pendingCalls.delete(message.id);
  }
};
function evaluate(expression) {
  const id = (nextId += 1);
  return new Promise((resolveCall) => {
    pendingCalls.set(id, (message) => {
      const result = message.result?.result;
      if (message.result?.exceptionDetails) {
        resolveCall({ error: message.result.exceptionDetails.text });
        return;
      }
      try { resolveCall(JSON.parse(result.value)); } catch { resolveCall({ raw: result?.value }); }
    });
    cdp.send(JSON.stringify({ id, method: 'Runtime.evaluate',
      params: { expression, returnByValue: true, awaitPromise: true } }));
  });
}

await sleep(1500);
const driver = await evaluate('window.__probe.then((value) => JSON.stringify(value))');

// Soak mode: sample the browser's own counters once a minute and reset the
// in-page accumulators so an hour of monitoring does not grow without bound.
const SAMPLE_EXPR = `JSON.stringify((() => {
  const snap = window.SaturnRemotePerf && window.SaturnRemotePerf.snapshot
    ? window.SaturnRemotePerf.snapshot() : null;
  const trace = window.__opusTrace || {};
  const sample = {
    codec: snap && snap.rxAudioCodec ? snap.rxAudioCodec : null,
    decodes: trace.decodeCount || 0,
    outputs: trace.outputCount || 0,
    mismatchedTimestamps: trace.mismatchCount || 0,
    maxOutputGapMs: Math.round(trace.maxGapMs || 0),
    decoderCreates: trace.creates || 0,
    decoderErrors: (trace.errors || []).length,
  };
  trace.decodeCount = 0;
  trace.outputCount = 0;
  trace.mismatchCount = 0;
  trace.maxGapMs = 0;
  trace.decodes.length = 0;
  trace.outputs.length = 0;
  return sample;
})())`;

if (soakSeconds > 0) {
  const deadline = Date.now() + soakSeconds * 1000;
  let samples = 0;
  let worstOutputGapMs = 0;
  while (Date.now() < deadline) {
    await sleep(sampleSeconds * 1000);
    const sample = await evaluate(SAMPLE_EXPR);
    if (!sample || typeof sample !== 'object' || sample.error) {
      console.error(`sample ${samples + 1} failed: ${JSON.stringify(sample)}`);
      continue;
    }
    samples += 1;
    worstOutputGapMs = Math.max(worstOutputGapMs, sample.maxOutputGapMs || 0);
    const line = JSON.stringify({ ts: new Date().toISOString(), mode, ...sample });
    if (outFile) appendFileSync(outFile, `${line}\n`);
    console.log(line);
  }
  console.log(JSON.stringify({
    mode, wsUrl, driver, samples, worstOutputGapMs, out: outFile || null,
  }, null, 2));
  cdp.close();
  shutdown(0);
}

await sleep(seconds * 1000);
const collected = await evaluate(`JSON.stringify((() => {
  const snap = window.SaturnRemotePerf && window.SaturnRemotePerf.snapshot
    ? window.SaturnRemotePerf.snapshot() : null;
  const trace = window.__opusTrace || { decodes: [], outputs: [], creates: 0, errors: [] };
  const inTs = trace.decodes.map((entry) => entry.ts);
  const outTs = trace.outputs.map((entry) => entry.ts);
  const seen = new Set(outTs);
  const renumbered = inTs.filter((ts) => !seen.has(ts));
  const deltas = [];
  for (let i = 1; i < trace.outputs.length; i += 1) {
    deltas.push(trace.outputs[i].at - trace.outputs[i - 1].at);
  }
  deltas.sort((a, b) => a - b);
  return {
    codec: snap && snap.rxAudioCodec ? snap.rxAudioCodec : null,
    trace: {
      creates: trace.creates,
      decodes: trace.decodes.length,
      outputs: trace.outputs.length,
      errors: trace.errors.slice(0, 5),
      inputsWithoutMatchingOutput: renumbered.length,
      outputIntervalP50Ms: deltas.length ? Math.round(deltas[Math.floor(deltas.length / 2)]) : null,
      outputIntervalMaxMs: deltas.length ? Math.round(deltas[deltas.length - 1]) : null,
    },
  };
})())`);

console.log(JSON.stringify({ mode, wsUrl, driver, collected }, null, 2));
cdp.close();
shutdown(0);
