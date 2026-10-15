// Node 22+, no dependencies. Default is description-only: no socket or RF.
// Explicit run is a fresh operator action for a host-enabled MeasurementOnly
// candidate on loopback (normally through an SSH tunnel). Never ordinary PTT.
import { pathToFileURL } from 'node:url';

export function pcmFrame(token, sequence) {
  if (!/^[0-9a-f]{32}$/.test(token) || !Number.isSafeInteger(sequence) || sequence < 0 || sequence > 0xffffffff)
    throw new Error('Invalid generation or sequence');
  const bytes = Buffer.alloc(64 + 512 * 4);
  for (const [offset, value] of [[4, 48000], [8, 3], [20, 512], [24, 2], [28, 1], [32, sequence], [40, 2048], [60, 0x50534731]])
    bytes.writeUInt32LE(value, offset);
  Buffer.from(token, 'hex').copy(bytes, 44);
  for (let i = 0; i < 512; i++) {
    const n = sequence * 512 + i;
    bytes.writeFloatLE(0.2 * (Math.sin(2 * Math.PI * 700 * n / 48000) + Math.sin(2 * Math.PI * 1900 * n / 48000)), 64 + i * 4);
  }
  return bytes;
}
export function validateRun(options) {
  const url = new URL(options.url);
  if (url.protocol !== 'ws:' || url.hostname !== '127.0.0.1' || url.pathname !== '/tci' || url.search || url.username || url.password)
    throw new Error('Only explicit ws://127.0.0.1:PORT/tci is allowed; use an SSH tunnel');
  if (!/^[0-9a-f]{64}$/.test(options.sourcePin)) throw new Error('Exact reviewed source pin required');
  if (!Number.isSafeInteger(options.durationMs) || options.durationMs < 1 || options.durationMs > 1000)
    throw new Error('Duration must be 1..1000 ms');
  if (!Number.isSafeInteger(options.frequencyHz) || options.frequencyHz < 1500000 || options.frequencyHz > 35000000)
    throw new Error('Sampler-qualified HF frequency required; no 6 m');
  if (!options.confirm) throw new Error('Explicit --confirm-ant1-dummy-load-lp700-attended required');
  return url;
}
export async function run(options) {
  const url = validateRun(options);
  const emit = options.emit ?? (record => process.stdout.write(`${JSON.stringify(record)}\n`));
  const ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';
  let role, ready = false, diagnostic = false, ownerClean = false, rf = false, codec = false;
  let configured = false, configSent = false, started = false, complete = false, captureId;
  let token, sequence = 0, pumpTimer, stopTimer, nextAudio = 0, fault = false, finished = false;
  let diagnosticFrequency, pin;
  const send = text => { if (ws.readyState === WebSocket.OPEN) ws.send(text); };
  const stopAudio = () => { token = undefined; clearTimeout(pumpTimer); };
  const requestStop = reason => { stopAudio(); fault = true; emit({ event: 'stop-requested', reason }); send('saturn_ps_measurement:0,stop;'); };
  const pump = () => {
    if (!token || complete || finished || ws.readyState !== WebSocket.OPEN) return;
    const now = performance.now();
    if (now - nextAudio > 50 || ws.bufferedAmount > 8192) { requestStop('PCM scheduling/queue deadline'); return; }
    ws.send(pcmFrame(token, sequence++));
    nextAudio += 512 / 48;
    pumpTimer = setTimeout(pump, Math.max(1, nextAudio - performance.now()));
  };
  const maybeStart = () => {
    if (started || !ready || role !== 'operator' || !diagnostic || !ownerClean || !codec || !rf) return;
    if (pin !== options.sourcePin || diagnosticFrequency !== options.frequencyHz) { requestStop('candidate identity/frequency mismatch'); return; }
    if (!configured) {
      if (!configSent) { configSent = true; send('drive:0,1;'); }
      return;
    }
    started = true;
    emit({ event: 'fresh-measurement-request', durationMs: options.durationMs, commandedWatts: 1, frequencyHz: options.frequencyHz,
      stimulus: '700/1900-Hz-tagged-PCM', stimulusPeakCeiling: 0.4, externalCouplingDb: null, rfImdQualified: false });
    send(`saturn_ps_measurement:0,start,${options.durationMs};`);
    stopTimer = setTimeout(() => { if (!complete) requestStop('local duration bound'); }, options.durationMs + 200);
  };
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      requestStop('transaction/cleanup deadline');
      // A socket close invokes the Bridge's independent cancellation path.
      // It is NOT evidence of completed backend stop or physical RF-off.
      finish(new Error('Cleanup/export not verified; inspect candidate before restarting anything'));
    }, 15000);
    const interrupt = () => requestStop('operator interrupt');
    process.once('SIGINT', interrupt); process.once('SIGTERM', interrupt);
    function finish(error, result) {
      if (finished) return;
      finished = true; stopAudio(); clearTimeout(timeout); clearTimeout(stopTimer);
      process.off('SIGINT', interrupt); process.off('SIGTERM', interrupt);
      ws.close();
      error ? reject(error) : resolve(result);
    }
    ws.addEventListener('open', () => send('tx_codec_caps:pcm;'));
    ws.addEventListener('error', () => finish(new Error('Diagnostic socket failed; cleanup unverified')));
    ws.addEventListener('close', () => { if (!finished) finish(new Error('Connection lost; cleanup unverified')); });
    ws.addEventListener('message', event => {
      if (typeof event.data !== 'string' || finished) return;
      for (const message of event.data.split(';').filter(Boolean)) {
        const colon = message.indexOf(':');
        const name = colon < 0 ? message : message.slice(0, colon);
        const args = colon < 0 ? [] : message.slice(colon + 1).split(',');
        if (name === 'remote_client_role') {
          role = args[1];
          if (role !== 'operator') { finish(new Error('Not the operator; disconnect the existing operator client first')); return; }
        } else if (name === 'ready') ready = true;
        else if (name === 'tx_enable') rf = args[1] === 'true';
        else if (name === 'tx_codec_accept') codec = args[1] === 'pcm';
        else if (name === 'saturn_ps_measurement_caps') {
          diagnostic = args[1] === 'true' && args[2] === '1000' && args[3] === 'rx-bypass';
          pin = args[4]; diagnosticFrequency = Number(args[6]);
          if (!diagnostic) { finish(new Error('Bridge is not in explicit MeasurementOnly mode')); return; }
        } else if (name === 'saturn_tx_owner') {
          ownerClean = args[0] === 'shared' && args[2] === 'ReadyOff' && args[6] === 'true';
          if (args[2] === 'Blocked') { requestStop('owner Blocked'); finish(new Error('Owner Blocked; no cleanup claim')); return; }
        } else if (name === 'tx_config' && configSent && !started) {
          if (args[2] === 'applied') configured = true;
          if (args[2] === 'rejected') { finish(new Error(`Drive settings rejected: ${args[3]}`)); return; }
        } else if (name === 'saturn_tx_generation') {
          stopAudio();
          if (args[0] === 'psg1' && /^[0-9a-f]{32}$/.test(args[1]) && started && !complete && !fault) {
            token = args[1]; sequence = 0; nextAudio = performance.now(); pump();
          }
        } else if (name === 'saturn_ps_measurement') {
          emit({ event: 'bridge', message });
          if (args[2] === 'queued') captureId = args[1];
          if (args[2] === 'rejected' || args[2] === 'export-rejected') { requestStop(message); finish(new Error(message)); return; }
          if (args[2] === 'complete' && args[1] === captureId) {
            complete = true; stopAudio(); clearTimeout(stopTimer);
            fault ||= args[4] !== 'none' || Number(args[3]) === 0;
            send(`saturn_ps_measurement:0,export,${captureId};`);
          }
          if (args[2] === 'exported' && args[1] === captureId) {
            const result = { event: 'exported', pathOnBridge: args[3], framesSent: sequence, fault, rfImdQualified: false };
            emit(result); finish(fault ? new Error('Evidence exported with a diagnostic fault; no automatic retry') : null, result);
          }
        } else if (name === 'tx_fault' && started) {
          // A final old-generation frame can be rejected after cancellation.
          // The lifecycle completion owns the actual reason for stopping.
          if (message.endsWith(',GenerationMismatch')) stopAudio();
          else requestStop(message);
        }
        if (ready && !diagnostic) { finish(new Error('No diagnostic capability; refusing legacy TX')); return; }
        if (ready && diagnostic && role === 'operator' && codec && !rf) {
          finish(new Error('RF permission inhibited; no measurement requested')); return;
        }
        if (ready && diagnostic && (pin !== options.sourcePin || diagnosticFrequency !== options.frequencyHz)) {
          finish(new Error('Candidate identity/frequency mismatch; no measurement requested')); return;
        }
        maybeStart();
      }
    });
  });
}
async function main() {
  const args = process.argv.slice(2);
  if (!args.includes('--run')) {
    process.stdout.write(`${JSON.stringify({ mode: 'description-only', socketOpened: false, rfRequested: false,
      runRequires: ['--run', '--url=ws://127.0.0.1:PORT/tci', '--source-pin=64-lowercase-hex', '--confirm-ant1-dummy-load-lp700-attended'],
      durationMs: 500, commandedWatts: 1, frequencyHz: 7200000, maxDurationMs: 1000, rfImdQualified: false })}\n`);
    return;
  }
  const value = key => args.find(s => s.startsWith(`${key}=`))?.slice(key.length + 1);
  const known = ['--run', '--confirm-ant1-dummy-load-lp700-attended'];
  if (args.some(a => !known.includes(a) && !['--url=', '--source-pin=', '--duration-ms=', '--frequency-hz='].some(k => a.startsWith(k))))
    throw new Error('Unknown argument');
  await run({ url: value('--url'), sourcePin: value('--source-pin'), durationMs: Number(value('--duration-ms') ?? 500),
    frequencyHz: Number(value('--frequency-hz') ?? 7200000), confirm: args.includes('--confirm-ant1-dummy-load-lp700-attended') });
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main().catch(error => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
