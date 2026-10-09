import { parse } from 'acorn';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { afterEach, describe, expect, it, vi } from 'vitest';
import * as spectrumRow from '../src/transport/spectrum-row';
import { displaySpanHz, shiftBinsHorizontally, visibleBinsForDisplay } from '../src/dsp/display';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

const TRANSPORT_START = '    // ---- WAN display transport: server spectrum rows (stream type 16) ----';
const TRANSPORT_END = '    // ---- end WAN display transport ----';
const BINARY_START = '    function handleBinaryFrame(buffer) {';
const BINARY_END = '    function adoptWsDiagSocket(socket) {';
const MEASURE_START = '    let rxSpectrumAccumulator = null;';
const MEASURE_END = '    function sampleRxSpectrum(bins, now) {';
const IQ_HANDLER_START = '    let lastIqUiRefreshAt = -Infinity;';
const IQ_HANDLER_END = '    function renderIdleFrame(now) {';
const DRAW_START = '    function drawDisplayBins(bins, now, waterfallEnabled, phoneWanLite, centerShiftHz = 0) {';
const DRAW_END = '    function animationLoop(now) {';

function slice(start: string, end: string): string {
  const from = template.indexOf(start);
  const to = template.indexOf(end, from);
  if (from < 0 || to < 0) throw new Error(`template slice not found: ${start}`);
  return template.slice(from, to);
}

/**
 * The page's main script, parsed whole. The tests above load *slices* of it, which
 * cannot see that another declaration of the same name elsewhere in the page would
 * win (a later function declaration replaces an earlier one). These helpers look at
 * the complete script.
 */
const pageScript = [...template.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
  .map((match) => match[1] ?? '')
  .find((script) => script.includes('function startRxSpectrumCapture'))!;
type FunctionNode = { type: string; id?: { name: string }; start: number; end: number };
const pageFunctions = (parse(pageScript, { ecmaVersion: 'latest' }) as unknown as { body: FunctionNode[] }).body
  .filter((node) => node.type === 'FunctionDeclaration' && node.id)
  .map((node) => ({ name: node.id!.name, text: pageScript.slice(node.start, node.end) }));

/** Names of the functions a block of loaded template code declares. */
function declaredFunctionNames(code: string): Set<string> {
  const body = (parse(code, { ecmaVersion: 'latest' }) as unknown as { body: FunctionNode[] }).body;
  return new Set(body.filter((node) => node.type === 'FunctionDeclaration' && node.id).map((node) => node.id!.name));
}

type RowOptions = {
  binCount?: number;
  centerHz?: number;
  sequence?: number;
  streamType?: number;
  format?: number;
};

function buildRow(options: RowOptions = {}): ArrayBuffer {
  const binCount = options.binCount ?? 256;
  const centerHz = options.centerHz ?? 14_200_000;
  const buffer = new ArrayBuffer(64 + binCount);
  const view = new DataView(buffer);
  view.setUint32(4, 384_000, true);
  view.setUint32(8, options.format ?? 0x5301, true);
  view.setUint32(12, binCount, true);
  view.setUint32(16, 1, true);
  view.setUint32(20, binCount, true);
  view.setUint32(24, options.streamType ?? 16, true);
  view.setUint32(28, 1, true);
  view.setUint32(32, options.sequence ?? 1, true);
  view.setUint32(36, centerHz >>> 0, true);
  view.setFloat32(44, -160, true);
  view.setFloat32(48, 0.625, true);
  return buffer;
}

/** A decoded-looking IQ frame: stream type 0 is RX IQ, 3 is TX IQ. */
function iqFrame(streamType = 0, floats = 128): ArrayBuffer {
  const buffer = new ArrayBuffer(64 + floats * 4);
  const view = new DataView(buffer);
  view.setUint32(4, 384_000, true);
  view.setUint32(20, floats, true);
  view.setUint32(24, streamType, true);
  return buffer;
}

type HarnessApi = {
  requestDisplayTransport: (reason?: string, force?: boolean) => boolean;
  applyDisplayEcho: (args: readonly (string | undefined)[]) => boolean;
  desiredDisplayCommand: () => string;
  iqStartGated: () => boolean;
  sendIqStart: () => boolean;
  flushDeferredIqStart: () => void;
  handleSpectrumRow: (buffer: ArrayBuffer) => void;
  handleBinaryFrame: (buffer: ArrayBuffer) => void;
  scanDisplayTransportText: (text: string) => void;
  resetDisplayTransport: (reason?: string) => void;
  readDisplayTransportOverride: (search: string) => string;
  displayTransportOverride: () => string;
  displayTransportRequested: () => boolean;
  acquireRawIqLease: (isReady: () => boolean, timeoutMs?: number, pollMs?: number) => Promise<boolean>;
  releaseRawIqLease: (reason?: string) => boolean;
};

type MeasureApi = {
  startRxSpectrumCapture: () => Promise<void>;
  finishRxSpectrumCapture: () => void;
  abortRxSpectrumCapture: (reason: string) => void;
  rxSpectrumSourceReady: () => boolean;
};

function makeHarness(options: {
  streamMode?: string;
  caps?: boolean;
  terrain?: boolean;
  targetFftSize?: number;
  intervalMs?: number;
  echoTimeoutMs?: number;
  dds?: number;
  search?: string;
  /** Also load the RX Measure functions, with fast timers and a fake accumulator. */
  measure?: {
    identityFails?: boolean;
    mode?: string;
    identityGate?: Promise<void>;
    /** Per call of the identity request (0, 1, ...): a promise to wait for, or none. */
    identityGateFor?: (call: number) => Promise<void> | undefined;
  };
} = {}) {
  const sent: string[] = [];
  const logs: string[] = [];
  const audioFrames: number[] = [];
  const opusFrames: number[] = [];
  const iqFrames: number[] = [];
  const sandbox: Record<string, unknown> = {
    _next: spectrumRow,
    state: {
      connected: true,
      bridgeReady: true,
      streamMode: options.streamMode ?? 'wan',
      displayCapsSpectrum: options.caps ?? true,
      displayRequested: '',
      displayEchoMode: 'iq',
      displayEchoFftSize: 0,
      displayEchoIntervalMs: 0,
      displayEchoReceived: false,
      displayEchoPending: false,
      displayRenderSource: 'iq',
      displayServerBins: null,
      displayServerFftSize: 0,
      displayServerCenterHz: 0,
      displayServerSeq: 0,
      displaySequence: 0,
      displayServerRowVersion: 0,
      displayServerLastRowAt: 0,
      displayServerBytes: 0,
      displayServerRowsWindow: 0,
      displayServerBytesWindow: 0,
      displayServerRowsReceived: 0,
      displayServerRowsAccepted: 0,
      displayServerRowsDroppedCenter: 0,
      displayServerRowsShiftedCenter: 0,
      displayServerRowsRejected: 0,
      displayServerSeqGaps: 0,
      displayServerLastReceivedSeq: 0,
      displayServerAckSeq: 0,
      displayServerAckCount: 0,
      displayServerAckLag: 0,
      displayServerRowRate: 0,
      displayServerByteRate: 0,
      displayIqStartDeferred: false,
      displayIqSource: 'rx',
      displayIqLease: false,
      displayIqLeaseEchoed: false,
      displayIqLeaseEchoBaseline: 0,
      displayIqLeaseGeneration: 0,
      displaySessionGeneration: 0,
      iqFrameVersion: 0,
      rxIqFrameVersion: 0,
      lastRxIqFrameAt: 0,
      iqPackets: [],
      iqStreaming: true,
      displayPaused: false,
      mode: options.measure?.mode ?? 'USB',
      dds: options.dds ?? 14_200_000,
      sampleRate: 192_000,
      lastFrameAt: 0,
      frameCounter: 0,
      displayCaption: '',
    },
    sendTci: (text: string) => { sent.push(text); },
    logEvent: (message: string) => { logs.push(message); },
    updateWsDiagMarker: () => {},
    scheduleUiRefresh: () => {},
    performance: { now: () => 1000 },
    window: {
      // RX Measure's 100 ms polling runs on fast timers so its 3 s wait takes
      // milliseconds; the page's own 1 s echo-fallback timer keeps its real delay.
      setTimeout: options.measure
        ? (fn: () => void, ms?: number) => setTimeout(fn, (ms ?? 0) >= 500 ? ms : 1)
        : setTimeout,
      clearTimeout,
      setInterval,
      clearInterval,
      location: { search: options.search ?? '' },
    },
    displayFftTargetSize: () => options.targetFftSize ?? 2048,
    displayRenderIntervalMs: () => options.intervalMs ?? 50,
    DISPLAY_ECHO_TIMEOUT_MS: options.echoTimeoutMs ?? 1000,
    DISPLAY_STATUS_REFRESH_INTERVAL_MS: 250,
    displayFirstIqAt: null,
    lastIqUiRefreshAt: -Infinity,
    TCI_STREAM_AUDIO_RX: 1,
    TCI_STREAM_AUDIO_OPUS_RX: 17,
    TCI_STREAM_IQ_TX: 3,
    TCI_STREAM_IQ_RX: 0,
    handleAudioFrame: (buffer: ArrayBuffer) => { audioFrames.push(buffer.byteLength); },
    handleOpusAudioFrame: (buffer: ArrayBuffer) => { opusFrames.push(buffer.byteLength); },
    handleIqFrame: (buffer: ArrayBuffer) => { iqFrames.push(buffer.byteLength); },
    txReceiveSuppressionActive: () => false,
    // Share the host realm's binary types so `instanceof` and typed-array
    // identity work across the vm boundary.
    ArrayBuffer,
    DataView,
    Uint8Array,
    Float32Array,
    // A browser global the vm context does not provide on its own.
    URLSearchParams,
  };
  const elements: Record<string, { textContent: string; disabled: boolean; hidden: boolean; value: string }> = {};
  if (options.measure) {
    class FakeAccumulator {
      samples = 0;
      startedAtMs: number;
      durationMs: number;
      settings: unknown;
      constructor(settings: unknown, _identity: unknown, startedAtMs: number, durationMs: number) {
        this.settings = settings;
        this.startedAtMs = startedAtMs;
        this.durationMs = durationMs;
      }
      finish() {
        return {
          samples: 1,
          quality: 'full',
          summary: { adjacentNoiseFloorMedianDb: 0, passbandExcessMedianDb: 0, widebandBurstRiseDb: 0 },
        };
      }
    }
    sandbox._next = { ...spectrumRow, RxSpectrumAccumulator: FakeAccumulator, rxSpectrumSettingsMatch: () => true };
    sandbox.$ = (id: string) =>
      (elements[id] ??= { textContent: '', disabled: false, hidden: false, value: '30' });
    sandbox.fftProcessor = { size: 2048 };
    sandbox.currentRxPassbandHz = () => ({ lowHz: 0, highHz: 0 });
    let identityCalls = 0;
    sandbox.fetch = async () => {
      const call = identityCalls;
      identityCalls += 1;
      const gate = options.measure?.identityGateFor?.(call);
      if (gate) await gate;
      if (options.measure?.identityGate) await options.measure.identityGate;
      if (options.measure?.identityFails) throw new Error('identity unavailable');
      return {
        ok: true,
        json: async () => ({
          pid_matches_service: true,
          age_seconds: 1,
          fpga: {
            build_identity_status: 'identified',
            firmware_display: '1.31',
            rx_filter: 'test',
            build_id_raw: 1,
            build_id_hex: '0x1',
          },
        }),
      };
    };
    Object.assign(sandbox.state as Record<string, unknown>, {
      rxAdc: 0, rxAntenna: 0, rxAttenuationDb: 0, agcMode: 'med', agcGain: 0,
      rxNoiseReductionMode: 'off', rxNbMode: 'off', filterLow: 0, filterHigh: 0, rxFilterShiftHz: 0,
    });
  }
  const measureSlice = options.measure ? `${slice(MEASURE_START, MEASURE_END)}\n` : '';
  const loadedCode = `${slice(TRANSPORT_START, TRANSPORT_END)}\n${slice(BINARY_START, BINARY_END)}\n${measureSlice}`;
  const api = runInNewContext(
    loadedCode +
      '({ requestDisplayTransport, applyDisplayEcho, desiredDisplayCommand, iqStartGated, ' +
      'sendIqStart, flushDeferredIqStart, handleSpectrumRow, handleBinaryFrame, scanDisplayTransportText, resetDisplayTransport, ' +
      'readDisplayTransportOverride, displayTransportOverride, displayTransportRequested, ' +
      'acquireRawIqLease, releaseRawIqLease' +
      (options.measure
        ? ', startRxSpectrumCapture, finishRxSpectrumCapture, abortRxSpectrumCapture, rxSpectrumSourceReady'
        : '') +
      ' })',
    sandbox,
  ) as unknown as HarnessApi & MeasureApi;
  if (options.measure) {
    // The page's own IQ-frame handler, not a stub: RX Measure's freshness rests on
    // what it records. Only drawing and history collaborators are replaced.
    const s = sandbox.state as Record<string, unknown> & { iqPackets: Float32Array[] };
    Object.assign(sandbox, {
      displaySampleRateHz: () => s.sampleRate,
      resetDisplayHistory: () => { s.iqPackets = []; s.displayIqSource = 'rx'; },
      appendIqPacket: (iq: Float32Array) => { s.iqPackets.push(iq); },
      displayIqForSource: (iq: Float32Array) => iq,
      TX_DISPLAY_SETTLE_SKIP_FRAMES: 0,
      TX_DISPLAY_SPAN_HZ: 96_000,
    });
    runInNewContext(slice(IQ_HANDLER_START, IQ_HANDLER_END), sandbox);
  }
  // Wherever the complete page declares a function the loaded code also declares, evaluate
  // the page's declarations in source order too, so the later one wins exactly as it does
  // in the real page. With no such name collision this changes nothing.
  const loadedNames = declaredFunctionNames(
    options.measure ? `${loadedCode}\n${slice(IQ_HANDLER_START, IQ_HANDLER_END)}` : loadedCode,
  );
  const pageDeclarations = pageFunctions.filter((fn) => loadedNames.has(fn.name)).map((fn) => fn.text).join('\n');
  if (pageDeclarations) runInNewContext(pageDeclarations, sandbox);
  return {
    api, sandbox, sent, logs, audioFrames, opusFrames, iqFrames, elements,
    state: sandbox.state as Record<string, unknown>,
  };
}

describe('WAN display transport negotiation', () => {
  it('sends nothing at all when the bridge did not advertise spectrum caps', () => {
    const h = makeHarness({ caps: false, streamMode: 'wan' });
    expect(h.api.requestDisplayTransport('test', true)).toBe(false);
    expect(h.sent).toEqual([]);
  });

  it('requests spectrum rows in WAN mode and raw IQ in LAN mode', () => {
    const wan = makeHarness({ streamMode: 'wan', targetFftSize: 2048, intervalMs: 50 });
    expect(wan.api.requestDisplayTransport('test', true)).toBe(true);
    expect(wan.sent).toEqual(['saturn_display:spectrum,2048,50;']);

    const lan = makeHarness({ streamMode: 'lan' });
    expect(lan.api.desiredDisplayCommand()).toBe('saturn_display:iq;');
    expect(lan.api.requestDisplayTransport('test', true)).toBe(true);
    expect(lan.sent).toEqual(['saturn_display:iq;']);
  });

  it('applies the 4096 cap and the 10 Hz terrain cadence', () => {
    const terrain = makeHarness({ terrain: true, targetFftSize: 16384, intervalMs: 50 });
    terrain.state.terrainActive = true;
    expect(terrain.api.desiredDisplayCommand()).toBe('saturn_display:spectrum,4096,100;');
  });

  it('switches the render source only after the echo', () => {
    const h = makeHarness({ streamMode: 'wan' });
    h.api.requestDisplayTransport('test', true);
    expect(h.state.displayRenderSource).toBe('iq');
    expect(h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50'])).toBe(true);
    expect(h.state.displayRenderSource).toBe('server');
    expect(h.state.displayEchoFftSize).toBe(2048);
    expect(h.state.displayEchoIntervalMs).toBe(50);
    // An iq echo (LAN or a revert) drops the source back and releases rows.
    expect(h.api.applyDisplayEcho(['0', 'iq'])).toBe(true);
    expect(h.state.displayRenderSource).toBe('iq');
    expect(h.state.displayServerBins).toBeNull();
  });

  it('withholds iq_start until the echo arrives, then flushes it', () => {
    const h = makeHarness({ streamMode: 'wan' });
    expect(h.api.iqStartGated()).toBe(true);
    expect(h.api.sendIqStart()).toBe(false);
    expect(h.sent).not.toContain('iq_start:0;');
    expect(h.state.displayIqStartDeferred).toBe(true);
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    expect(h.sent).toContain('iq_start:0;');
    expect(h.state.displayIqStartDeferred).toBe(false);
  });

  it('never withholds iq_start without caps or outside WAN', () => {
    const lan = makeHarness({ streamMode: 'lan' });
    expect(lan.api.iqStartGated()).toBe(false);
    expect(lan.api.sendIqStart()).toBe(true);
    expect(lan.sent).toContain('iq_start:0;');

    const noCaps = makeHarness({ streamMode: 'wan', caps: false });
    expect(noCaps.api.iqStartGated()).toBe(false);
    expect(noCaps.api.sendIqStart()).toBe(true);
    expect(noCaps.sent).toContain('iq_start:0;');
  });

  it('falls back to raw IQ when the echo never arrives', async () => {
    const h = makeHarness({ streamMode: 'wan', echoTimeoutMs: 5 });
    h.api.sendIqStart();
    expect(h.state.displayIqStartDeferred).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(h.state.displayEchoReceived).toBe(true);
    expect(h.state.displayRenderSource).toBe('iq');
    expect(h.sent).toContain('iq_start:0;');
    expect(h.state.displayIqStartDeferred).toBe(false);
  });

  it('does not send a duplicate request while an echo is already pending', () => {
    const h = makeHarness({ streamMode: 'wan' });
    // `ready;` requests the mode once, then recovery calls sendIqStart.
    h.api.requestDisplayTransport('bridge ready', true);
    const afterReady = h.sent.length;
    expect(afterReady).toBe(1);
    expect(h.api.sendIqStart()).toBe(false);
    expect(h.sent.length).toBe(afterReady);
    expect(h.state.displayIqStartDeferred).toBe(true);
    // Once the outstanding request resolves, a real gate can ask again.
    h.state.displayEchoPending = false;
    h.api.sendIqStart();
    expect(h.sent.length).toBe(afterReady + 1);
  });

  it('re-negotiates on the greeting caps line and on pairing', () => {
    const h = makeHarness({ streamMode: 'wan' });
    h.api.scanDisplayTransportText('saturn_display_caps:spectrum_u8;');
    expect(h.state.displayCapsSpectrum).toBe(true);
    h.api.scanDisplayTransportText('session_paired:split-1;');
    expect(h.sent).toEqual(['saturn_display:spectrum,2048,50;']);
    h.api.scanDisplayTransportText('saturn_display:0,spectrum,1024,33;');
    expect(h.state.displayRenderSource).toBe('server');
    expect(h.state.displayEchoFftSize).toBe(1024);
  });

  it('recovers when a superseded echo arrives after a mode toggle', () => {
    const h = makeHarness({ streamMode: 'wan' });
    h.api.requestDisplayTransport('wan', true);          // asks for spectrum
    h.state.streamMode = 'lan';
    h.api.requestDisplayTransport('lan', true);          // supersedes with iq
    // The slow echo for the first request lands now and flips the source.
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    expect(h.state.displayRenderSource).toBe('server');
    // The periodic reconcile sees a confirmed mode that no longer matches the
    // desire and re-asserts it instead of leaving a frozen server display.
    expect(h.api.requestDisplayTransport('periodic')).toBe(true);
    expect(h.sent[h.sent.length - 1]).toBe('saturn_display:iq;');
  });
});

describe('spectrum row handling in the template', () => {
  it('accepts a matching row, publishes bins and acks it', () => {
    const h = makeHarness();
    h.state.displayEchoMode = 'spectrum';
    h.state.displayRenderSource = 'server';
    h.api.handleSpectrumRow(buildRow({ binCount: 256, sequence: 5 }));
    const bins = h.state.displayServerBins as Float32Array;
    expect(bins.length).toBe(256);
    expect(bins[0]).toBeCloseTo(-160, 5);
    expect(h.state.displayServerSeq).toBe(5);
    expect(h.state.displayServerRowVersion).toBe(1);
    expect(h.state.displayServerRowsAccepted).toBe(1);
    expect(h.state.sampleRate).toBe(384_000);
    expect(h.state.lastFrameAt).toBe(1000);
    expect(h.sent).toEqual(['saturn_display_ack:5;']);
  });

  it('keeps a slightly lagging row while tuning instead of freezing the display', () => {
    // state.dds leads the bridge by a few kHz during a drag; a 5 kHz offset is
    // far inside the 384 kHz row, so the row must still be drawn.
    const h = makeHarness({ dds: 14_205_000 });
    h.api.handleSpectrumRow(buildRow({ sequence: 9, centerHz: 14_200_000 }));
    expect(h.state.displayServerRowsAccepted).toBe(1);
    expect(h.state.displayServerRowsShiftedCenter).toBe(1);
    expect(h.state.displayServerRowsDroppedCenter).toBe(0);
    expect(h.state.displayServerCenterHz).toBe(14_200_000);
    expect(h.state.displayServerBins).toBeInstanceOf(Float32Array);
    expect(h.sent).toEqual(['saturn_display_ack:9;']);
  });

  it('drops a row whose center is outside the row span but still acks it', () => {
    // 7.1 MHz away: the row no longer covers the display at all.
    const h = makeHarness({ dds: 7_100_000 });
    h.api.handleSpectrumRow(buildRow({ sequence: 9, centerHz: 14_200_000 }));
    expect(h.state.displayServerRowsDroppedCenter).toBe(1);
    expect(h.state.displayServerRowsAccepted).toBe(0);
    expect(h.state.displayServerBins).toBeNull();
    expect(h.sent).toEqual(['saturn_display_ack:9;']);
  });

  it('does not count center-dropped rows as sequence gaps', () => {
    const h = makeHarness({ dds: 7_100_000 });
    h.api.handleSpectrumRow(buildRow({ sequence: 1, centerHz: 14_200_000 }));
    h.api.handleSpectrumRow(buildRow({ sequence: 2, centerHz: 14_200_000 }));
    h.api.handleSpectrumRow(buildRow({ sequence: 3, centerHz: 14_200_000 }));
    expect(h.state.displayServerRowsDroppedCenter).toBe(3);
    expect(h.state.displayServerSeqGaps).toBe(0);
  });

  it('counts a real sequence gap and ignores a late duplicate', () => {
    const h = makeHarness();
    h.api.handleSpectrumRow(buildRow({ sequence: 5 }));
    h.api.handleSpectrumRow(buildRow({ sequence: 9 }));
    h.api.handleSpectrumRow(buildRow({ sequence: 9 }));
    expect(h.state.displayServerSeqGaps).toBe(3);
    expect(h.state.displayServerLastReceivedSeq).toBe(9);
  });

  it('rebases on a backward sequence instead of disabling gap accounting', () => {
    // A new media socket restarts the bridge's per-client sequence at 1; the
    // old code left `sequence > previous` permanently false.
    const h = makeHarness();
    h.api.handleSpectrumRow(buildRow({ sequence: 170 }));
    expect(h.state.displayServerLastReceivedSeq).toBe(170);
    h.api.handleSpectrumRow(buildRow({ sequence: 1 }));
    expect(h.state.displayServerLastReceivedSeq).toBe(1);
    expect(h.state.displayServerSeqGaps).toBe(0);
    // Gap accounting keeps working after the rebase.
    h.api.handleSpectrumRow(buildRow({ sequence: 5 }));
    expect(h.state.displayServerSeqGaps).toBe(3);
    expect(h.state.displayServerLastReceivedSeq).toBe(5);
  });

  it('clears the row sequence counters on transport reset', () => {
    const h = makeHarness();
    h.api.handleSpectrumRow(buildRow({ sequence: 44 }));
    expect(h.state.displayServerLastReceivedSeq).toBe(44);
    h.api.resetDisplayTransport('test');
    expect(h.state.displayServerLastReceivedSeq).toBe(0);
    expect(h.state.displayServerSeq).toBe(0);
    expect(h.state.displayServerAckSeq).toBe(0);
  });

  it('rejects a malformed row yet still acks a readable sequence', () => {
    const h = makeHarness();
    // Right stream type (so the sequence is readable) but a bad format: the
    // row is dropped, and the ack keeps the bridge credit window moving.
    h.api.handleSpectrumRow(buildRow({ sequence: 11, format: 0x5300 }));
    expect(h.state.displayServerRowsRejected).toBe(1);
    expect(h.state.displayServerRowsAccepted).toBe(0);
    expect(h.sent).toEqual(['saturn_display_ack:11;']);
  });

  it('routes type 16 and Opus audio independently, and ignores unknown types', () => {
    const h = makeHarness();
    h.api.handleBinaryFrame(buildRow({ binCount: 256 }));
    expect(h.state.displayServerBins).toBeInstanceOf(Float32Array);
    expect(h.iqFrames).toEqual([]);
    expect(h.audioFrames).toEqual([]);

    h.api.handleBinaryFrame(buildRow({ binCount: 64, streamType: 17 }));
    expect(h.opusFrames).toEqual([128]);
    expect(h.iqFrames).toEqual([]);

    const before = h.state.displayServerRowsReceived;
    const unknown = buildRow({ binCount: 64 });
    new DataView(unknown).setUint32(24, 99, true);
    h.api.handleBinaryFrame(unknown);
    expect(h.state.displayServerRowsReceived).toBe(before);
  });

  it('clears caps and rows on disconnect', () => {
    const h = makeHarness();
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    h.api.resetDisplayTransport('test');
    expect(h.state.displayCapsSpectrum).toBe(false);
    expect(h.state.displayRenderSource).toBe('iq');
    expect(h.state.displayServerBins).toBeNull();
    expect(h.state.displayRequested).toBe('');
  });
});

function toneBins(size: number, toneIndex: number): Float32Array {
  const bins = new Float32Array(size).fill(-160);
  bins[toneIndex] = -20;
  return bins;
}

function makeDrawHarness() {
  const captured: Array<{ bins: Float32Array; sequence: number; sourceBins: number }> = [];
  const state: Record<string, unknown> = {
    displayPaused: false,
    displaySequence: 0,
    displayFloorDb: -160,
    waterfallSettleFrames: 0,
    waterfallFrameSkipCounter: 0,
    lastSpectrumRenderAt: 0,
    waterfallFloorDb: -200,
    waterfallCeilingDb: -120,
    waterfallPalette: 'classic',
    waterfallContrast: 100,
    waterfallSmoothing: 0,
    sampleRate: 384_000,
    displayZoom: 4,
    terrainActive: false,
  };
  const sandbox: Record<string, unknown> = {
    _next: spectrumRow,
    state,
    // The template has a one-arg wrapper over the two-arg helper.
    visibleBinsForDisplay: (bins: Float32Array | null) => visibleBinsForDisplay(bins, Number(state.displayZoom)),
    shiftBinsHorizontally,
    displaySpanHz: () => displaySpanHz(Number(state.sampleRate), Number(state.displayZoom)),
    acceptSpectrumFrame: (bins: Float32Array, _now: number, sequence: number, sourceBins: number) => {
      captured.push({ bins: Float32Array.from(bins), sequence, sourceBins });
    },
    processedDisplayBins: (bins: Float32Array) => bins,
    updateDisplayRange: () => {},
    spectrumRenderer: { render: () => {} },
    waterfallRenderer: { pushLine: () => {}, render: () => {} },
    shouldPushWaterfallLine: () => true,
    clampWaterfallSpeed: () => 1,
    performance: { now: () => 1000 },
  };
  const api = runInNewContext(
    `${slice(DRAW_START, DRAW_END)}\n({ drawDisplayBins })`,
    sandbox,
  ) as unknown as {
    drawDisplayBins: (
      bins: Float32Array,
      now: number,
      waterfallEnabled: boolean,
      phoneWanLite: boolean,
      centerShiftHz?: number,
    ) => number;
  };
  return { api, captured, state };
}

describe('drawDisplayBins across both render sources', () => {
  it('feeds history one monotonic sequence, so a source switch cannot stall it', () => {
    // SpectrumHistory rejects frames whose sequence does not advance. With
    // per-source counters, switching iq -> server over an identical mapping
    // would present sequences 1..N below the raw counter's last value and the
    // whole history (terrain, noise floor, waterfall rebuild) would stall.
    const h = makeDrawHarness();
    const bins = toneBins(2048, 1024);
    h.api.drawDisplayBins(bins, 1000, true, false); // last raw IQ frame
    h.api.drawDisplayBins(bins, 2000, true, false); // first WAN server row
    h.api.drawDisplayBins(bins, 3000, true, false);
    expect(h.captured.map((entry) => entry.sequence)).toEqual([1, 2, 3]);
    expect(h.state.displaySequence).toBe(3);
  });

  it('shifts a lagging row by exactly the ruler displacement', () => {
    const h = makeDrawHarness();
    const bins = toneBins(2048, 1024);
    h.api.drawDisplayBins(bins, 0, true, false, 0);
    // Row center sits 5 kHz below the optimistic dds, as it does mid-drag.
    h.api.drawDisplayBins(bins, 0, true, false, -5000);
    const baseline = h.captured[0]!;
    const shifted = h.captured[1]!;
    const peakIndex = (values: Float32Array) => values.indexOf(Math.max(...values));
    const visibleCount = visibleBinsForDisplay(bins, 4)!.length;
    const expected = Math.round((-5000 / (384_000 / 4)) * visibleCount);
    expect(expected).toBe(-27);
    expect(peakIndex(shifted.bins) - peakIndex(baseline.bins)).toBe(expected);
    // The vacated edge is filled with the display floor, never NaN.
    expect(shifted.bins[shifted.bins.length - 1]).toBe(-160);
  });

  it('moves a real tone with the ruler, not just the array centre', () => {
    const h = makeDrawHarness();
    const bins = toneBins(2048, 1024 + 37);
    h.api.drawDisplayBins(bins, 0, true, false, 0);
    h.api.drawDisplayBins(bins, 0, true, false, -5000);
    const baseline = h.captured[0]!;
    const shifted = h.captured[1]!;
    const peakIndex = (values: Float32Array) => values.indexOf(Math.max(...values));
    expect(peakIndex(shifted.bins) - peakIndex(baseline.bins)).toBe(-27);
    // The tone is translated, not clipped.
    expect(Math.max(...shifted.bins)).toBeCloseTo(-20, 5);
  });
});

describe('display transport override for matched measurements', () => {
  it('defaults to auto and keeps the original rule: rows only in WAN', () => {
    const wan = makeHarness({ streamMode: 'wan' });
    expect(wan.api.displayTransportOverride()).toBe('auto');
    expect(wan.api.desiredDisplayCommand()).toMatch(/^saturn_display:spectrum,/);
    const lan = makeHarness({ streamMode: 'lan' });
    expect(lan.api.desiredDisplayCommand()).toBe('saturn_display:iq;');
    expect(lan.api.iqStartGated()).toBe(false);
  });

  it('reads only spectrum and iq, in any case, and treats anything else as auto', () => {
    const { api } = makeHarness();
    expect(api.readDisplayTransportOverride('?display_transport=spectrum')).toBe('spectrum');
    expect(api.readDisplayTransportOverride('?x=1&display_transport=IQ')).toBe('iq');
    expect(api.readDisplayTransportOverride('?display_transport= Spectrum ')).toBe('spectrum');
    for (const search of ['', '?display_transport=', '?display_transport=wan', '?display_transport=1', '?transport=spectrum', undefined as unknown as string]) {
      expect(api.readDisplayTransportOverride(search)).toBe('auto');
    }
  });

  it('spectrum override requests rows in a LAN session at the LAN display profile', () => {
    const h = makeHarness({
      streamMode: 'lan', search: '?display_transport=spectrum', targetFftSize: 8192, intervalMs: 16,
    });
    expect(h.api.displayTransportOverride()).toBe('spectrum');
    // The bridge's own clamps: 4096 bins at no faster than 33 ms (about 30 rows/s).
    expect(h.api.desiredDisplayCommand()).toBe('saturn_display:spectrum,4096,33;');
    expect(h.api.requestDisplayTransport('test', true)).toBe(true);
    expect(h.sent).toEqual(['saturn_display:spectrum,4096,33;']);
    // iq_start waits for the echo, then is flushed, exactly as in WAN.
    expect(h.api.iqStartGated()).toBe(true);
    expect(h.api.sendIqStart()).toBe(false);
    h.api.applyDisplayEcho(['0', 'spectrum', '4096', '33']);
    expect(h.state.displayRenderSource).toBe('server');
    expect(h.sent).toContain('iq_start:0;');
    // The override does not change the RX transport mode.
    expect(h.state.streamMode).toBe('lan');
  });

  it('iq override keeps raw IQ in a WAN session and never gates iq_start', () => {
    const h = makeHarness({ streamMode: 'wan', search: '?display_transport=iq' });
    expect(h.api.desiredDisplayCommand()).toBe('saturn_display:iq;');
    expect(h.api.iqStartGated()).toBe(false);
    expect(h.api.sendIqStart()).toBe(true);
    expect(h.sent).toContain('iq_start:0;');
    expect(h.state.streamMode).toBe('wan');
  });

  it('still sends nothing when the bridge did not advertise spectrum caps', () => {
    const h = makeHarness({ caps: false, streamMode: 'lan', search: '?display_transport=spectrum' });
    expect(h.api.requestDisplayTransport('test', true)).toBe(false);
    expect(h.sent).toEqual([]);
    expect(h.api.iqStartGated()).toBe(false);
  });

  it('cannot change the RX audio or IQ profile: those depend only on the RX transport mode', () => {
    expect(template).toContain('_next.buildRxAudioStartCommand(state.streamMode, state.rxVolumeDb)');
    expect(template).toContain('_next.rxAudioTransportProfile(state.streamMode)');
    expect(template).toContain('_next.effectiveIqSampleRate(state.sampleRate, state.streamMode)');
    const overrideAt = template.indexOf('function readDisplayTransportOverride');
    const overrideEnd = template.indexOf('function displayTransportRequested');
    const overrideSource = template.slice(overrideAt, overrideEnd);
    expect(overrideSource).not.toMatch(/streamMode\s*=[^=]/);
    expect(overrideSource).not.toMatch(/sendTci|iq_samplerate|audio_start/);
  });

  it('labels the override in the exported display diagnostics', () => {
    expect(template).toContain('Display transport override: ${displayTransportOverride()}');
    expect(template).toContain('displayTransportOverride: displayTransportOverride(),');
  });
});

describe('RX Measure raw-IQ lease under the display override', () => {
  const ROWS = 'saturn_display:spectrum,2048,50;';
  const IQ = 'saturn_display:iq;';
  const SPECTRUM_OVERRIDE = '?display_transport=spectrum';

  /** A LAN page the override holds on server rows: echo received, rows flowing. */
  function onRows(options: { search?: string; streamMode?: string; measure?: { identityFails?: boolean; mode?: string } } = {}) {
    const h = makeHarness({
      search: options.search ?? SPECTRUM_OVERRIDE,
      streamMode: options.streamMode ?? 'lan',
      measure: options.measure ?? {},
    });
    h.api.requestDisplayTransport('test', true);
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    // Rows are flowing: a genuine row, which refreshes the shared frame timestamp.
    h.api.handleSpectrumRow(buildRow());
    h.sent.length = 0;
    return h;
  }
  const status = (h: ReturnType<typeof onRows>) => h.elements['rx-spectrum-status']?.textContent ?? '';
  /** What the bridge does after `saturn_display:iq;`: echo it, then raw RX IQ frames flow. */
  const bridgeSwitchesToIq = (h: ReturnType<typeof onRows>) => {
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
  };

  it('takes raw IQ for the capture, holds it throughout, and puts the rows back at the end', async () => {
    const h = onRows();
    expect(h.api.rxSpectrumSourceReady()).toBe(false);
    const started = h.api.startRxSpectrumCapture();
    // Explicitly asks for raw IQ, and only for that.
    expect(h.sent).toEqual([IQ]);
    expect(h.state.displayIqLease).toBe(true);
    expect(h.api.displayTransportRequested()).toBe(false);
    expect(status(h)).toContain('Switching the display to raw RX IQ');
    // The capture does not begin on the request alone, only once raw IQ is confirmed.
    expect(h.api.rxSpectrumSourceReady()).toBe(false);
    bridgeSwitchesToIq(h);
    await started;
    expect(status(h)).toContain('Capturing');
    expect(h.state.displayIqLease).toBe(true);
    expect(h.sent).toEqual([IQ]);
    h.api.finishRxSpectrumCapture();
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent).toEqual([IQ, ROWS]);
    expect(h.api.displayTransportRequested()).toBe(true);
    // Audio and RX transport settings were never involved.
    expect(h.state.streamMode).toBe('lan');
    expect(h.sent.every((command) => command.startsWith('saturn_display:'))).toBe(true);
  });

  it('puts the rows back when the capture is stopped', async () => {
    const h = onRows();
    const started = h.api.startRxSpectrumCapture();
    bridgeSwitchesToIq(h);
    await started;
    h.api.abortRxSpectrumCapture('stream or RX settings changed');
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent).toEqual([IQ, ROWS]);
    expect(status(h)).toContain('Capture stopped');
    expect(h.elements['rx-spectrum-start-btn']?.disabled).toBe(false);
  });

  it('gives the display back when raw IQ never arrives', async () => {
    const h = onRows();
    await h.api.startRxSpectrumCapture();
    expect(h.sent).toEqual([IQ, ROWS]);
    expect(h.state.displayIqLease).toBe(false);
    expect(status(h)).toContain('display was put back');
    expect(h.elements['rx-spectrum-start-btn']?.disabled).toBe(false);
    expect(h.api.displayTransportRequested()).toBe(true);
  });

  it('gives the display back when the capture cannot start after raw IQ arrived', async () => {
    const h = onRows({ measure: { identityFails: true } });
    const started = h.api.startRxSpectrumCapture();
    bridgeSwitchesToIq(h);
    await started;
    expect(status(h)).toContain('Cannot start capture');
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent).toEqual([IQ, ROWS]);
    expect(h.elements['rx-spectrum-start-btn']?.disabled).toBe(false);
  });

  it('leaves everything alone for a mode RX Measure does not support', async () => {
    const h = onRows({ measure: { mode: 'WFM' } });
    await h.api.startRxSpectrumCapture();
    expect(h.sent).toEqual([]);
    expect(h.state.displayIqLease).toBe(false);
  });

  it('does not change what RX Measure does without the override', async () => {
    // WAN rows from the original rule: RX Measure still refuses, and asks for nothing.
    const wan = onRows({ search: '', streamMode: 'wan' });
    await wan.api.startRxSpectrumCapture();
    expect(status(wan)).toContain('Start RX IQ on the raw IQ display path');
    expect(wan.sent).toEqual([]);
    expect(wan.state.displayIqLease).toBe(false);
  });

  it('needs no lease when the page is already on raw IQ', async () => {
    const h = makeHarness({ search: '?display_transport=iq', streamMode: 'wan', measure: {} });
    h.api.requestDisplayTransport('test', true);
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    h.sent.length = 0;
    await h.api.startRxSpectrumCapture();
    expect(status(h)).toContain('Capturing');
    h.api.finishRxSpectrumCapture();
    expect(h.sent).toEqual([]);
    expect(h.state.displayIqLease).toBe(false);
  });

  it('a disconnect while waiting clears the lease and sends nothing further', async () => {
    const h = onRows();
    const started = h.api.startRxSpectrumCapture();
    expect(h.sent).toEqual([IQ]);
    h.api.resetDisplayTransport('disconnect');
    expect(h.state.displayIqLease).toBe(false);
    h.state.connected = false;
    await started;
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent).toEqual([IQ]);
    expect(status(h)).toContain('connection changed');
  });

  it('a disconnect during the capture ends it without a stale request', async () => {
    const h = onRows();
    const started = h.api.startRxSpectrumCapture();
    bridgeSwitchesToIq(h);
    await started;
    h.api.resetDisplayTransport('disconnect');
    expect(h.state.displayIqLease).toBe(false);
    h.state.connected = false;
    h.api.abortRxSpectrumCapture('RX IQ stream stopped');
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent).toEqual([IQ]);
  });

  it('a lease cannot outlive a disconnect and distort the next connection', async () => {
    const h = onRows();
    const started = h.api.startRxSpectrumCapture();
    bridgeSwitchesToIq(h);
    await started;
    // The connection drops mid-capture and comes back before the capture's own
    // timer notices: the new connection must ask for rows, not raw IQ.
    h.api.resetDisplayTransport('disconnect');
    h.sent.length = 0;
    h.api.scanDisplayTransportText('saturn_display_caps:spectrum_u8;');
    expect(h.api.requestDisplayTransport('bridge ready', true)).toBe(true);
    expect(h.sent).toEqual([ROWS]);
  });

  it('asks for nothing from a bridge that did not advertise spectrum support', async () => {
    const h = makeHarness({ caps: false, search: SPECTRUM_OVERRIDE, streamMode: 'lan' });
    expect(await h.api.acquireRawIqLease(() => true)).toBe(false);
    expect(h.sent).toEqual([]);
    expect(h.state.displayIqLease).toBe(false);
  });
});

describe('display override across reconnect and fallback', () => {
  it('asks for rows again after a reconnect, and withholds iq_start until the echo', () => {
    const h = makeHarness({ search: '?display_transport=spectrum', streamMode: 'lan' });
    h.api.scanDisplayTransportText('saturn_display_caps:spectrum_u8;');
    h.api.requestDisplayTransport('bridge ready', true);
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    h.api.resetDisplayTransport('disconnect');
    expect(h.state.displayCapsSpectrum).toBe(false);
    expect(h.api.displayTransportRequested()).toBe(false);
    h.sent.length = 0;
    // The new connection's greeting advertises the capability again.
    h.api.scanDisplayTransportText('saturn_display_caps:spectrum_u8;');
    expect(h.api.requestDisplayTransport('bridge ready', true)).toBe(true);
    expect(h.sent).toEqual(['saturn_display:spectrum,2048,50;']);
    expect(h.api.iqStartGated()).toBe(true);
    expect(h.api.sendIqStart()).toBe(false);
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    expect(h.sent).toContain('iq_start:0;');
  });

  it('falls back to raw IQ when the echo never arrives, even in a LAN session', async () => {
    const h = makeHarness({ search: '?display_transport=spectrum', streamMode: 'lan', echoTimeoutMs: 5 });
    h.api.sendIqStart();
    expect(h.state.displayIqStartDeferred).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(h.state.displayRenderSource).toBe('iq');
    expect(h.sent).toContain('iq_start:0;');
    expect(h.state.streamMode).toBe('lan');
  });
});

// The three probes marked "review probe" come from DarkOverLord's review of
// dea989e (R1), ported onto this file's shared harness with the same behavioral
// contract. Original file: review-raw-iq-freshness.test.ts in that review package.
describe('RX Measure readiness needs fresh raw RX IQ from this connection', () => {
  afterEach(() => { vi.useRealTimers(); });

  const ROWS = 'saturn_display:spectrum,2048,50;';
  const IQ = 'saturn_display:iq;';

  /** A fake clock, so the page's own 1 s display fallback and the 3 s lease timeout both play out. */
  function timed(measure: { identityGate?: Promise<void>; identityGateFor?: (call: number) => Promise<void> | undefined; identityFails?: boolean; mode?: string } = {}, search = '?display_transport=spectrum') {
    vi.useFakeTimers();
    vi.setSystemTime(1000);
    const h = makeHarness({ streamMode: 'lan', search, measure });
    Object.assign(h.sandbox.window as object, { setTimeout, clearTimeout, setInterval, clearInterval });
    h.sandbox.performance = { now: () => Date.now() };
    h.api.requestDisplayTransport('setup', true);
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    h.api.handleSpectrumRow(buildRow()); // a genuine row: it refreshes the shared lastFrameAt
    h.sent.length = 0;
    return h;
  }
  type Timed = ReturnType<typeof timed>;
  const status = (h: Timed) => h.elements['rx-spectrum-status']?.textContent ?? '';
  const settle = (ms: number) => vi.advanceTimersByTimeAsync(ms);
  const cleanup = async (h: Timed, pending: Promise<void>) => {
    h.api.finishRxSpectrumCapture();
    h.api.resetDisplayTransport();
    await settle(3500);
    await pending;
  };

  it('review probe, positive control: the IQ echo plus a newly decoded RX IQ frame starts a capture', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(100);
    await pending;
    try {
      expect(h.state.rxIqFrameVersion).toBe(1);
      expect(status(h)).toContain('Capturing');
    } finally {
      h.api.finishRxSpectrumCapture();
      h.api.resetDisplayTransport();
    }
  });

  it('review probe: the echo alone does not start a capture, and the lease times out and gives the rows back', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    await settle(100);
    expect(h.state.rxIqFrameVersion).toBe(0);
    expect(status(h)).not.toContain('Capturing');
    await settle(3500);
    await pending;
    expect(status(h)).not.toContain('Capturing');
    expect(status(h)).toContain('display was put back');
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent).toEqual([IQ, ROWS]);
  });

  it('review probe: the display fallback with no echo and no raw IQ does not start a capture', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    // The page gives up negotiating at 1000 ms, before the lease's 3000 ms timeout.
    await settle(1100);
    expect(h.state.displayRenderSource).toBe('iq');
    expect(h.state.rxIqFrameVersion).toBe(0);
    expect(status(h)).not.toContain('Capturing');
    await settle(3000);
    await pending;
    expect(status(h)).not.toContain('Capturing');
    expect(status(h)).toContain('display was put back');
    expect(h.sent.at(-1)).toBe(ROWS);
  });

  it('spectrum rows arriving all the while confirm nothing', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    for (let i = 0; i < 25; i += 1) {
      h.api.handleSpectrumRow(buildRow({ sequence: 2 + i }));
      await settle(100);
    }
    await settle(1000);
    await pending;
    expect(status(h)).not.toContain('Capturing');
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent.at(-1)).toBe(ROWS);
  });

  it('raw IQ that arrived before the request does not count, and neither does IQ before the echo', async () => {
    const h = timed();
    h.api.handleBinaryFrame(iqFrame(0)); // old: before the lease
    h.state.iqPackets = [new Float32Array(8)]; // IQ kept from before the lease
    const pending = h.api.startRxSpectrumCapture();
    h.api.handleBinaryFrame(iqFrame(0)); // in flight, after the request but before the echo
    h.api.applyDisplayEcho(['0', 'iq']);
    await settle(300);
    expect(h.state.rxIqFrameVersion).toBe(2);
    expect(status(h)).not.toContain('Capturing');
    // Whatever was kept from before is gone: the window starts at the acknowledgement.
    expect(h.state.iqPackets).toEqual([]);
    h.api.handleBinaryFrame(iqFrame(0)); // the first frame after the acknowledgement
    await settle(100);
    await pending;
    expect(status(h)).toContain('Capturing');
    expect((h.state.iqPackets as unknown[]).length).toBe(1);
    await cleanup(h, Promise.resolve());
  });

  it('TX IQ does not confirm a raw RX IQ lease', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(3));
    h.api.handleBinaryFrame(iqFrame(3));
    await settle(300);
    expect(h.state.iqFrameVersion).toBe(2);
    expect(h.state.rxIqFrameVersion).toBe(0);
    expect(status(h)).not.toContain('Capturing');
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(100);
    await pending;
    expect(status(h)).toContain('Capturing');
    await cleanup(h, Promise.resolve());
  });

  it('a reconnect while waiting cannot start a capture on the new connection', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    h.api.resetDisplayTransport('disconnect');
    h.state.connected = false;
    await settle(200);
    // The connection comes back, negotiates rows again, and raw IQ then flows.
    h.state.connected = true;
    h.api.scanDisplayTransportText('saturn_display_caps:spectrum_u8;');
    h.api.requestDisplayTransport('bridge ready', true);
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(3500);
    await pending;
    expect(status(h)).not.toContain('Capturing');
    expect(h.state.displayIqLease).toBe(false);
  });

  it('a disconnect while the FPGA identity is being read cannot start a later capture', async () => {
    let open!: () => void;
    const gate = new Promise<void>((resolve) => { open = resolve; });
    const h = timed({ identityGate: gate });
    const pending = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(100); // confirmed; the start now waits on the identity request
    expect(status(h)).not.toContain('Capturing');
    h.api.resetDisplayTransport('disconnect');
    h.state.connected = true; // reconnected before the old request answers
    h.api.handleBinaryFrame(iqFrame(0)); // and raw IQ is flowing again
    open();
    await settle(100);
    await pending;
    expect(status(h)).toContain('connection changed');
    expect(status(h)).not.toContain('Capturing');
    expect(h.elements['rx-spectrum-start-btn']?.disabled).toBe(false);
  });

  it('the display fallback followed by raw IQ is still not an acknowledgement', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    await settle(1100); // the page gave up waiting for the echo and fell back to raw IQ
    expect(h.state.displayRenderSource).toBe('iq');
    for (let i = 0; i < 10; i += 1) {
      h.api.handleBinaryFrame(iqFrame(0)); // raw IQ is flowing, but the bridge never echoed
      await settle(100);
    }
    expect(status(h)).not.toContain('Capturing');
    await settle(3000);
    await pending;
    expect(status(h)).not.toContain('Capturing');
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent.at(-1)).toBe(ROWS);
  });

  it('a stale raw RX IQ frame is not fresh', async () => {
    const h = timed({}, '?display_transport=iq');
    h.api.requestDisplayTransport('setup', true);
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(2000); // the raw IQ stream then goes quiet
    await h.api.startRxSpectrumCapture();
    expect(status(h)).toContain('Start RX IQ on the raw IQ display path');
    h.api.handleBinaryFrame(iqFrame(0));
    await h.api.startRxSpectrumCapture();
    expect(status(h)).toContain('Capturing');
    await cleanup(h, Promise.resolve());
  });

  it('a start from a dead connection cannot disturb the capture running on the new one', async () => {
    let open!: () => void;
    const firstGate = new Promise<void>((resolve) => { open = resolve; });
    const h = timed({ identityGateFor: (call) => (call === 0 ? firstGate : undefined) });
    const first = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(100); // start #1 is confirmed and now waits on its identity request
    // The connection drops and comes back; rows are negotiated again.
    h.api.resetDisplayTransport('disconnect');
    h.state.connected = true;
    h.api.scanDisplayTransportText('saturn_display_caps:spectrum_u8;');
    h.api.requestDisplayTransport('bridge ready', true);
    h.api.applyDisplayEcho(['0', 'spectrum', '2048', '50']);
    h.api.handleSpectrumRow(buildRow({ sequence: 9 }));
    h.sent.length = 0;
    // A new capture on the new connection gets raw IQ and starts.
    const second = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(100);
    await second;
    expect(status(h)).toContain('Capturing');
    expect(h.state.displayIqLease).toBe(true);
    const sentBefore = h.sent.slice();
    // Now the old start's identity request answers.
    open();
    await settle(100);
    await first;
    expect(status(h)).toContain('Capturing'); // not overwritten
    expect(h.state.displayIqLease).toBe(true); // not released by the old start
    expect(h.sent).toEqual(sentBefore); // and it asked the bridge for nothing
    h.api.finishRxSpectrumCapture(); // the new capture is still there to finish
    expect(status(h)).toContain('raw IQ spectra captured');
    expect(h.state.displayIqLease).toBe(false);
    h.api.resetDisplayTransport();
  });

  it('without a lease, old or TX IQ does not make the raw IQ path look ready either', async () => {
    const h = timed({}, '?display_transport=iq');
    h.api.requestDisplayTransport('setup', true);
    h.api.applyDisplayEcho(['0', 'iq']);
    h.sent.length = 0;
    // Only the shared timestamp is fresh (a row would do that): no RX IQ frame was ever decoded.
    h.state.lastFrameAt = Date.now();
    await h.api.startRxSpectrumCapture();
    expect(status(h)).toContain('Start RX IQ on the raw IQ display path');
    h.api.handleBinaryFrame(iqFrame(3));
    await h.api.startRxSpectrumCapture();
    expect(status(h)).toContain('Start RX IQ on the raw IQ display path');
    h.api.handleBinaryFrame(iqFrame(0));
    await h.api.startRxSpectrumCapture();
    expect(status(h)).toContain('Capturing');
    expect(h.sent).toEqual([]);
    await cleanup(h, Promise.resolve());
  });

  it('puts the rows back after a successful capture that began on confirmed raw IQ', async () => {
    const h = timed();
    const pending = h.api.startRxSpectrumCapture();
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await settle(100);
    await pending;
    expect(status(h)).toContain('Capturing');
    h.api.finishRxSpectrumCapture();
    expect(h.state.displayIqLease).toBe(false);
    expect(h.sent).toEqual([IQ, ROWS]);
    h.api.resetDisplayTransport();
  });
});

describe('RX Measure readiness against the complete page', () => {
  afterEach(() => { vi.useRealTimers(); });

  it('no function is declared twice at the top level of the page script', () => {
    // A later declaration silently replaces an earlier one, and the sliced tests above
    // cannot see it. This is how the measurement helper once got overridden by an older
    // function of the same name that checks a different timestamp.
    const seen = new Map<string, number>();
    for (const fn of pageFunctions) seen.set(fn.name, (seen.get(fn.name) ?? 0) + 1);
    const duplicated = [...seen].filter(([, count]) => count > 1).map(([name]) => name);
    expect(duplicated).toEqual([]);
    expect(pageFunctions.length).toBeGreaterThan(300); // the parse really covered the page
  });

  it('the measurement helper and the media-recovery helper are separate, with their own meanings', () => {
    const names = pageFunctions.map((fn) => fn.name);
    expect(names.filter((name) => name === 'rxMeasureIqFresh')).toHaveLength(1);
    expect(names.filter((name) => name === 'rxIqFresh')).toHaveLength(1);
    const recovery = pageFunctions.find((fn) => fn.name === 'rxIqFresh')!.text;
    // Media recovery counts any display frame (rows too) as activity; that must not change.
    expect(recovery).toContain('state.lastFrameAt');
    // The measurement helper looks only at raw RX IQ, never at the shared timestamp.
    const measure = pageFunctions.find((fn) => fn.name === 'rxMeasureIqFresh')!.text;
    expect(measure).toContain('lastRxIqFrameAt');
    expect(measure).toContain('rxIqFrameVersion');
    expect(measure).not.toContain('lastFrameAt');
    // And every RX Measure start check calls it, not the recovery helper.
    const startCheck = pageFunctions.find((fn) => fn.name === 'startRxSpectrumCapture')!.text;
    expect(startCheck).toContain('rxMeasureIqFresh()');
    expect(startCheck).not.toMatch(/\brxIqFresh\(/);
  });

  function plainRawIqPage() {
    vi.useFakeTimers();
    vi.setSystemTime(1000);
    const h = makeHarness({ search: '?display_transport=iq', streamMode: 'lan', measure: {} });
    Object.assign(h.sandbox.window as object, { setTimeout, clearTimeout, setInterval, clearInterval });
    h.sandbox.performance = { now: () => Date.now() };
    return h;
  }
  const statusOf = (h: ReturnType<typeof plainRawIqPage>) => h.elements['rx-spectrum-status']?.textContent ?? '';

  it('with the complete page bound, a spectrum row and an IQ echo alone do not start a capture', async () => {
    const h = plainRawIqPage();
    h.api.handleSpectrumRow(buildRow()); // refreshes the shared lastFrameAt
    h.api.applyDisplayEcho(['0', 'iq']);
    await h.api.startRxSpectrumCapture();
    expect(statusOf(h)).not.toContain('Capturing');
    expect(statusOf(h)).toContain('Start RX IQ on the raw IQ display path');
  });

  it('positive control: with actual fresh raw RX IQ the same page starts a capture', async () => {
    const h = plainRawIqPage();
    h.api.handleSpectrumRow(buildRow());
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0));
    await h.api.startRxSpectrumCapture();
    expect(statusOf(h)).toContain('Capturing');
    h.api.finishRxSpectrumCapture();
    h.api.resetDisplayTransport('cleanup');
  });

  it('RX IQ freshness belongs to the connection: a quick reconnect cannot borrow the previous one', async () => {
    const h = plainRawIqPage();
    h.api.applyDisplayEcho(['0', 'iq']);
    h.api.handleBinaryFrame(iqFrame(0)); // decoded on the first connection
    expect(h.state.rxIqFrameVersion).toBe(1);
    h.api.resetDisplayTransport('socket closed');
    expect(h.state.rxIqFrameVersion).toBe(0);
    expect(h.state.lastRxIqFrameAt).toBe(0);
    h.state.connected = false;
    await vi.advanceTimersByTimeAsync(100);
    // The new connection is up and has asked for the stream, but no data has arrived yet.
    h.state.connected = true;
    h.state.bridgeReady = true;
    h.state.iqStreaming = true;
    h.api.scanDisplayTransportText('saturn_display_caps:spectrum_u8;');
    h.api.applyDisplayEcho(['0', 'iq']);
    await h.api.startRxSpectrumCapture();
    expect(statusOf(h)).not.toContain('Capturing');
    // The new connection's own frame is what makes it ready.
    h.api.handleBinaryFrame(iqFrame(0));
    await h.api.startRxSpectrumCapture();
    expect(statusOf(h)).toContain('Capturing');
    h.api.finishRxSpectrumCapture();
    h.api.resetDisplayTransport('cleanup');
  });
});
