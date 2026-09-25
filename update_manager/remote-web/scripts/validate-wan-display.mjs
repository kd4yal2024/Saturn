#!/usr/bin/env node
// End-to-end WAN display validation against a stub bridge.
//
// Loads the real template and the real bundle in headless Chromium, with
// window.WebSocket replaced by a stub that speaks the frozen v2 contract:
//   greeting caps -> ready -> saturn_display:spectrum echo -> type-16 rows.
// It asserts the browser withholds iq_start until the echo, switches the
// render source to server rows, decodes them and acks every row.
import { existsSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const remoteWebRoot = resolve(scriptDir, '..');
const updateManagerRoot = resolve(remoteWebRoot, '..');
const templatePath = resolve(updateManagerRoot, 'templates/saturn-remote-next.html');
const bundlePath = resolve(remoteWebRoot, 'dist/saturn-remote-next.js');
const outputRoot = join(tmpdir(), 'saturn-wan-display');
const chromium = process.env.CHROMIUM || 'chromium';

function requireFile(path, label) {
  if (!existsSync(path)) throw new Error(`${label} not found: ${path}`);
}

const STUB_SOCKET = `
  <script>
  (() => {
    const sent = [];
    window.__wanSent = sent;
    localStorage.setItem('saturn.remote.streamMode', '__SATURN_MODE__');
    const ROWS_ENABLED = __ROWS_ENABLED__;
    const sockets = [];
    function buildRow(sequence) {
      const binCount = 2048;
      const buffer = new ArrayBuffer(64 + binCount);
      const view = new DataView(buffer);
      view.setUint32(4, 384000, true);
      view.setUint32(8, 0x5301, true);
      view.setUint32(12, 2048, true);
      view.setUint32(16, 1, true);
      view.setUint32(20, 2048, true);
      view.setUint32(24, 16, true);
      view.setUint32(28, 1, true);
      view.setUint32(32, sequence, true);
      view.setUint32(36, 14200000, true);
      view.setFloat32(44, -160, true);
      view.setFloat32(48, 0.625, true);
      const payload = new Uint8Array(buffer, 64);
      const peak = 300 + ((sequence * 7) % 1400);
      for (let i = 0; i < binCount; i += 1) {
        const db = -120 + 100 * Math.exp(-((i - peak) ** 2) / (2 * 40 ** 2));
        payload[i] = Math.max(0, Math.min(255, Math.round((db + 160) / 0.625)));
      }
      return buffer;
    }
    class StubWebSocket {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      constructor(url) {
        this.url = String(url);
        this.readyState = 0;
        this.bufferedAmount = 0;
        this.binaryType = 'arraybuffer';
        this.onopen = null; this.onmessage = null; this.onclose = null; this.onerror = null;
        this._listeners = { open: [], message: [], close: [], error: [] };
        this._rows = null;
        sockets.push(this);
        setTimeout(() => {
          this.readyState = 1;
          this._emit('open', {});
          if (!this._isMedia()) this._greet();
        }, 0);
      }
      _isMedia() { return this.url.includes('/saturn/media'); }
      _emit(type, event) {
        const handler = this['on' + type];
        if (typeof handler === 'function') handler(event);
        for (const listener of this._listeners[type] || []) listener(event);
      }
      addEventListener(type, listener) { (this._listeners[type] = this._listeners[type] || []).push(listener); }
      removeEventListener(type, listener) {
        const list = this._listeners[type] || [];
        const index = list.indexOf(listener);
        if (index >= 0) list.splice(index, 1);
      }
      close() { if (this.readyState < 3) { this.readyState = 3; this._emit('close', { code: 1000, wasClean: true, reason: '' }); } }
      send(data) {
        if (typeof data !== 'string') return;
        sent.push(data);
        if (data.indexOf('saturn_display:spectrum') === 0) {
          setTimeout(() => this._text('saturn_display:0,spectrum,2048,50;'), 5);
        }
        if (data.indexOf('iq_start:0;') >= 0) this._startRows();
      }
      _text(text) { this._emit('message', { data: text }); }
      _binary(buffer) { this._emit('message', { data: buffer }); }
      _greet() {
        const lines = [
          'protocol:SaturnBridge,2.0;', 'device:ANAN-G2;', 'receive_only:false;',
          'trx_count:1;', 'channel_count:2;', 'vfo_limits:10000,61440000;',
          'modulations_list:USB,LSB,CWU,CWL,AM,SAM,FM,NFM,DIGU,DIGL;',
          'saturn_satp_supported:false;', 'saturn_satp_enabled:false;',
          'saturn_satp_version:2;', 'saturn_satp_tx_port:0;',
          'saturn_satp_tx_format:48000,float32_le,1,128;', 'saturn_satp_feedback:false;',
          'saturn_display_caps:spectrum_u8;',
          'dds:0,14200000;', 'vfo:0,0,14200000;', 'vfo:0,1,14200000;',
          'iq_samplerate:384000;', 'modulation:0,USB;', 'rx_volume:0,0,-20;',
          'remote_client_role:0,operator,1;',
          'ready;',
        ];
        for (const line of lines) this._text(line);
      }
      _startRows() {
        if (this._rows || !ROWS_ENABLED) return;
        const media = sockets.find((socket) => socket._isMedia()) || this;
        let sequence = 0;
        this._rows = setInterval(() => {
          sequence += 1;
          media._binary(buildRow(sequence));
        }, 50);
      }
    }
    window.WebSocket = StubWebSocket;
  })();
  </script>
`;

const CHECKER = `
  <script>
  (() => {
    const started = performance.now();
    const report = (value) => {
      const node = document.createElement('script');
      node.id = 'wan-display-report';
      node.type = 'application/json';
      node.textContent = JSON.stringify(value);
      document.body.appendChild(node);
    };
    function poll() {
      const sent = window.__wanSent || [];
      const snap = window.SaturnRemotePerf && window.SaturnRemotePerf.snapshot
        ? window.SaturnRemotePerf.snapshot()
        : null;
      const pipeline = snap && snap.displayPipeline ? snap.displayPipeline : null;
      const server = pipeline && pipeline.serverDisplay ? pipeline.serverDisplay : null;
      // WAN: wait for the 1 s telemetry rollup so rows/s and bytes/s populate.
      // LAN: settle on a fixed window; there are no rows to wait for.
      const ready = __READY__;
      if (!ready && performance.now() - started < 9000) {
        setTimeout(poll, 100);
        return;
      }
      report({
        sent,
        perfType: typeof window.SaturnRemotePerf,
        snapshotKeys: snap ? Object.keys(snap) : null,
        pipelineKeys: snap && snap.displayPipeline ? Object.keys(snap.displayPipeline) : null,
        spectrumRequestIndex: sent.findIndex((t) => t.indexOf('saturn_display:spectrum') === 0),
        spectrumRequestCount: sent.filter((t) => t.indexOf('saturn_display:spectrum') === 0).length,
        iqStartIndex: sent.findIndex((t) => t.indexOf('iq_start:0;') >= 0),
        server,
        displaySource: pipeline ? pipeline.displaySource : null,
        displayFftSize: pipeline ? pipeline.displayFftSize : null,
        binSpacingHz: pipeline ? pipeline.binSpacingHz : null,
      });
    }
    const start = () => setTimeout(poll, 300);
    if (typeof window.saturnConnectBridgeDiagnostic === 'function') {
      try { window.saturnConnectBridgeDiagnostic(); } catch (error) { report({ error: String(error) }); return; }
      start();
    } else {
      report({ error: 'connect hook unavailable' });
    }
  })();
  </script>
`;

const SCENARIOS = {
  wan: {
    mode: 'wan',
    rowsEnabled: true,
    ready: 'server && server.rowsAccepted >= 5 && server.bytesPerSec > 0',
  },
  lan: {
    mode: 'lan',
    rowsEnabled: false,
    ready: 'performance.now() - started >= 3500',
  },
};

function buildPage(scenario) {
  const template = readFileSync(templatePath, 'utf8');
  if (!template.includes('<head>')) throw new Error('template has no <head>');
  const stub = STUB_SOCKET
    .replace('__SATURN_MODE__', scenario.mode)
    .replace('__ROWS_ENABLED__', String(scenario.rowsEnabled));
  const checker = CHECKER.replace('__READY__', scenario.ready);
  const withStub = template.replace('<head>', `<head>\n${stub}`);
  const bodyEnd = withStub.lastIndexOf('</body>');
  if (bodyEnd < 0) throw new Error('template has no </body>');
  return `${withStub.slice(0, bodyEnd)}${checker}${withStub.slice(bodyEnd)}`;
}

function runChromium(args, timeoutMs = 120000) {
  return new Promise((resolveRun, rejectRun) => {
    const proc = spawn(chromium, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => { proc.kill('SIGTERM'); rejectRun(new Error(`chromium timed out\n${stderr}`)); }, timeoutMs);
    proc.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
    proc.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
    proc.once('error', (error) => { clearTimeout(timer); rejectRun(error); });
    proc.once('exit', (code) => {
      clearTimeout(timer);
      if (code === 0) resolveRun(stdout);
      else rejectRun(new Error(`chromium exit ${code}\n${stderr || stdout}`));
    });
  });
}

async function runScenario(scenario) {
  const page = buildPage(scenario);
  const scenarioDir = join(outputRoot, scenario.mode);
  mkdirSync(scenarioDir, { recursive: true });
  const server = createServer((request, response) => {
    const url = request.url || '/';
    if (url.includes('remote-next.js')) {
      response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8' });
      response.end(readFileSync(bundlePath));
      return;
    }
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
    response.end(page);
  });
  await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen));
  const { port } = server.address();
  const args = [
    '--headless=new', '--disable-gpu', '--disable-dev-shm-usage', '--disable-extensions',
    '--disable-background-networking', '--disable-breakpad', '--disable-crash-reporter',
    '--disable-default-apps', '--disable-features=Crashpad', '--hide-scrollbars',
    '--mute-audio', '--autoplay-policy=no-user-gesture-required', '--no-first-run',
    '--no-default-browser-check', '--no-sandbox', '--force-device-scale-factor=1',
    '--window-size=1440,900', '--virtual-time-budget=12000',
    `--user-data-dir=${join(scenarioDir, 'profile')}`,
    '--dump-dom', `http://127.0.0.1:${port}/`,
  ];
  let dump;
  try {
    dump = await runChromium(args);
  } finally {
    server.close();
  }
  writeFileSync(join(scenarioDir, 'dump.html'), dump);
  const match = dump.match(/<script id="wan-display-report" type="application\/json">([\s\S]*?)<\/script>/);
  if (!match) throw new Error(`${scenario.mode}: DOM dump did not contain the wan-display-report marker`);
  const result = JSON.parse(match[1]);
  writeFileSync(join(scenarioDir, 'report.json'), JSON.stringify(result, null, 2));
  return result;
}

async function main() {
  requireFile(templatePath, 'remote-next template');
  requireFile(bundlePath, 'remote-next bundle (run npm run build)');
  rmSync(outputRoot, { recursive: true, force: true });
  mkdirSync(outputRoot, { recursive: true });
  const failures = [];
  const check = (ok, message) => { if (!ok) failures.push(message); };

  const wan = await runScenario(SCENARIOS.wan);
  check(!wan.error, `WAN page reported: ${wan.error}`);
  check(wan.spectrumRequestIndex >= 0, 'WAN: browser never requested spectrum rows');
  // `ready;` and the iq_start gate must not both fire a request.
  check(wan.spectrumRequestCount === 1, `WAN: sent ${wan.spectrumRequestCount} spectrum requests, expected 1`);
  check(wan.iqStartIndex >= 0, 'WAN: browser never sent iq_start');
  check(wan.spectrumRequestIndex >= 0 && wan.iqStartIndex > wan.spectrumRequestIndex,
    'WAN: iq_start was sent before the spectrum request (echo gate not applied)');
  check(wan.server && wan.server.mode === 'spectrum', `WAN: display mode is ${wan.server && wan.server.mode}`);
  check(wan.server && wan.server.rowsAccepted >= 5, `WAN: accepted rows ${wan.server && wan.server.rowsAccepted}`);
  check(wan.server && wan.server.rowsRejected === 0, `WAN: rejected rows ${wan.server && wan.server.rowsRejected}`);
  check(wan.server && wan.server.ackCount >= wan.server.rowsAccepted, 'WAN: not every accepted row was acked');
  check(wan.server && wan.server.bytesPerSec > 0, 'WAN: no measured row byte rate');
  check(wan.displaySource === 'server', `WAN: render source is ${wan.displaySource}`);
  check(wan.displayFftSize === 2048, `WAN: display fft size is ${wan.displayFftSize}`);
  check(wan.binSpacingHz === 384000 / 2048, `WAN: bin spacing is ${wan.binSpacingHz}`);

  const lan = await runScenario(SCENARIOS.lan);
  check(!lan.error, `LAN page reported: ${lan.error}`);
  check(lan.spectrumRequestIndex === -1, 'LAN: browser requested spectrum rows on LAN');
  check(lan.sent.includes('saturn_display:iq;'), 'LAN: browser did not assert the raw IQ mode');
  check(lan.iqStartIndex >= 0, 'LAN: browser never sent iq_start');
  check(lan.displaySource === 'iq', `LAN: render source is ${lan.displaySource}`);
  check(!lan.server || lan.server.rowsAccepted === 0, `LAN: accepted ${lan.server && lan.server.rowsAccepted} rows`);

  if (failures.length > 0) {
    console.error('validate-wan-display: FAILED');
    for (const failure of failures) console.error(`  - ${failure}`);
    console.error(JSON.stringify({ wan, lan }, null, 2));
    process.exit(1);
  }
  console.log('validate-wan-display: PASS');
  console.log(`  WAN: request ${wan.spectrumRequestIndex}, iq_start ${wan.iqStartIndex}, rows ${wan.server.rowsAccepted} acked ${wan.server.ackCount}, ${Math.round(wan.server.bytesPerSec)} B/s`);
  console.log(`  WAN: source ${wan.displaySource}, fft ${wan.displayFftSize}, bin ${wan.binSpacingHz} Hz`);
  console.log(`  LAN: source ${lan.displaySource}, spectrum requests ${lan.spectrumRequestIndex}, iq_start ${lan.iqStartIndex}`);
  console.log(`  output: ${outputRoot}`);
}

main().catch((error) => {
  console.error(`validate-wan-display: ERROR ${error.message}`);
  process.exit(1);
});
