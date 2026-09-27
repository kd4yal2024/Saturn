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
import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const remoteWebRoot = resolve(scriptDir, '..');
const updateManagerRoot = resolve(remoteWebRoot, '..');
const templatePath = resolve(updateManagerRoot, 'templates/saturn-remote-next.html');
const bundlePath = resolve(remoteWebRoot, 'dist/saturn-remote-next.js');

function option(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

const mode = option('mode', 'lan');
const seconds = Number(option('seconds', '20'));
const wsUrl = option('ws', 'ws://127.0.0.1:15001/');
const chrome = process.env.CHROMIUM || process.env.CHROME || 'google-chrome';
const profileDir = join('/tmp', `live-rx-opus-${mode}`);
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
    const trace = { decodes: [], outputs: [], creates: 0, errors: [] };
    window.__opusTrace = trace;
    const Decoder = window.AudioDecoder;
    if (typeof Decoder === 'function') {
      window.AudioDecoder = class extends Decoder {
        constructor(init) {
          trace.creates += 1;
          super({
            output: (data) => {
              trace.outputs.push({ at: performance.now(), ts: data.timestamp,
                frames: data.numberOfFrames, ch: data.numberOfChannels, sr: data.sampleRate });
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
const debugPort = Number(option('debug-port', mode === 'lan' ? '9334' : '9333'));
const browser = spawn(chrome, [
  '--headless=new', '--disable-gpu', '--disable-dev-shm-usage', '--no-sandbox',
  '--mute-audio', '--autoplay-policy=no-user-gesture-required', '--no-first-run',
  '--disable-background-networking', '--disable-breakpad', '--disable-crash-reporter',
  '--disable-extensions', '--hide-scrollbars', '--window-size=1280,800',
  `--user-data-dir=${profileDir}`, `--remote-debugging-port=${debugPort}`,
  `http://127.0.0.1:${port}/remote-next?transport=legacy`,
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
