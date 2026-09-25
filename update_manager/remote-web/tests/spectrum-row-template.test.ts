import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';
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
const DRAW_START = '    function drawDisplayBins(bins, now, waterfallEnabled, phoneWanLite, centerShiftHz = 0) {';
const DRAW_END = '    function animationLoop(now) {';

function slice(start: string, end: string): string {
  const from = template.indexOf(start);
  const to = template.indexOf(end, from);
  if (from < 0 || to < 0) throw new Error(`template slice not found: ${start}`);
  return template.slice(from, to);
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
};

function makeHarness(options: {
  streamMode?: string;
  caps?: boolean;
  terrain?: boolean;
  targetFftSize?: number;
  intervalMs?: number;
  echoTimeoutMs?: number;
  dds?: number;
} = {}) {
  const sent: string[] = [];
  const logs: string[] = [];
  const audioFrames: number[] = [];
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
    window: { setTimeout, clearTimeout },
    displayFftTargetSize: () => options.targetFftSize ?? 2048,
    displayRenderIntervalMs: () => options.intervalMs ?? 50,
    DISPLAY_ECHO_TIMEOUT_MS: options.echoTimeoutMs ?? 1000,
    DISPLAY_STATUS_REFRESH_INTERVAL_MS: 250,
    displayFirstIqAt: null,
    lastIqUiRefreshAt: -Infinity,
    TCI_STREAM_AUDIO_RX: 1,
    TCI_STREAM_IQ_TX: 3,
    TCI_STREAM_IQ_RX: 0,
    handleAudioFrame: (buffer: ArrayBuffer) => { audioFrames.push(buffer.byteLength); },
    handleIqFrame: (buffer: ArrayBuffer) => { iqFrames.push(buffer.byteLength); },
    txReceiveSuppressionActive: () => false,
    // Share the host realm's binary types so `instanceof` and typed-array
    // identity work across the vm boundary.
    ArrayBuffer,
    DataView,
    Uint8Array,
    Float32Array,
  };
  const api = runInNewContext(
    `${slice(TRANSPORT_START, TRANSPORT_END)}\n${slice(BINARY_START, BINARY_END)}\n` +
      '({ requestDisplayTransport, applyDisplayEcho, desiredDisplayCommand, iqStartGated, ' +
      'sendIqStart, flushDeferredIqStart, handleSpectrumRow, handleBinaryFrame, scanDisplayTransportText, resetDisplayTransport })',
    sandbox,
  ) as unknown as HarnessApi;
  return { api, sandbox, sent, logs, audioFrames, iqFrames, state: sandbox.state as Record<string, unknown> };
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

  it('routes type 16 through handleBinaryFrame and ignores unknown types', () => {
    const h = makeHarness();
    h.api.handleBinaryFrame(buildRow({ binCount: 256 }));
    expect(h.state.displayServerBins).toBeInstanceOf(Float32Array);
    expect(h.iqFrames).toEqual([]);
    expect(h.audioFrames).toEqual([]);

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
