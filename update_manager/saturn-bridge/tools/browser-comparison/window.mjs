// One measurement window, in a real browser, against a Bridge reached through a proxy.
//
//   node window.mjs PORT LOGIN URL BRIDGE_WORK_DIR SECONDS OUT MODE META_JSON
//
//   PORT          Chrome's DevTools port (headless Chrome already running)
//   LOGIN         user:password for the proxy's basic auth
//   URL           the remote page, with ?display_transport=iq|spectrum
//   BRIDGE_WORK_DIR  directory holding the Bridge's perf.json (local rehearsal: the replay Bridge's work dir)
//   SECONDS       measured span after warm-up (and, for mode c, after the drain)
//   OUT           path prefix; the record is written to OUT.json (and OUT.png)
//   MODE          'normal' (display as the URL selects) or 'c' (audio only: stopIq() with the automatic restart suppressed)
//   META_JSON     identity of the window: index, arm, bridgeNoDelay, bridgePid, bridgeSha256, bridgeRestartNumber, rxc1
//
// Where the Bridge's telemetry comes from (the same for every window of a run):
//   unset                         BRIDGE_WORK_DIR/perf.json is read directly (the local replay rehearsal)
//   SATURN_CMP_COLLECTOR=local    the same file, through the read-only reader (owner_reader.py): adds the Pi-side age and
//                                 the owner's process identity, retains every raw answer in OUT.collector.jsonl
//   SATURN_CMP_COLLECTOR=ssh      the owner's cached document on the Pi, through the same reader over SSH
//                                 (SATURN_CMP_COLLECTOR_HOST, and optionally _PATH and _SERVICE); see collector.mjs
//
// The record is written on success AND on failure. If anything throws, what was recorded so far is kept, a
// `failure` field is added, and the process exits nonzero. The checker rejects a record that carries a failure.
import { connect, sleep } from './cdp.mjs';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { collectorFromEnv, mapMetrics } from './collector.mjs';

const [,, port, login, url, workDir, seconds, out, mode, metaJson] = process.argv;
const record = {
  url, mode, meta: JSON.parse(metaJson || '{}'), stoppedAt: null,
  atConnect: null, atWarm: null, start: null, samples: [], frames: [], logs: [],
};
let collector = null;
const save = () => {
  if (collector) record.collector = collector.summary();
  fs.writeFileSync(out + '.json', JSON.stringify(record, null, 1));
};
let c = null;

try {
  collector = collectorFromEnv(process.env, { workDir, sidecar: out + '.collector.jsonl', readerPath: fileURLToPath(new URL('./owner_reader.py', import.meta.url)) });
  const [user, pass] = login.split(':');
  c = await connect(port);
  c.on((m) => {
    if (m.method === 'Runtime.consoleAPICalled') record.logs.push(`console.${m.params.type}: ` + m.params.args.map((a) => a.value ?? a.description ?? '').join(' '));
    if (m.method === 'Runtime.exceptionThrown') record.logs.push('EXCEPTION: ' + (m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text));
    if (m.method === 'Fetch.authRequired') c.send('Fetch.continueWithAuth', { requestId: m.params.requestId, authChallengeResponse: { response: 'ProvideCredentials', username: user, password: pass } });
    if (m.method === 'Fetch.requestPaused') c.send('Fetch.continueRequest', { requestId: m.params.requestId });
    if (m.method === 'Network.webSocketFrameSent' && /iq_start|iq_stop|saturn_display:/.test(m.params.response.payloadData || '') && !/ack/.test(m.params.response.payloadData)) {
      record.frames.push({ t: Date.now(), p: m.params.response.payloadData });
    }
  });
  await c.send('Runtime.enable'); await c.send('Page.enable'); await c.send('Network.enable');
  await c.send('Fetch.enable', { handleAuthRequests: true });
  await c.send('Page.navigate', { url });
  await sleep(4000);

  const page = () => c.evaluate(`(() => ({
    t: Date.now(), connected: state.connected, override: displayTransportOverride(), iqStreaming: state.iqStreaming,
    renderSource: state.displayRenderSource, echoMode: state.displayEchoMode,
    iq: state.iqFrameVersion, rxIq: state.rxIqFrameVersion, rows: state.displayServerRowsReceived,
    codec: rxAudioCodecSession.accepted, opusFrames: rxOpus.opusFrames, pcmFrames: rxOpus.pcmFrames, decodeErrors: rxOpus.decodeErrors, lateDrops: rxOpus.lateDrops,
    audioPlayed: state.audioFramesPlayed, lastAudioSeq: state.lastAudioSequence, audioGaps: state.audioSeqGapCount, audioResyncs: state.audioSeqResyncCount,
    rate: state.audioSampleRate, channels: state.audioChannels, worklet: state.audioWorkletMode, profile: audioProfileLabel(),
    queuedMs: state.rxWorkletQueuedMs, underruns: state.rxWorkletUnderruns, overflows: state.rxWorkletOverflows, drops: state.rxWorkletDrops,
    jitterP50: state.rxAudioJitterP50Ms, jitterP95: state.rxAudioJitterP95Ms, jitterP99: state.rxAudioJitterP99Ms,
  }))()`);
  // A failed read is recorded as an error, never dropped: the checker rejects it.
  const bridge = async () => {
    if (collector) return collector.read();   // never throws: a failure is an error record
    try {
      return mapMetrics(JSON.parse(fs.readFileSync(workDir + '/perf.json', 'utf8')));
    } catch (e) { return { error: String(e) }; }
  };

  await c.evaluate(`document.getElementById('go-live-btn').click()`);
  for (let i = 0; i < 40; i += 1) { await sleep(500); if ((await page()).connected) break; }
  record.atConnect = { page: await page(), bridge: await bridge() };
  await c.evaluate(`(() => { const b = Array.from(document.querySelectorAll('button')).find((x) => /Start Audio/.test(x.textContent)); if (b) b.click(); })()`);
  await sleep(6000); // warm-up on the normal subscription
  record.atWarm = { page: await page(), bridge: await bridge() };
  if (mode === 'c') {
    await c.evaluate(`(() => { globalThis.__saved = shouldAutoStartIq; shouldAutoStartIq = () => false; stopIq(); return true; })()`);
    record.stoppedAt = Date.now();
    await sleep(4000); // drain
  }
  record.start = { page: await page(), bridge: await bridge() };
  const end = Date.now() + Number(seconds) * 1000;
  while (Date.now() < end) { record.samples.push({ page: await page(), bridge: await bridge() }); await sleep(1000); }
  await c.shot(out + '.png');
  await c.send('Page.navigate', { url: 'about:blank' }); await sleep(800);
  save();
  console.log(`window ${out.split('/').pop()} done: ${record.samples.length} samples`);
} catch (error) {
  record.failure = { message: String((error && error.message) || error), stack: String((error && error.stack) || '') };
  save();
  console.error(`window ${out.split('/').pop()} FAILED: ${record.failure.message}`);
  process.exitCode = 1;
} finally {
  try { if (c) c.close(); } catch (_) { /* already closed */ }
  try { if (collector) await collector.close(); } catch (_) { /* the reader is already gone */ }
}
