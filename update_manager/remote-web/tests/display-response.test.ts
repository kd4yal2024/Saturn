import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(resolve(process.cwd(), '../templates/saturn-remote-next.html'), 'utf8');
const start = template.indexOf('    function processedDisplayBins(');
const end = template.indexOf('    function clearPeakTuneAssist(', start);
const source = template.slice(start, end);

function display(average: number, bypass: number) {
  const state = {
    spectrumAverage: average,
    spectrumAverageBypassFrames: bypass,
    spectrumPeakHold: false,
    latestBins: new Float32Array(2),
    peakHoldBins: new Float32Array(2),
  };
  const process = runInNewContext(`${source}; processedDisplayBins`, {
    state, Float32Array,
    clampSpectrumAverage: (n: number) => n,
    updatePeakTuneAssist: () => {},
    resetPeakHoldBins: (n: number) => { state.peakHoldBins = new Float32Array(n); },
  }) as (bins: Float32Array) => Float32Array;
  return { state, process };
}

describe('live display averaging', () => {
  it('seeds startup from the first real spectrum before applying the selected average', () => {
    const { process } = display(4, 1);
    expect(Array.from(process(new Float32Array([-100, -80])))).toEqual([-100, -80]);
    expect(Array.from(process(new Float32Array([-60, -40])))).toEqual([-90, -70]);
  });

  it('follows each frame with averaging set to one', () => {
    const { process } = display(1, 0);
    process(new Float32Array([-40, -20]));
    expect(Array.from(process(new Float32Array([-160, -160])))).toEqual([-160, -160]);
  });

  it('reseeds averaging when the FFT/zoom changes the bin count', () => {
    const { process } = display(8, 0);
    expect(Array.from(process(new Float32Array([-100, -80, -60])))).toEqual([-100, -80, -60]);
  });
});

describe('IQ reception and status refresh', () => {
  it('accepts every IQ frame while coalescing routine UI refreshes', () => {
    const handlerStart = template.indexOf('    let lastIqUiRefreshAt =');
    const handlerEnd = template.indexOf('    function renderIdleFrame(', handlerStart);
    const state = {
      sampleRate: 384000, displayIqSource: 'rx', txDisplaySettleFrames: 0,
      lastFrameAt: 0, iqFrameVersion: 0, frameCounter: 0, displayCaption: '',
    };
    let now = 0;
    let refreshes = 0;
    let accepted = 0;
    const handle = runInNewContext(`let displayFirstIqAt = null;
      ${template.slice(handlerStart, handlerEnd)}; handleIqFrame`, {
      state, DataView, Float32Array,
      performance: { now: () => now },
      DISPLAY_STATUS_REFRESH_INTERVAL_MS: 250,
      TX_DISPLAY_SETTLE_SKIP_FRAMES: 0, TX_DISPLAY_SPAN_HZ: 48000,
      displaySampleRateHz: () => state.sampleRate,
      resetDisplayHistory: () => {},
      displayIqForSource: (iq: Float32Array) => iq,
      appendIqPacket: () => { accepted++; },
      logEvent: () => {},
      scheduleUiRefresh: () => { refreshes++; },
    }) as (buffer: ArrayBuffer) => void;
    const frame = new ArrayBuffer(64 + 16 * 4);
    const header = new DataView(frame);
    header.setUint32(4, 384000, true);
    header.setUint32(20, 16, true);
    for (let i = 0; i < 30; i++) {
      now = i * 1000 / 30;
      handle(frame);
    }
    expect(accepted).toBe(30);
    expect(state.frameCounter).toBe(30);
    expect(state.iqFrameVersion).toBe(30);
    expect(refreshes).toBe(4);
    // A rate transition must refresh immediately, within the throttle window.
    header.setUint32(4, 192000, true);
    now += 1;
    handle(frame);
    expect(refreshes).toBe(5);
    expect(accepted).toBe(31);
  });
});
