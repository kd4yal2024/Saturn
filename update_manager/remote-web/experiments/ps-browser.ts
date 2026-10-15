import {recordedCapture, type CaptureAudit} from '../src/puresignal/browser-capture-review';
import {TxGenerationReviewClient} from '../src/transport/tx-generation-review';
import {buildTxMicPcmS16Frame} from '../src/transport/tx-uplink';
import {createTxOpusAudioDataFromFloat32, txOpusAudioEncoderConfig,
  txOpusProfileForCodec} from '../src/audio/tx-opus-encoder';

type Mode = 'pcm' | 'continuous' | 'banked';
const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
const base64 = (buffer: ArrayBuffer) => {
  let s = ''; for (const b of new Uint8Array(buffer)) s += String.fromCharCode(b);
  return btoa(s);
};
async function input(path: string) {
  const response = await fetch(path); if (!response.ok) throw new Error('input_missing');
  return new Float32Array(await response.arrayBuffer());
}
function auditResult(a: CaptureAudit | undefined) {
  if (!a) throw new Error('capture_not_started');
  return {sampleRate: a.sampleRate, inputSamples: a.inputSamples,
    tailSamples: a.tailSamples, blocks: a.blocks, workletToMainMs: a.workletToMainMs};
}

// Diagnostic reference retains ALL raw output (including flush priming/tail).
// It is not sent through PSG1 and cannot authorize TX or mask managed failure.
async function compare(mode: Mode, path: string) {
  const samples = await input(path);
  const packets: unknown[] = []; const errors: string[] = [];
  let a: CaptureAudit | undefined;
  let sequence = 0, bank = -1;
  const captured = new Float32Array(samples.length);
  let encoder: AudioEncoder | undefined;
  const pending = new Map<number, {oldest: number; submitted: number; bank: number}>();
  const flushes: Promise<void>[] = [];
  const config = txOpusAudioEncoderConfig(txOpusProfileForCodec('opus_wb'));
  const support = await AudioEncoder.isConfigSupported(config);
  function create(id: number) {
    const e = new AudioEncoder({error: err => errors.push(String(err)), output: chunk => {
      const payload = new Uint8Array(chunk.byteLength); chunk.copyTo(payload);
      const key = Math.round(chunk.timestamp / 20000);
      const p = pending.get(key);
      // Extra flush output is kept, never reassigned to sequence zero.
      if (p?.bank === id) pending.delete(key);
      packets.push({bank: id, timestampUs: chunk.timestamp, durationUs: chunk.duration,
        bytes: base64(payload.buffer), atMs: performance.now(),
        observationMs: p?.bank === id ? p.oldest : null,
        encodeMs: p?.bank === id ? performance.now() - p.submitted : null,
        submittedSequence: p?.bank === id ? key : null});
    }});
    e.configure(config); return e;
  }
  const capture = recordedCapture(samples, mode === 'pcm' ? 512 : 960, value => { a = value; });
  const source = await capture('00000000000000000000000000000001', (block, oldest) => {
    const seq = sequence++;
    captured.set(block, seq * block.length);
    if (mode === 'pcm') {
      packets.push({bank: 0, submittedSequence: seq, observationMs: oldest,
        atMs: performance.now(), bytes: base64(buildTxMicPcmS16Frame(block, seq).frame),
        durationUs: block.length / 48000 * 1e6, encodeMs: 0});
      return;
    }
    const id = mode === 'banked' ? Math.floor(seq / 4) : 0;
    if (bank !== id) { bank = id; encoder = create(id); }
    pending.set(seq, {oldest, submitted: performance.now(), bank: id});
    const data = createTxOpusAudioDataFromFloat32(block, 'opus_wb', {scope: window, sequence: seq});
    if (data.state !== 'ready') throw new Error(data.reason);
    encoder!.encode(data.audioData as AudioData); data.audioData.close?.();
    if (mode === 'banked' && seq % 4 === 3) {
      const complete = encoder!;
      flushes.push(complete.flush().then(() => complete.close())); encoder = undefined;
    }
  });
  await a!.done;
  if (encoder) { await encoder.flush(); encoder.close(); }
  await Promise.all(flushes); source.close();
  return {mode, support, config, capture: auditResult(a), packets, errors,
    captured: base64(captured.buffer), unmatchedInput: [...pending.keys()]};
}

// Same production reference, real AudioEncoder, but delivery can be held by
// the experiment AFTER native encoding. Authority still freezes before encode.
type CallbackTrace = {event: string; atMs: number; encoder: number};
function delayedScope(delayMs: number, trace: CallbackTrace[]) {
  let serial = 0;
  return {AudioData, AudioEncoder: class {
    private encoder: AudioEncoder;
    private deliveries: Promise<void>[] = [];
    private id = serial++;
    constructor(init: {output: (chunk: EncodedAudioChunk) => void; error: (e: unknown) => void}) {
      this.encoder = new AudioEncoder({error: init.error, output: chunk => {
        trace.push({event: 'encoded', atMs: performance.now(), encoder: this.id});
        this.deliveries.push(new Promise(resolve => setTimeout(() => {
          trace.push({event: 'callback', atMs: performance.now(), encoder: this.id});
          init.output(chunk); resolve();
        }, delayMs)));
      }});
    }
    configure(config: AudioEncoderConfig) { this.encoder.configure(config); }
    encode(data: AudioData) { this.encoder.encode(data); }
    async flush() { await this.encoder.flush(); await Promise.all(this.deliveries); }
    close() { this.encoder.close(); }
  }};
}
async function managed(url: string, path: string, codec: 'pcm' | 'opus_wb', delayMs = 0,
  reconnectOldFrame: string | null = null, disconnectWhileKeyed = false) {
  const destination = new URL(url);
  if (destination.protocol !== 'ws:' || destination.hostname !== '127.0.0.1') throw new Error('loopback_only');
  const samples = await input(path);
  const log: {atMs: number; text: string}[] = []; const sent: string[] = []; const trace: CallbackTrace[] = [];
  let a: CaptureAudit | undefined;
  let oldInjected = false, reconnectInjected = false;
  let original: ArrayBuffer | null = null;
  const control = new WebSocket(`${url}/control`), media = new WebSocket(`${url}/media`);
  const opened = (s: WebSocket) => new Promise<void>((resolve, reject) => {
    s.onopen = () => resolve(); s.onerror = () => reject(new Error('socket_open'));
  });
  let client: TxGenerationReviewClient;
  const capture = recordedCapture(samples, codec === 'pcm' ? 512 : 960, value => { a = value; });
  client = new TxGenerationReviewClient({enabled: true, codec,
    scope: delayMs ? delayedScope(delayMs, trace) : window, now: () => performance.now(), capture,
    media: frame => {
      if (original && !oldInjected && new Uint8Array(frame).slice(44, 60)
        .some((v, i) => v !== new Uint8Array(original!)[44 + i])) {
        media.send(original); oldInjected = true;
        log.push({atMs: performance.now(), text: 'experiment:old-generation-frame-delivered-after-rearm'});
      }
      if (reconnectOldFrame && !reconnectInjected) {
        media.send(Uint8Array.from(atob(reconnectOldFrame), c => c.charCodeAt(0)));
        reconnectInjected = true;
        log.push({atMs: performance.now(), text: 'experiment:old-session-frame-delivered-after-reconnect'});
      }
      original ??= frame.slice(0); sent.push(base64(frame)); media.send(frame);
    },
    control: text => { log.push({atMs: performance.now(), text: `client:${text}`}); control.send(text); },
  });
  control.onmessage = e => {
    if (typeof e.data !== 'string') return;
    log.push({atMs: performance.now(), text: e.data});
    if (e.data.startsWith('saturn_tx_generation:')) void client.generation(e.data);
  };
  media.onclose = control.onclose = () => client.disconnect();
  await Promise.all([opened(control), opened(media)]);
  const session = `browser-${crypto.randomUUID()}`;
  control.send(`session_open:${session},operator;session_lane:${session},control;tx_codec_caps:${codec === 'pcm' ? 'pcm' : 'pcm,opus_wb'};`);
  media.send(`session_open:${session},viewer;session_lane:${session},media;`);
  await wait(100);
  if (!log.some(e => e.text === 'saturn_tx_media_caps:psg1,pcm,opus_wb,bank4;')
    || !log.some(e => e.text === `tx_codec_accept:0,${codec};`)) {
    client.disconnect(); control.close(); media.close(); throw new Error('required_profile_not_negotiated');
  }
  control.send('tx_mic_gain:0,-3;'); control.send('tx_eq_enable:0,true;');
  const configuredDeadline = performance.now() + 3000;
  while (log.filter(e => /^tx_config:0,[0-9]+,applied,/.test(e.text)).length < 2) {
    if (performance.now() > configuredDeadline) {
      client.disconnect(); control.close(); media.close();
      throw new Error('settings_not_applied:' + JSON.stringify(log.filter(e => e.text.startsWith('tx_config:'))));
    }
    await wait(10);
  }
  control.send('trx:0,true;');
  await wait(delayMs ? 165 : codec === 'pcm' ? 1000 : 350);
  log.push({atMs: performance.now(), text: 'experiment:manual-release-1'});
  client.release();
  const releasedCount = sent.length;
  await wait(350);
  const delayedOutputAfterRelease = sent.length - releasedCount;
  if (delayMs && delayedOutputAfterRelease) throw new Error('delayed_callback_crossed_release');
  // Fresh authorized action; do not reuse an old boolean or token.
  control.send('trx:0,true;'); await wait(codec === 'pcm' ? 1000 : 350);
  if (disconnectWhileKeyed) {
    log.push({atMs: performance.now(), text: 'experiment:disconnect-while-keyed'});
    client.disconnect(); control.close(); media.close();
  } else {
    log.push({atMs: performance.now(), text: 'experiment:manual-release-2'});
    client.release();
  }
  await wait(delayMs + 350);
  const captureResult = a ? auditResult(a) : null;
  client.disconnect(); control.close(); media.close(); await wait(150);
  if (codec === 'pcm' && (sent.length < 50 || !oldInjected
    || (reconnectOldFrame && !reconnectInjected))) throw new Error('PCM_capture_or_old_generation_control_vacuous');
  if (delayMs) {
    const offers = log.filter(e => /^saturn_tx_generation:psg1,[0-9a-f]{32},/.test(e.text));
    const second = offers[1];
    if (!second || !trace.some(e => e.event === 'encoded' && e.encoder === 0)
      || !trace.some(e => e.event === 'callback' && e.encoder === 0 && e.atMs > second.atMs)
      || sent.length) throw new Error('old_callback_did_not_cross_rearm_or_output_leaked');
  }
  if (codec === 'opus_wb' && !delayMs) {
    const manual = log.find(e => e.text === 'experiment:manual-release-1')!;
    if (sent.length !== 8 || !oldInjected || (reconnectOldFrame && !reconnectInjected)
      || !log.some(e => e.text === 'client:trx:0,false;' && e.atMs < manual.atMs)) {
      throw new Error('expected_real_four_input_flush_contract_failure');
    }
  }
  return {codec, delayMs, log, sent, trace, capture: captureResult,
    oldInjected, reconnectInjected, disconnectWhileKeyed, delayedOutputAfterRelease};
}

Object.assign(window, {psExperiment: {compare, managed}});
