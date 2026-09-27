import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';
import {
  RX_OPUS_FRAME_DURATION_US,
  copyRxOpusAudio,
  createRxAudioCodecSession,
  createRxOpusPendingQueue,
  parseRxOpusPacket,
  probeRxOpusDecoder,
  rxOpusBacklogAction,
  rxOpusMalformedAction,
  rxOpusStalled,
} from '../src/audio/rx-opus';
import { audioFramesToMilliseconds, rxAudioArrivalJitterMs } from '../src/audio/rx-telemetry';
import { parseTciText } from '../src/tci/parser';
import { decodeAudioFrame } from '../src/transport/rx-frame';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

function slice(start: string, end: string): string {
  const from = template.indexOf(start);
  const to = template.indexOf(end, from);
  if (from < 0 || to < 0) throw new Error(`template slice not found: ${start}`);
  return template.slice(from, to);
}

const CODEC_REGION = slice(
  '    function closeRxOpusDecoder() {',
  '    function updateRxAdaptiveRateTelemetry(telemetry) {',
);
const INGEST_REGION = slice(
  '    function recordAudioSequence(frame) {',
  '    function handleBinaryFrame(buffer) {',
);

type DecoderBehaviour = 'ok' | 'silent' | 'silent-stereo-mismatch' | 'throw' | 'short';

// Chrome's Opus decoder was observed renumbering output timestamps for a subset
// of packets; this fake models that so the regression is covered.
class FakeEncodedAudioChunk {
  readonly type: string;
  readonly timestamp: number;
  readonly duration: number;
  readonly data: Uint8Array;
  constructor(init: { type?: string; timestamp: number; duration?: number; data: Uint8Array }) {
    this.type = init.type ?? 'key';
    this.timestamp = init.timestamp;
    this.duration = init.duration ?? 0;
    this.data = init.data;
  }
}

// Minimal WebCodecs stand-in. `decode()` invokes the decoder output callback
// synchronously so assertions stay deterministic.
class FakeAudioDecoder {
  static behaviour: DecoderBehaviour | 'rewrite-stamps' = 'ok';
  static instances: FakeAudioDecoder[] = [];
  static async isConfigSupported() { return { supported: true }; }
  static reset(behaviour: DecoderBehaviour | 'rewrite-stamps' = 'ok') {
    FakeAudioDecoder.behaviour = behaviour;
    FakeAudioDecoder.instances = [];
  }
  static rewriteCount = 0;
  readonly init: { output: (data: unknown) => void; error: (error: unknown) => void };
  config: { codec?: string; sampleRate?: number; numberOfChannels?: number } | null = null;
  closed = false;
  decodeCalls = 0;
  constructor(init: FakeAudioDecoder['init']) {
    this.init = init;
    FakeAudioDecoder.instances.push(this);
  }
  configure(config: { codec?: string; sampleRate?: number; numberOfChannels?: number }) {
    this.config = config;
  }
  close() { this.closed = true; }
  decode(chunk: { timestamp: number }) {
    this.decodeCalls += 1;
    const behaviour = FakeAudioDecoder.behaviour;
    if (behaviour === 'silent' || behaviour === 'silent-stereo-mismatch') return;
    if (behaviour === 'throw') throw new Error('decode boom');
    const channels = this.config?.numberOfChannels ?? 1;
    const frames = behaviour === 'short' ? 480 : 960;
    // Model a decoder that renumbers output timestamps for part of the stream:
    // the live LAN trace showed 42 of 1046 outputs landing on a timestamp no
    // input ever used, which used to leak pending entries and drop audio.
    let outputTimestamp = chunk.timestamp;
    if (behaviour === 'rewrite-stamps') {
      FakeAudioDecoder.rewriteCount += 1;
      outputTimestamp = chunk.timestamp + (FakeAudioDecoder.rewriteCount % 3 === 0 ? 1000 : 0);
    }
    this.init.output({
      timestamp: outputTimestamp,
      sampleRate: 48_000,
      numberOfChannels: channels,
      numberOfFrames: frames,
      copyTo(destination: Float32Array, options: { planeIndex: number }) {
        destination.fill(options.planeIndex === 0 ? 0.25 : -0.25);
      },
      close() {},
    });
  }
}

function opusFrame(payloadLength = 60, channels = 1, sequence = 1): ArrayBuffer {
  const buffer = new ArrayBuffer(64 + payloadLength);
  const view = new DataView(buffer);
  view.setUint32(4, 48_000, true);
  view.setUint32(8, 20, true);
  view.setUint32(12, 0, true);
  view.setUint32(16, 960, true);
  view.setUint32(20, payloadLength, true);
  view.setUint32(24, 17, true);
  view.setUint32(28, channels, true);
  view.setUint32(32, sequence, true);
  new Uint8Array(buffer, 64).fill(0x5a);
  return buffer;
}

function makeHarness(options: { preference?: string } = {}) {
  const sent: string[] = [];
  const logs: string[] = [];
  const faults: string[] = [];
  const played: number[] = [];
  const flushes: string[] = [];
  const state: Record<string, unknown> = {
    connected: true,
    ws: null,
    audioCtx: { state: 'running', sampleRate: 48_000, currentTime: 0, resume() {} },
    audioStreaming: true,
    audioWorkletMode: 'msg',
    rxWorkletNode: { port: { postMessage: (message: { type: string; left: Float32Array }) => {
      if (message.type === 'audio') played.push(message.left.length);
    } } },
    audioNextTime: 0,
    lastAudioArrivalAt: 0,
    lastAudioFrameAt: 0,
    lastAudioSequence: null,
    lastAudioPacketDurationMs: 0,
    audioSeqGapCount: 0,
    audioSeqResyncCount: 0,
    audioSeqMissingPacketCount: 0,
    lastAudioSeqGapReportAt: 0,
    audioSampleRate: 48_000,
    audioChannels: 1,
    audioFramesPlayed: 0,
    rxAudioJitterSamples: [] as number[],
    rxAudioJitterP99Ms: 0,
    rxWorkletUnderruns: 0,
    rxWorkletOverflows: 0,
    rxAdaptiveRateRatio: 1,
  };
  const rxOpus: Record<string, unknown> = {
    capability: 'probing', gainEcho: false, probeSupported: false, fallbackReason: '',
    decoder: null, decoderChannels: 0, decoderFramesSubmitted: 0, decoderFramesOutput: 0,
    epoch: 0, socketEpoch: 0,
    pending: createRxOpusPendingQueue(),
    pcmBytes: 0, opusBytes: 0, pcmWindowBytes: 0, opusWindowBytes: 0,
    pcmBytesPerSec: 0, opusBytesPerSec: 0, decodedFrames: 0,
    pcmFrames: 0, opusFrames: 0,
    malformedFrames: 0, malformedRun: 0, decodeErrors: 0, lateDrops: 0, resyncs: 0,
  };
  const rxAudioCodecSession = createRxAudioCodecSession();
  // Packet time is driven by the test so age-based pruning is deterministic.
  const clock = { value: 1000 };
  const sandbox: Record<string, unknown> = {
    _next: {
      parseRxOpusPacket,
      copyRxOpusAudio,
      rxOpusBacklogAction,
      rxOpusMalformedAction,
      createRxOpusPendingQueue,
      rxOpusStalled,
      decodeAudioFrame,
      parseTciText,
      probeRxOpusDecoder,
      audioFramesToMilliseconds,
      rxAudioArrivalJitterMs,
      RX_OPUS_FRAME_DURATION_US,
    },
    state,
    rxOpus,
    rxAudioCodecSession,
    rxAudioCodecPreference: options.preference ?? 'auto',
    sendTci: (text: string) => { sent.push(text); },
    logEvent: (message: string) => { logs.push(message); },
    recordOperatorFault: (label: string) => { faults.push(label); },
    flushRxAudioQueue: (reason: string) => { flushes.push(reason); },
    resetRxLatencyTelemetry: () => {},
    scheduleUiRefresh: () => {},
    txReceiveSuppressionActive: () => false,
    observeRxPlaybackQueue: () => {},
    captureRxAudioScope: () => {},
    writeRxAudioFrameToSab: () => true,
    scheduleAudioPlayback: () => { played.push(960); },
    refreshRxAudioJitterSummary: () => {},
    audioLeadMs: () => 20,
    rxAudioRecorder: { packetId: 0 },
    document: { visibilityState: 'visible' },
    rxAdaptiveRateController: {
      prepare: (left: Float32Array, right: Float32Array, frames: number, sampleRate: number) => ({
        left, right, frames, sampleRate,
      }),
    },
    performance: { now: () => clock.value },
    AudioDecoder: FakeAudioDecoder,
    EncodedAudioChunk: FakeEncodedAudioChunk,
    ArrayBuffer,
    DataView,
    Uint8Array,
    Float32Array,
    Map,
  };
  const api = runInNewContext(
    `${CODEC_REGION}\n${INGEST_REGION}\n` +
      '({ requestRxAudioCodec, handleRxAudioCodecEcho, rxAudioCodecSnapshot, ' +
      'handleOpusAudioFrame, closeRxOpusDecoder, resetRxAudioCodecTransport })',
    sandbox,
  ) as {
    requestRxAudioCodec: (ws: unknown) => Promise<void>;
    handleRxAudioCodecEcho: (text: string) => void;
    rxAudioCodecSnapshot: () => Record<string, unknown>;
    handleOpusAudioFrame: (buffer: ArrayBuffer) => void;
    closeRxOpusDecoder: () => void;
    resetRxAudioCodecTransport: () => void;
  };
  return { api, sandbox, sent, logs, faults, played, flushes, state, rxOpus, rxAudioCodecSession, clock };
}

async function connectWithOpus(harness: ReturnType<typeof makeHarness>) {
  FakeAudioDecoder.reset('ok');
  const ws = { id: 'socket-1' };
  harness.state.ws = ws;
  await harness.api.requestRxAudioCodec(ws);
  harness.api.handleRxAudioCodecEcho('audio_gain:client;');
  harness.api.handleRxAudioCodecEcho('audio_codec:opus;audio_samplerate:48000;');
}

// Feeds frames at a realistic 20 ms packet cadence so age-based pruning sees the
// same clock progression it would on a live link.
function pushFrames(
  harness: ReturnType<typeof makeHarness>,
  count: number,
  options: { channels?: number; payloadBytes?: number; startSequence?: number } = {},
) {
  const start = options.startSequence ?? 1;
  for (let index = 0; index < count; index += 1) {
    harness.clock.value += 20;
    harness.api.handleOpusAudioFrame(
      opusFrame(options.payloadBytes ?? 60, options.channels ?? 1, start + index),
    );
  }
}

describe('RX Opus negotiation in the template', () => {
  it('requests client-side gain first and offers Opus only after that echo', async () => {
    const h = makeHarness();
    const ws = { id: 'socket-1' };
    h.state.ws = ws;
    await h.api.requestRxAudioCodec(ws);
    expect(h.sent).toEqual(['audio_gain:client;']);
    // Opus is only offered after the bridge confirms client-side gain, so the
    // capability probe result alone must not change the request yet.
    expect(h.rxOpus.capability).toBe('supported');
    expect(h.rxAudioCodecSession.requested).toBe('pcm');

    h.api.handleRxAudioCodecEcho('remote_backpressure:0,1,2;');
    expect(h.sent).toEqual(['audio_gain:client;']);

    h.api.handleRxAudioCodecEcho('audio_gain:client;');
    expect(h.sent).toEqual(['audio_gain:client;', 'audio_codec:opus;']);
    expect(h.rxAudioCodecSession.requested).toBe('opus');
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');

    h.api.handleRxAudioCodecEcho('audio_codec:opus;audio_samplerate:48000;');
    expect(h.rxAudioCodecSession.accepted).toBe('opus');
    expect(h.sent.filter((text) => text === 'audio_codec:opus;').length).toBe(1);
  });

  it('forces PCM without probing when ?rx_audio_codec=pcm is set', async () => {
    const h = makeHarness({ preference: 'pcm' });
    const ws = { id: 'socket-1' };
    h.state.ws = ws;
    await h.api.requestRxAudioCodec(ws);
    expect(h.sent).toEqual(['audio_gain:client;', 'audio_codec:pcm;']);
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');
    expect(h.rxOpus.capability).toBe('forced PCM');
  });

  it('stays on PCM when WebCodecs cannot decode Opus', async () => {
    const h = makeHarness();
    FakeAudioDecoder.reset('ok');
    h.sandbox.AudioDecoder = undefined;
    const ws = { id: 'socket-1' };
    h.state.ws = ws;
    await h.api.requestRxAudioCodec(ws);
    expect(h.sent).toEqual(['audio_gain:client;', 'audio_codec:pcm;']);
    expect(h.rxOpus.capability).toBe('WebCodecs unavailable');
    expect(h.rxOpus.fallbackReason).toBe('WebCodecs Opus decoder unavailable');
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');
  });

  it('latches PCM and raises an operator fault when the bridge refuses Opus', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    expect(h.rxAudioCodecSession.accepted).toBe('opus');
    h.api.handleRxAudioCodecEcho('audio_codec:pcm;');
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');
    expect(h.rxAudioCodecSession.failed).toBe(true);
    expect(h.rxOpus.fallbackReason).toBe('bridge selected PCM');
    expect(h.rxOpus.capability).toBe('bridge refused');
    expect(h.faults).toEqual(['RX Opus refused by bridge']);
    expect(h.flushes).toEqual(['audio-codec-opus', 'audio-codec-pcm']);
    expect(h.rxOpus.decoder).toBeNull();
  });
});

describe('RX Opus ingest resilience in the template', () => {
  it('decodes type 17 packets into the shared playback sink', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    pushFrames(h, 3);
    expect(h.rxOpus.decodedFrames).toBe(3);
    expect(h.rxOpus.malformedFrames).toBe(0);
    expect(h.played).toEqual([960, 960, 960]);
    expect(h.state.audioChannels).toBe(1);
    expect(h.api.rxAudioCodecSnapshot().accepted).toBe('opus');
  });

  it('drops a single corrupt packet instead of abandoning Opus', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    pushFrames(h, 1);
    const corrupt = opusFrame(60, 1, 2);
    new DataView(corrupt).setUint32(20, 999, true);
    h.api.handleOpusAudioFrame(corrupt);
    pushFrames(h, 1, { startSequence: 3 });

    expect(h.rxOpus.malformedFrames).toBe(1);
    expect(h.rxOpus.malformedRun).toBe(0);
    expect(h.rxOpus.resyncs).toBe(0);
    expect(h.sent).not.toContain('audio_codec:pcm;');
    expect(h.rxAudioCodecSession.accepted).toBe('opus');
    expect(h.rxOpus.decodedFrames).toBe(2);
  });

  it('resets the malformed run once a good packet arrives', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    for (let i = 0; i < 3; i += 1) {
      const corrupt = opusFrame(60, 1, i + 1);
      new DataView(corrupt).setUint32(24, 99, true);
      h.api.handleOpusAudioFrame(corrupt);
    }
    expect(h.rxOpus.malformedRun).toBe(3);
    h.api.handleOpusAudioFrame(opusFrame(60, 1, 10));
    expect(h.rxOpus.malformedRun).toBe(0);
    for (let i = 0; i < 4; i += 1) {
      const corrupt = opusFrame(60, 1, 11 + i);
      new DataView(corrupt).setUint32(24, 99, true);
      h.api.handleOpusAudioFrame(corrupt);
    }
    expect(h.rxOpus.malformedRun).toBe(4);
    expect(h.sent).not.toContain('audio_codec:pcm;');
    expect(h.rxAudioCodecSession.accepted).toBe('opus');
  });

  it('falls back to PCM once a sustained run of malformed frames proves a contract mismatch', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    for (let sequence = 1; sequence <= 5; sequence += 1) {
      const corrupt = opusFrame(60, 1, sequence);
      new DataView(corrupt).setUint32(8, 40, true);
      h.api.handleOpusAudioFrame(corrupt);
    }
    expect(h.rxOpus.malformedFrames).toBe(5);
    expect(h.sent).toContain('audio_codec:pcm;');
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');
    expect(h.rxOpus.fallbackReason).toBe('malformed Opus frame x5');
    expect(h.faults).toEqual(['RX Opus fallback']);
  });

  it('pairs decoder output in order when the decoder renumbers timestamps', async () => {
    // Regression for the live failure: Chrome's Opus decoder returned 1045
    // outputs for 1046 decodes with 42 input timestamps never appearing on any
    // output. Timestamp-keyed matching leaked pending entries until the backlog
    // guard fired, dropping ~660 ms of audio on a timer.
    const h = makeHarness();
    await connectWithOpus(h);
    FakeAudioDecoder.reset('rewrite-stamps');
    pushFrames(h, 200);
    const snapshot = h.api.rxAudioCodecSnapshot();
    expect(snapshot.timestampRewrites as number).toBeGreaterThan(0);
    expect(h.rxOpus.decodedFrames).toBe(200);
    expect(h.rxOpus.resyncs).toBe(0);
    expect(snapshot.pendingPackets).toBe(0);
    expect(h.sent).not.toContain('audio_codec:pcm;');
    expect(h.rxAudioCodecSession.accepted).toBe('opus');
  });

  it('resyncs once when the safety limit is reached with no packet ageing', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    FakeAudioDecoder.reset('silent');
    // Frozen clock: nothing ages out, so the safety limit is what protects us.
    for (let sequence = 1; sequence <= 70; sequence += 1) {
      h.api.handleOpusAudioFrame(opusFrame(60, 1, sequence));
    }
    expect(h.rxOpus.resyncs).toBe(1);
    expect(h.rxOpus.decodedFrames).toBe(0);
    expect((h.rxOpus.pending as { size: number }).size).toBeLessThanOrEqual(64);
    expect(h.rxAudioCodecSession.accepted).toBe('opus');
  });

  it('falls back to PCM when the decoder never produces audio', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    FakeAudioDecoder.reset('silent');
    pushFrames(h, 120);
    expect(h.rxOpus.decodedFrames).toBe(0);
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');
    expect(String(h.rxOpus.fallbackReason)).toMatch(/decoder produced no audio/);
    expect(h.sent).toContain('audio_codec:pcm;');
    expect(h.faults).toEqual(['RX Opus fallback']);
  });

  it('falls back if a decoder stalls after an earlier decoder produced audio', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    pushFrames(h, 3);
    expect(h.rxOpus.decodedFrames).toBe(3);
    h.api.closeRxOpusDecoder();
    FakeAudioDecoder.reset('silent');
    pushFrames(h, 120, { startSequence: 4 });
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');
    expect(String(h.rxOpus.fallbackReason)).toMatch(/decoder produced no audio/);
    expect(h.sent).toContain('audio_codec:pcm;');
  });

  it('falls back when decoded output does not match the packet contract', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    FakeAudioDecoder.behaviour = 'short';
    h.api.handleOpusAudioFrame(opusFrame(60, 2, 1));
    expect(h.rxOpus.decodedFrames).toBe(0);
    expect(h.sent).toContain('audio_codec:pcm;');
    expect(h.rxOpus.fallbackReason).toBe('decoder output format mismatch');
    expect(h.rxAudioCodecSession.accepted).toBe('pcm');
  });

  it('exposes the A/B readout through rxAudioCodecSnapshot', async () => {
    const h = makeHarness();
    await connectWithOpus(h);
    h.api.handleOpusAudioFrame(opusFrame(120, 2, 1));
    h.rxOpus.opusBytesPerSec = 9_600;
    const snapshot = h.api.rxAudioCodecSnapshot();
    expect(snapshot).toMatchObject({
      preference: 'auto',
      requested: 'opus',
      accepted: 'opus',
      clientGainEcho: true,
      decoderChannels: 2,
      decodedFrames: 1,
      malformedFrames: 0,
      resyncs: 0,
      orphanOutputs: 0,
      sequenceGaps: 0,
      workletUnderruns: 0,
    });
    expect(snapshot.opusBytes).toBe(184);
    expect(snapshot.pendingPackets).toBe(0);
  });
});
