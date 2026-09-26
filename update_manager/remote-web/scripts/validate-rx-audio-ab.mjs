#!/usr/bin/env node
// RX audio Phase 0 A/B: PCM over the WS media lane versus shared Opus.
//
// Loads the real template and the real bundle in headless Chrome, with
// window.WebSocket replaced by a stub bridge that speaks the RX audio contract:
//   audio_gain:client echo -> audio_codec:opus|pcm echo -> type-17 / type-1 frames
// It measures on-wire bytes per arm and asserts the negotiation, the capability
// fallback, the malformed-frame policy and the decoder-backlog resync.
//
// This is the browser-logic half of the A/B. Real libopus decode quality and
// real WAN behaviour still need the live radio run (see RX_AUDIO_AB.md).
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
const outputRoot = join(tmpdir(), 'saturn-rx-audio-ab');
const chromium = process.env.CHROMIUM || process.env.CHROME || 'google-chrome';

// 20 ms of 12 kHz mono f32: the WAN PCM profile the bridge still sends today.
const PCM_FRAME_BYTES = 64 + 240 * 4;
// 20 ms of 48 kHz stereo f32: the LAN PCM profile, unchanged by this work.
const PCM_LAN_FRAME_BYTES = 64 + 960 * 2 * 4;
// 20 ms Opus packets: mono at 24 kbit/s and stereo at 80 kbit/s, payload only.
const OPUS_MONO_FRAME_BYTES = 64 + 60;
const OPUS_STEREO_FRAME_BYTES = 64 + 200;

const STUB_SOCKET = `
  <script>
  (() => {
    const sent = [];
    window.__abSent = sent;
    const stats = { framesSent: 0, lane: '', startedAtMs: null, endedAtMs: null };
    window.__abStats = stats;
    const CONFIG = __CONFIG__;
    localStorage.setItem('saturn.remote.streamMode', CONFIG.streamMode);
    const sockets = [];
    function opusFrame(sequence, payloadBytes, channels) {
      const buffer = new ArrayBuffer(64 + payloadBytes);
      const view = new DataView(buffer);
      view.setUint32(4, 48000, true);
      view.setUint32(8, 20, true);
      view.setUint32(12, 0, true);
      view.setUint32(16, 960, true);
      view.setUint32(20, payloadBytes, true);
      view.setUint32(24, 17, true);
      view.setUint32(28, channels, true);
      view.setUint32(32, sequence, true);
      new Uint8Array(buffer, 64).fill(0x5a);
      return buffer;
    }
    function pcmFrame(sequence) {
      const samples = CONFIG.pcmSamples;
      const channels = CONFIG.pcmChannels;
      const buffer = new ArrayBuffer(64 + samples * channels * 4);
      const view = new DataView(buffer);
      view.setUint32(4, CONFIG.pcmRate, true);
      view.setUint32(20, samples * channels, true);
      view.setUint32(24, 1, true);
      view.setUint32(28, channels, true);
      view.setUint32(32, sequence, true);
      const floats = new Float32Array(buffer, 64, samples * channels);
      for (let i = 0; i < samples; i += 1) {
        const value = 0.2 * Math.sin((2 * Math.PI * 1000 * (i + sequence * samples)) / CONFIG.pcmRate);
        for (let channel = 0; channel < channels; channel += 1) {
          floats[i * channels + channel] = channel === 0 ? value : value * 0.5;
        }
      }
      return buffer;
    }
    function streamFrame(mode, sequence) {
      if (mode === 'pcm') return pcmFrame(sequence);
      const frame = opusFrame(sequence, CONFIG.opusPayloadBytes, CONFIG.opusChannels);
      // Keep stream type 17 so the frame reaches the Opus path, then break the
      // 20 ms frame-duration field: a wire-contract mismatch, not a bad type.
      if (mode === 'malformed') new DataView(frame).setUint32(8, 40, true);
      return frame;
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
        this._timer = null;
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
      close() {
        if (this.readyState < 3) {
          this.readyState = 3;
          this._emit('close', { code: 1000, wasClean: true, reason: '' });
        }
      }
      _text(text) { this._emit('message', { data: text }); }
      _binary(buffer) { this._emit('message', { data: buffer }); }
      _mediaSocket() { return sockets.find((socket) => socket._isMedia()) || this; }
      // Frames are burst rather than streamed on a timer: once the page renders
      // audio, Chrome stops fast-forwarding the virtual clock, so a timer-driven
      // stream would deliver an arbitrary handful of frames. The A/B measures
      // bytes/frame, which a burst preserves exactly.
      _burst() {
        if (this._timer) return;
        const media = this._mediaSocket();
        stats.lane = media === this ? 'control' : 'media';
        stats.startedAtMs = performance.now();
        this._timer = true;
        setTimeout(() => {
          for (let sequence = 1; sequence <= CONFIG.frames; sequence += 1) {
            stats.framesSent += 1;
            stats.endedAtMs = performance.now();
            media._binary(streamFrame(CONFIG.mode, sequence));
          }
        }, 0);
      }
      send(data) {
        if (typeof data !== 'string') return;
        sent.push(data);
        if (data === 'audio_gain:client;') { this._text('audio_gain:client;'); return; }
        if (data === 'audio_codec:opus;') {
          this._text('audio_codec:opus;audio_samplerate:48000;');
          return;
        }
        if (data === 'audio_codec:pcm;') {
          this._text('audio_codec:pcm;');
          return;
        }
        if (data.includes('audio_start:0;')) {
          this._text('audio_start:0;');
          this._burst();
          return;
        }
      }
      _greet() {
        const lines = [
          'protocol:SaturnBridge,2.0;', 'device:ANAN-G2;', 'receive_only:false;',
          'trx_count:1;', 'channel_count:2;', 'vfo_limits:10000,61440000;',
          'modulations_list:USB,LSB,CWU,CWL,AM,SAM,FM,NFM,DIGU,DIGL;',
          'saturn_satp_supported:false;', 'saturn_satp_enabled:false;',
          'saturn_satp_version:2;', 'saturn_satp_tx_port:0;',
          'saturn_satp_tx_format:48000,float32_le,1,128;', 'saturn_satp_feedback:false;',
          'dds:0,14200000;', 'vfo:0,0,14200000;', 'vfo:0,1,14200000;',
          'iq_samplerate:384000;', 'modulation:0,USB;', 'rx_volume:0,0,-10;',
          'remote_client_role:0,operator,1;',
          'ready;',
        ];
        for (const line of lines) this._text(line);
      }
    }
    window.WebSocket = StubWebSocket;
    // Ask what the real browser supports, then install the deterministic decoder
    // the A/B measures with (or remove WebCodecs for the fallback arm).
    window.__abRealOpus = (async () => {
      try {
        if (typeof window.AudioDecoder !== 'function') return false;
        const stereo = await window.AudioDecoder.isConfigSupported(
          { codec: 'opus', sampleRate: 48000, numberOfChannels: 2 });
        const mono = await window.AudioDecoder.isConfigSupported(
          { codec: 'opus', sampleRate: 48000, numberOfChannels: 1 });
        return Boolean(stereo.supported && mono.supported);
      } catch (_) { return false; }
    })();
    if (CONFIG.decoder === 'fake' || CONFIG.decoder === 'silent') {
      class FakeAudioDecoder {
        static async isConfigSupported() { return { supported: true }; }
        constructor(init) { this.init = init; this.channels = 1; }
        configure(config) { this.channels = config.numberOfChannels; }
        close() {}
        decode(chunk) {
          if (CONFIG.decoder === 'silent') return;
          this.init.output({
            timestamp: chunk.timestamp,
            sampleRate: 48000,
            numberOfChannels: this.channels,
            numberOfFrames: 960,
            copyTo(destination, options) {
              destination.fill(options.planeIndex === 0 ? 0.2 : -0.2);
            },
            close() {},
          });
        }
      }
      window.AudioDecoder = FakeAudioDecoder;
      window.EncodedAudioChunk = class { constructor(init) { Object.assign(this, init); } };
    } else if (CONFIG.decoder === 'absent') {
      window.AudioDecoder = undefined;
      window.EncodedAudioChunk = undefined;
    }
  })();
  </script>
`;

const CHECKER = `
  <script>
  (() => {
    const started = performance.now();
    const report = (value) => {
      const node = document.createElement('script');
      node.id = 'rx-audio-ab-report';
      node.type = 'application/json';
      node.textContent = JSON.stringify(value);
      document.body.appendChild(node);
    };
    function snapshot() {
      try {
        return window.SaturnRemotePerf && window.SaturnRemotePerf.snapshot
          ? window.SaturnRemotePerf.snapshot()
          : null;
      } catch (error) {
        return { error: String(error) };
      }
    }
    function poll() {
      const sent = window.__abSent || [];
      const toggle = document.getElementById('rx-audio-toggle-btn');
      if (!window.__abClicked && toggle && !toggle.disabled
          && toggle.getAttribute('aria-pressed') !== 'true') {
        window.__abClicked = true;
        toggle.click();
      }
      const snap = snapshot();
      const codec = snap && snap.rxAudioCodec ? snap.rxAudioCodec : null;
      const seen = codec ? (codec.pcmBytes + codec.opusBytes) : 0;
      if (__READY_EXPR__ && performance.now() - started < 8000) {
        setTimeout(poll, 50);
        return;
      }
      Promise.resolve(window.__abRealOpus).then((realOpus) => {
        const final = snapshot();
        report({
          sent,
          realWebCodecsOpus: Boolean(realOpus),
          audioStreaming: Boolean(toggle && toggle.getAttribute('aria-pressed') === 'true'),
          perfType: typeof window.SaturnRemotePerf,
          snapshotKeys: final ? Object.keys(final) : null,
          codec: final && final.rxAudioCodec ? final.rxAudioCodec : null,
          framesSeenBytes: seen,
          stream: window.__abStats || null,
        });
      });
    }
    const start = () => setTimeout(poll, 250);
    const connect = () => {
      try {
        window.saturnConnectBridgeDiagnostic();
      } catch (error) {
        report({ error: String(error) });
        return;
      }
      start();
    };
    if (typeof window.saturnConnectBridgeDiagnostic === 'function') {
      connect();
    } else {
      setTimeout(() => {
        if (typeof window.saturnConnectBridgeDiagnostic !== 'function') {
          report({ error: 'connect hook unavailable' });
          return;
        }
        connect();
      }, 100);
    }
  })();
  </script>
`;

// Each arm states what it wants to observe. `mode` chooses the wire the stub
// streams once the negotiation settles; `decoder` chooses WebCodecs behaviour.
const SCENARIOS = [
  {
    name: 'opus-wan-auto',
    query: '',
    streamMode: 'wan',
    mode: 'opus',
    decoder: 'fake',
    opusPayloadBytes: 60,
    opusChannels: 1,
    pcmRate: 12000,
    pcmChannels: 1,
    pcmSamples: 240,
    malformedFrames: 0,
    frames: 40,
    ready: 'codec && codec.opusFrames >= 40',
  },
  {
    name: 'pcm-wan-forced',
    query: '?rx_audio_codec=pcm',
    streamMode: 'wan',
    mode: 'pcm',
    decoder: 'fake',
    opusPayloadBytes: 60,
    opusChannels: 1,
    pcmRate: 12000,
    pcmChannels: 1,
    pcmSamples: 240,
    malformedFrames: 0,
    frames: 40,
    ready: 'codec && codec.pcmFrames >= 40',
  },
  {
    name: 'opus-lan-stereo-auto',
    query: '',
    streamMode: 'lan',
    mode: 'opus',
    decoder: 'fake',
    opusPayloadBytes: 200,
    opusChannels: 2,
    pcmRate: 48000,
    pcmChannels: 2,
    pcmSamples: 960,
    malformedFrames: 0,
    frames: 40,
    ready: 'codec && codec.opusFrames >= 40',
  },
  {
    name: 'pcm-lan-stereo-forced',
    query: '?rx_audio_codec=pcm',
    streamMode: 'lan',
    mode: 'pcm',
    decoder: 'fake',
    opusPayloadBytes: 200,
    opusChannels: 2,
    pcmRate: 48000,
    pcmChannels: 2,
    pcmSamples: 960,
    malformedFrames: 0,
    frames: 40,
    ready: 'codec && codec.pcmFrames >= 40',
  },
  {
    name: 'opus-unavailable-falls-back',
    query: '',
    streamMode: 'wan',
    mode: 'pcm',
    decoder: 'absent',
    opusPayloadBytes: 60,
    opusChannels: 1,
    pcmRate: 12000,
    pcmChannels: 1,
    pcmSamples: 240,
    malformedFrames: 0,
    frames: 10,
    ready: 'codec && codec.pcmFrames >= 10',
  },
  {
    name: 'opus-malformed-run-falls-back',
    query: '',
    streamMode: 'wan',
    mode: 'malformed',
    decoder: 'fake',
    opusPayloadBytes: 60,
    opusChannels: 1,
    pcmRate: 12000,
    pcmChannels: 1,
    pcmSamples: 240,
    malformedFrames: 6,
    frames: 6,
    ready: 'codec && codec.malformedFrames >= 5',
  },
  {
    name: 'opus-decoder-backlog-resyncs',
    query: '',
    streamMode: 'wan',
    mode: 'silent',
    decoder: 'silent',
    opusPayloadBytes: 60,
    opusChannels: 1,
    pcmRate: 12000,
    pcmChannels: 1,
    pcmSamples: 240,
    malformedFrames: 0,
    frames: 40,
    ready: 'codec && codec.resyncs >= 1',
  },
];

function requireFile(path, label) {
  if (!existsSync(path)) throw new Error(`${label} not found: ${path}`);
}

function buildPage(scenario) {
  const template = readFileSync(templatePath, 'utf8');
  if (!template.includes('<head>')) throw new Error('template has no <head>');
  const config = {
    mode: scenario.mode,
    decoder: scenario.decoder,
    streamMode: scenario.streamMode,
    pcmRate: scenario.pcmRate,
    pcmChannels: scenario.pcmChannels,
    pcmSamples: scenario.pcmSamples,
    opusPayloadBytes: scenario.opusPayloadBytes,
    opusChannels: scenario.opusChannels,
    malformedFrames: scenario.malformedFrames,
    frames: scenario.frames,
  };
  const stub = STUB_SOCKET
    .replace('__CONFIG__', JSON.stringify(config));
  const checker = CHECKER.replace('__READY_EXPR__', scenario.ready);
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
    const timer = setTimeout(() => {
      proc.kill('SIGTERM');
      rejectRun(new Error(`chrome timed out\n${stderr}`));
    }, timeoutMs);
    proc.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
    proc.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
    proc.once('error', (error) => { clearTimeout(timer); rejectRun(error); });
    proc.once('exit', (code) => {
      clearTimeout(timer);
      if (code === 0) resolveRun(stdout);
      else rejectRun(new Error(`chrome exit ${code}\n${stderr || stdout}`));
    });
  });
}

async function runScenario(scenario) {
  const page = buildPage(scenario);
  const scenarioDir = join(outputRoot, scenario.name);
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
    '--dump-dom', `http://127.0.0.1:${port}/${scenario.query}`,
  ];
  let dump;
  try {
    dump = await runChromium(args);
  } finally {
    server.close();
  }
  writeFileSync(join(scenarioDir, 'dump.html'), dump);
  const match = dump.match(/<script id="rx-audio-ab-report" type="application\/json">([\s\S]*?)<\/script>/);
  if (!match) throw new Error(`${scenario.name}: DOM dump did not contain the rx-audio-ab-report marker`);
  const result = JSON.parse(match[1]);
  writeFileSync(join(scenarioDir, 'report.json'), JSON.stringify(result, null, 2));
  return result;
}

function bytesPerFrame(codec, prefix) {
  const frames = Number(codec[`${prefix}Frames`]) || 0;
  const bytes = Number(codec[`${prefix}Bytes`]) || 0;
  return frames > 0 ? bytes / frames : 0;
}

function within(actual, expected, tolerance) {
  return Math.abs(actual - expected) <= expected * tolerance;
}

async function main() {
  requireFile(templatePath, 'remote-next template');
  requireFile(bundlePath, 'remote-next bundle (run npm run build)');
  rmSync(outputRoot, { recursive: true, force: true });
  mkdirSync(outputRoot, { recursive: true });
  const failures = [];
  const check = (ok, message) => { if (!ok) failures.push(message); };
  const results = {};

  for (const scenario of SCENARIOS) {
    const result = await runScenario(scenario);
    results[scenario.name] = result;
    const codec = result.codec || {};
    const sent = result.sent || [];
    const opusRequest = sent.indexOf('audio_codec:opus;');
    const pcmRequest = sent.indexOf('audio_codec:pcm;');
    const gainRequest = sent.indexOf('audio_gain:client;');
    const label = scenario.name;

    check(!result.error, `${label}: page reported ${result.error}`);
    check(gainRequest >= 0, `${label}: browser never asked for client-side gain`);
    check(result.audioStreaming === true, `${label}: RX audio never started`);

    if (scenario.name === 'opus-wan-auto') {
      check(opusRequest > gainRequest, `${label}: Opus offered before the gain echo`);
      check(!sent.includes('audio_codec:pcm;'), `${label}: browser also requested PCM`);
      check(codec.accepted === 'opus', `${label}: accepted codec is ${codec.accepted}`);
      check(codec.clientGainEcho === true, `${label}: no client-gain echo recorded`);
      check(codec.decodedFrames >= 25, `${label}: decoded ${codec.decodedFrames} Opus frames`);
      check(codec.malformedFrames === 0 && codec.decoderErrors === 0,
        `${label}: malformed ${codec.malformedFrames} / decoder errors ${codec.decoderErrors}`);
      check(codec.resyncs === 0, `${label}: unexpected resyncs ${codec.resyncs}`);
      check(codec.sequenceGaps === 0, `${label}: ${codec.sequenceGaps} sequence gaps`);
      check(within(bytesPerFrame(codec, 'opus'), OPUS_MONO_FRAME_BYTES, 0.02),
        `${label}: ${bytesPerFrame(codec, 'opus').toFixed(1)} B/frame, expected ${OPUS_MONO_FRAME_BYTES}`);
    }

    if (scenario.name === 'pcm-wan-forced') {
      check(gainRequest >= 0 && pcmRequest > gainRequest, `${label}: PCM arm did not negotiate gain first`);
      check(!sent.includes('audio_codec:opus;'), `${label}: browser requested Opus while forced to PCM`);
      check(codec.accepted === 'pcm', `${label}: accepted codec is ${codec.accepted}`);
      check(codec.capability === 'forced PCM', `${label}: capability is ${codec.capability}`);
      check(codec.decodedFrames === 0, `${label}: decoded Opus frames on the PCM arm`);
      check(within(bytesPerFrame(codec, 'pcm'), PCM_FRAME_BYTES, 0.02),
        `${label}: ${bytesPerFrame(codec, 'pcm').toFixed(1)} B/frame, expected ${PCM_FRAME_BYTES}`);
    }

    if (scenario.name === 'opus-lan-stereo-auto') {
      check(opusRequest > gainRequest, `${label}: Opus offered before the gain echo`);
      check(!sent.includes('audio_codec:pcm;'), `${label}: browser also requested PCM`);
      check(codec.accepted === 'opus', `${label}: accepted codec is ${codec.accepted}`);
      check(codec.decoderChannels === 2, `${label}: decoder channels ${codec.decoderChannels}`);
      check(codec.decodedFrames >= 25, `${label}: decoded ${codec.decodedFrames} Opus frames`);
      check(codec.malformedFrames === 0 && codec.decoderErrors === 0,
        `${label}: malformed ${codec.malformedFrames} / decoder errors ${codec.decoderErrors}`);
      check(within(bytesPerFrame(codec, 'opus'), OPUS_STEREO_FRAME_BYTES, 0.02),
        `${label}: ${bytesPerFrame(codec, 'opus').toFixed(1)} B/frame, expected ${OPUS_STEREO_FRAME_BYTES}`);
    }

    if (scenario.name === 'pcm-lan-stereo-forced') {
      check(!sent.includes('audio_codec:opus;'), `${label}: browser requested Opus while forced to PCM`);
      check(codec.accepted === 'pcm', `${label}: accepted codec is ${codec.accepted}`);
      check(codec.capability === 'forced PCM', `${label}: capability is ${codec.capability}`);
      check(codec.decodedFrames === 0, `${label}: decoded Opus frames on the PCM arm`);
      check(within(bytesPerFrame(codec, 'pcm'), PCM_LAN_FRAME_BYTES, 0.02),
        `${label}: ${bytesPerFrame(codec, 'pcm').toFixed(1)} B/frame, expected ${PCM_LAN_FRAME_BYTES}`);
    }

    if (scenario.name === 'opus-unavailable-falls-back') {
      check(!sent.includes('audio_codec:opus;'), `${label}: offered Opus without a decoder`);
      check(pcmRequest > gainRequest, `${label}: did not select PCM`);
      check(codec.accepted === 'pcm', `${label}: accepted codec is ${codec.accepted}`);
      check(codec.capability === 'WebCodecs unavailable', `${label}: capability is ${codec.capability}`);
      check(codec.fallbackReason === 'WebCodecs Opus decoder unavailable',
        `${label}: fallback reason is ${codec.fallbackReason}`);
    }

    if (scenario.name === 'opus-malformed-run-falls-back') {
      check(opusRequest > gainRequest, `${label}: Opus was never offered`);
      check(codec.malformedFrames >= 5, `${label}: only ${codec.malformedFrames} malformed frames seen`);
      check(codec.accepted === 'pcm', `${label}: accepted codec is ${codec.accepted}`);
      check(String(codec.fallbackReason).startsWith('malformed Opus frame'),
        `${label}: fallback reason is ${codec.fallbackReason}`);
      check(sent.includes('audio_codec:pcm;'), `${label}: browser never asked the bridge for PCM`);
      check(codec.decodedFrames === 0, `${label}: decoded frames on a malformed-only stream`);
    }

    if (scenario.name === 'opus-decoder-backlog-resyncs') {
      check(codec.accepted === 'opus', `${label}: accepted codec changed to ${codec.accepted}`);
      check(codec.resyncs >= 1, `${label}: decoder backlog did not resync`);
      check(!sent.includes('audio_codec:pcm;'), `${label}: abandoned Opus on a decoder stall`);
      check(codec.decodedFrames === 0, `${label}: silent decoder reported decoded frames`);
    }
  }

  const opusArm = results['opus-wan-auto'] || {};
  const pcmArm = results['pcm-wan-forced'] || {};
  const opusBytesPerFrame = bytesPerFrame(opusArm.codec || {}, 'opus');
  const pcmBytesPerFrame = bytesPerFrame(pcmArm.codec || {}, 'pcm');
  const ratio = opusBytesPerFrame > 0 ? pcmBytesPerFrame / opusBytesPerFrame : 0;
  check(ratio > 6, `A/B: measured on-wire reduction is only ${ratio.toFixed(2)}x`);

  const lanOpusArm = results['opus-lan-stereo-auto'] || {};
  const lanPcmArm = results['pcm-lan-stereo-forced'] || {};
  const lanOpusBytesPerFrame = bytesPerFrame(lanOpusArm.codec || {}, 'opus');
  const lanPcmBytesPerFrame = bytesPerFrame(lanPcmArm.codec || {}, 'pcm');
  const lanRatio = lanOpusBytesPerFrame > 0 ? lanPcmBytesPerFrame / lanOpusBytesPerFrame : 0;
  check(lanRatio > 6, `A/B: measured LAN on-wire reduction is only ${lanRatio.toFixed(2)}x`);

  const summary = {
    generatedAtIso: new Date().toISOString(),
    measured: {
      pcmBytesPerFrame,
      opusMonoBytesPerFrame: opusBytesPerFrame,
      onWireReduction: Number(ratio.toFixed(2)),
      pcmLanStereoBytesPerFrame: lanPcmBytesPerFrame,
      opusLanStereoBytesPerFrame: lanOpusBytesPerFrame,
      lanOnWireReduction: Number(lanRatio.toFixed(2)),
      opusMonoPayloadOnlyKbit: 24,
      opusStereoPayloadOnlyKbit: 80,
      pcmWanMonoKbit: ((PCM_FRAME_BYTES - 64) * 8 * 50) / 1000,
      pcmLanStereoKbit: ((PCM_LAN_FRAME_BYTES - 64) * 8 * 50) / 1000,
      realWebCodecsOpus: Boolean(opusArm.realWebCodecsOpus),
      arms: Object.fromEntries(Object.entries(results).map(([name, result]) => {
        const codec = result.codec || {};
        return [name, {
          accepted: codec.accepted,
          capability: codec.capability,
          fallbackReason: codec.fallbackReason,
          decodedFrames: codec.decodedFrames,
          malformedFrames: codec.malformedFrames,
          decoderErrors: codec.decoderErrors,
          lateDrops: codec.lateDrops,
          resyncs: codec.resyncs,
          pendingPackets: codec.pendingPackets,
          sequenceGaps: codec.sequenceGaps,
          jitterP99Ms: codec.jitterP99Ms,
          workletUnderruns: codec.workletUnderruns,
          queueMs: codec.queueMs,
          pcmBytesPerSec: codec.pcmBytesPerSec,
          opusBytesPerSec: codec.opusBytesPerSec,
        }];
      })),
    },
  };
  writeFileSync(join(outputRoot, 'summary.json'), JSON.stringify(summary, null, 2));

  if (failures.length > 0) {
    console.error('validate-rx-audio-ab: FAILED');
    for (const failure of failures) console.error(`  - ${failure}`);
    console.error(JSON.stringify(summary, null, 2));
    process.exit(1);
  }
  console.log('validate-rx-audio-ab: PASS');
  console.log(`  WAN  12 kHz mono f32 PCM: ${pcmBytesPerFrame.toFixed(1)} B/frame  vs Opus ${opusBytesPerFrame.toFixed(1)} B/frame  -> ${ratio.toFixed(2)}x`);
  console.log(`  LAN  48 kHz stereo f32 PCM: ${lanPcmBytesPerFrame.toFixed(1)} B/frame  vs Opus ${lanOpusBytesPerFrame.toFixed(1)} B/frame  -> ${lanRatio.toFixed(2)}x`);
  console.log(`  Opus on-wire: mono ${(opusBytesPerFrame * 50 / 125).toFixed(1)} kbit/s, stereo ${(lanOpusBytesPerFrame * 50 / 125).toFixed(1)} kbit/s`);
  console.log(`  Arms: ${Object.keys(results).join(', ')}`);
  console.log(`  output: ${outputRoot}`);
}

main().catch((error) => {
  console.error(`validate-rx-audio-ab: ERROR ${error.message}`);
  process.exit(1);
});
