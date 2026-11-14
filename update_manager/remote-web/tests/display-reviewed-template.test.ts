import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';
import { visibleBinsForDisplay, shiftBinsHorizontally } from '../src/dsp/display';

const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
function section(start: string, end: string) {
  const a = template.indexOf(start), b = template.indexOf(end, a);
  if (a < 0 || b <= a) throw Error('Display test source boundary missing');
  return template.slice(a, b);
}

describe('retained display averaging and orientation', () => {
  function averaging() {
    const state: any = { spectrumAverage: 2, spectrumAverageBypassFrames: 0,
      spectrumPeakHold: false, latestBins: Float32Array.of(-100, -120), peakHoldBins: null };
    const ctx: any = { state, Float32Array, clampSpectrumAverage: (x: number) => x,
      updatePeakTuneAssist() {}, resetPeakHoldBins: (n: number) => { state.peakHoldBins = new Float32Array(n).fill(-Infinity); } };
    runInNewContext(section('    function processedDisplayBins(', '    function clearPeakTuneAssist(') + ';this.process=processedDisplayBins;', ctx);
    return ctx;
  }
  it('reuses display storage while preserving temporal averages and borrowed inputs', () => {
    const c = averaging(), retained = c.state.latestBins, input = Float32Array.of(-80, -100);
    expect(c.process(input)).toBe(retained);
    expect([...retained]).toEqual([-90, -110]);
    c.process(Float32Array.of(-100, -80));
    expect([...retained]).toEqual([-95, -95]);
    expect([...input]).toEqual([-80, -100]);
    c.state.spectrumAverageBypassFrames = 1;
    expect([...c.process(input)]).toEqual([-80, -100]);
    expect(c.state.spectrumAverageBypassFrames).toBe(0);
  });
  it('retains peak decay and allocates safe storage for overlap/size changes', () => {
    const c = averaging(); c.state.spectrumAverage = 1; c.state.spectrumPeakHold = true;
    c.process(Float32Array.of(-80, -100));
    expect([...c.process(Float32Array.of(-90, -110))]).toEqual([...Float32Array.of(-80.35, -100.35)]);
    c.state.spectrumPeakHold = false;
    const input = c.state.latestBins;
    expect(c.process(input)).not.toBe(input);
    expect([...input]).toEqual([...Float32Array.of(-80.35, -100.35)]);
    expect(c.process(Float32Array.of(-20))).toHaveLength(1);
  });
  it('lets the real helper size template scratch without a duplicate count rule', () => {
    const c: any = { Float32Array, zoom: 1, effectiveDisplayZoom: () => c.zoom,
      _visibleBinsForDisplay: visibleBinsForDisplay };
    runInNewContext(section('    let visibleDisplayScratch =', '    function shiftBinsHorizontally(') + ';this.orient=visibleBinsForDisplay;', c);
    for (const [size, zoom, width] of [[1, 1, 1], [32, 1000, 2], [4096, 5, 819], [16384, 2, 8192]]) {
      c.zoom = zoom;
      const input = Float32Array.from({ length: size! }, (_, i) => i);
      const out = c.orient(input);
      expect(out).toHaveLength(width!);
      expect(out).toEqual(visibleBinsForDisplay(input, zoom!));
      expect(c.orient(input)).toBe(out);
      expect(input[0]).toBe(0);
    }
  });
  it('reuses separate recentering storage and rejects input overlap', () => {
    const input = Float32Array.of(1, 2, 3, 4), scratch = new Float32Array(4);
    for (const shift of [-9, -2, 1, 9]) {
      expect(shiftBinsHorizontally(input, shift, -160, scratch)).toBe(scratch);
      expect(scratch).toEqual(shiftBinsHorizontally(input, shift, -160));
    }
    expect(shiftBinsHorizontally(input, 1, -160, input)).not.toBe(input);
    expect([...input]).toEqual([1, 2, 3, 4]);
  });
});

describe('server rows have one producer cadence and one draw per new row', () => {
  it('draws the latest row on the next RAF at every phase, without drawing duplicates', () => {
    const state: any = { connected: true, iqStreaming: true, iqPackets: [], displayRenderSource: 'server',
      displayServerBins: new Float32Array(256), displayServerRowVersion: 1,
      lastRenderedServerRowVersion: 0, lastSpectrumRenderAt: 1000, displayServerCenterHz: 14200000,
      dds: 14200000, displayIqSource: 'rx' };
    let draws = 0;
    const c: any = { state, browserTimingLastRafAt: 1000, recordRafTiming() {}, updateMeterVisualState() {},
      phoneWanLiteActive: () => false, waterfallRenderEnabled: () => true, txReceiveSuppressionActive: () => false,
      displayFftProducer: { context() {}, available: false }, displayWorkerKey: () => '',
      displayRenderIntervalMs: () => 100, performance: { now: () => 1001 },
      displayTimingBuckets: { drawCpu: [], arrivalToDraw: [], terrainCpu: [], metersCpu: [] },
      recordBrowserTimingSample() {}, recordDisplaySlowFrame() {}, displayFirstIqToDrawMs: 1, displayFirstIqAt: 1,
      drawDisplayBins: (_: unknown, now: number) => { draws++; state.lastSpectrumRenderAt = now; return now; },
      waterfallRenderer: { render() {} }, renderTerrain() {}, audioLeadMs() {}, updateInstrumentMultimeter() {}, updateTxMiniMeters() {},
      requestAnimationFrame() {}, $: () => ({}), safeFixed: () => '',
      renderIdleFrame() {}, resetDisplayHistory() {} };
    runInNewContext(section('    function animationLoop(', '    function pulseTelemetry(') + ';this.frame=animationLoop;', c);
    c.frame(1001); expect(draws).toBe(1); // 1 ms after prior draw, not 100 ms
    c.frame(1017); expect(draws).toBe(1);
    for (const phase of [1, 16, 50, 99]) {
      state.displayServerRowVersion++;
      c.frame(state.lastSpectrumRenderAt + phase);
    }
    expect(draws).toBe(5);
    state.displayPaused = true; state.displayServerRowVersion++; c.frame(2000);
    expect(draws).toBe(5);
  });
});
