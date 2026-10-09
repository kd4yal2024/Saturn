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
// The record is written on success AND on failure. If anything throws, what was recorded so far is kept, a
// `failure` field is added, and the process exits nonzero. The checker rejects a record that carries a failure.
import { connect, sleep } from './cdp.mjs';
import fs from 'node:fs';

const [,, port, login, url, workDir, seconds, out, mode, metaJson] = process.argv;
const record = {
  url, mode, meta: JSON.parse(metaJson || '{}'), stoppedAt: null,
  atConnect: null, atWarm: null, start: null, samples: [], frames: [], logs: [],
};
const save = () => fs.writeFileSync(out + '.json', JSON.stringify(record, null, 1));
let c = null;

try {
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
  const bridge = () => {
    try {
      const j = JSON.parse(fs.readFileSync(workDir + '/perf.json', 'utf8')); const m = j.metrics;
      return {
        updatedAtMs: j.updated_at_ms, iq: m.iq, audio: m.audio, connections: m.connections, iq_tci_frames_s: m.iq_tci_frames_s,
        rx_audio_frames_s: m.rx_audio_frames_s, rows_written: m.display_spectrum_rows_written, spectrum_clients: m.display_spectrum_clients,
        audio_dropped_s: m.audio_dropped_s, tcp_outq_hwm_bytes: m.tcp_outq_hwm_bytes, out_hwm_bytes: m.out_hwm_bytes, outbound_drops: m.outbound_drops,
      };
    } catch (e) { return { error: String(e) }; }
  };

  await c.evaluate(`document.getElementById('go-live-btn').click()`);
  for (let i = 0; i < 40; i += 1) { await sleep(500); if ((await page()).connected) break; }
  record.atConnect = { page: await page(), bridge: bridge() };
  await c.evaluate(`(() => { const b = Array.from(document.querySelectorAll('button')).find((x) => /Start Audio/.test(x.textContent)); if (b) b.click(); })()`);
  await sleep(6000); // warm-up on the normal subscription
  record.atWarm = { page: await page(), bridge: bridge() };
  if (mode === 'c') {
    await c.evaluate(`(() => { globalThis.__saved = shouldAutoStartIq; shouldAutoStartIq = () => false; stopIq(); return true; })()`);
    record.stoppedAt = Date.now();
    await sleep(4000); // drain
  }
  record.start = { page: await page(), bridge: bridge() };
  const end = Date.now() + Number(seconds) * 1000;
  while (Date.now() < end) { record.samples.push({ page: await page(), bridge: bridge() }); await sleep(1000); }
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
}
