import { describe, expect, it } from 'vitest';
import { SpectrumHistory, type SpectrumFrame } from '../src/dsp/spectrum-history';
import { normalizeTerrain, referenceColor, fitTerrainRange } from '../src/settings/terrain';
import { normalizeDisplayPrefs, normalizeWaterfallPalette } from '../src/settings/normalize';
import { visibleBinsForDisplay } from '../src/dsp/display';
import { FftProcessor } from '../src/dsp/fft';
import { createDefaultSettingsState } from '../src/settings/defaults';
import { createAppState } from '../src/state/app-state';
import { displayPrefsFromState, radioPrefsFromState } from '../src/state/prefs-from-state';
import { applyDisplayPrefsToState } from '../src/state/apply-prefs';
import { readFileSync } from 'node:fs';
const frame = (timestamp: number, bins: Float32Array = new Float32Array([-120,-50,-100,-110]), sequence = timestamp): SpectrumFrame => ({
  timestamp, bins, sequence, receiver:'rx:0', centerHz:14200000, spanHz:48000, sampleRate:48000, sourceBins:bins.length, units:'relative dB',
});
describe('numeric history and chronology', () => {
  it('owns samples, deduplicates sequences and aggregates only inside timestamp buckets', () => {
    const h = new SpectrumHistory(), f=frame(0);
    expect(h.accept(f,50)).toBe(true); f.bins.fill(0);
    expect(h.row(0)?.[1]).toBe(-50);
    expect(h.accept(f,50)).toBe(false);
    h.accept(frame(25,new Float32Array([-130,-40,-101,-111])),50);
    expect(h.count).toBe(1); expect(h.aggregated).toBe(1); expect(h.row(0)?.[1]).toBe(-40);
    h.accept(frame(50,new Float32Array([-130,-130,-130,-130])),50);
    expect(h.row(0)?.[1]).toBe(-130); expect(h.row(1)?.[1]).toBe(-40);
  });
  it('preserves ring chronology, leaves gaps, and bounds a long hiatus', () => {
    const h=new SpectrumHistory();
    for(let i=0;i<600;i++) h.accept(frame(i*50,new Float32Array([i])),50);
    expect(h.count).toBe(512); expect(h.row(0)?.[0]).toBe(599); expect(h.row(511)?.[0]).toBe(88);
    h.accept(frame(602*50,new Float32Array([602])),50); expect(h.row(1)).toBeNull(); expect(h.row(2)).toBeNull(); expect(h.missing).toBe(2);
    h.accept(frame(99999999,new Float32Array([1000])),50); expect(h.count).toBe(512); expect(h.row(511)).toBeNull();
  });
  it('timestamps decimated WAN display rows at presentation without hiding real IQ stalls', () => {
    const html=readFileSync('../templates/saturn-remote-next.html','utf8');
    expect(html).toContain('acceptSpectrumFrame(visibleBins, now, state.iqFrameVersion, bins.length)');
    const arrivalHistory=new SpectrumHistory(), displayHistory=new SpectrumHistory();
    let latestArrival=-Infinity, version=0, lastDrawnVersion=0, lastDraw=-Infinity;
    // A 30 Hz source and 60 Hz RAF with a 50 ms WAN display gate. Source
    // timestamps skip and repeat 50 ms buckets even though no IQ is lost.
    for(let tick=0;tick<=120;tick++) {
      const now=tick*1000/60;
      if(tick%2===0) { latestArrival=now; version++; }
      if(version!==lastDrawnVersion && now-lastDraw>=49) {
        arrivalHistory.accept(frame(latestArrival,new Float32Array([-120]),version),50);
        displayHistory.accept(frame(now,new Float32Array([-120]),version),50);
        lastDrawnVersion=version; lastDraw=now;
      }
    }
    expect(arrivalHistory.missing).toBeGreaterThan(0);
    expect(arrivalHistory.aggregated).toBeGreaterThan(0);
    expect(displayHistory.missing).toBe(0);
    expect(displayHistory.aggregated).toBe(0);
    // If the source actually stops, hasNewIq prevents rows until it resumes.
    displayHistory.accept(frame(2500,new Float32Array([-120]),version+1),50);
    expect(displayHistory.missing).toBeGreaterThan(0);
    expect(displayHistory.row(1)).toBeNull();
  });
  it('segments every incompatible RF mapping and cadence instead of stretching history', () => {
    for(const delta of [{centerHz:14201000},{spanHz:24000},{sampleRate:96000},{sourceBins:8},{receiver:'tx:0'}]) {
      const h=new SpectrumHistory(); h.accept(frame(0),50); h.accept({...frame(50),...delta},50);
      expect(h.count).toBe(1); expect(h.boundary).toContain('segment');
    }
    const h=new SpectrumHistory(); h.accept(frame(0),50); h.accept(frame(50),100); expect(h.count).toBe(1);
  });
  it('rejects invalid inputs and oversized allocations', () => {
    const h=new SpectrumHistory(); expect(h.accept(frame(NaN),50)).toBe(false);
    expect(h.accept(frame(0,new Float32Array(32768)),50)).toBe(false); expect(h.bytes).toBeLessThan(100000);
  });
  it('preserves narrow carriers during visual reduction without changing samples', () => {
    const h=new SpectrumHistory(), bins=new Float32Array(4096).fill(-120); bins[511]=-20; bins[514]=-30;
    h.accept(frame(0,bins),50); expect(h.peak(0,63,512)).toBe(-20); expect(h.peak(0,64,512)).toBe(-30);
    expect(h.latestRaw[511]).toBe(-20); expect(h.peak(0,63,512,0)).toBe(-20);
  });
  it('keeps smaller FFT zoom bins consistent with the frequency ruler', () => {
    for (const n of [1024,2048,4096]) for (const zoom of [1,2,4,8,16,32]) {
      const bins=Float32Array.from({length:n},(_,i)=>i);
      const visible=visibleBinsForDisplay(bins,zoom)!;
      expect(visible.length).toBe(n/zoom);
      expect(visible[0]).toBe(Math.floor((n-visible.length)/2)+visible.length-1);
      expect(visible[visible.length-1]).toBe(Math.floor((n-visible.length)/2));
    }
  });
  it('uses the existing FFT units and exactly one orientation reversal', () => {
    const n=1024, iq=new Float32Array(2*n);
    for(let i=0;i<n;i++) { iq[2*i]=Math.cos(2*Math.PI*100*i/n); iq[2*i+1]=Math.sin(2*Math.PI*100*i/n); }
    const fft=new FftProcessor(n), raw=visibleBinsForDisplay(fft.transform(iq),1)!;
    const h=new SpectrumHistory(); h.accept(frame(0,raw),50);
    expect(h.latestRaw.indexOf(Math.max(...raw))).toBe(411);
    expect(Math.max(...raw)).toBeCloseTo(20*Math.log10(0.5),1);
  });
});
describe('presentation settings', () => {
  it('does not share mutable 3D defaults between settings instances', () => {
    const a=createDefaultSettingsState(), b=createDefaultSettingsState();
    a.displayPrefs.terrain!.height=.2;
    expect(b.displayPrefs.terrain!.height).toBe(.65);
    expect(createDefaultSettingsState().displayPrefs.terrain!.height).toBe(.65);
  });
  it('defaults old profiles and validates imports', () => {
    expect(normalizeDisplayPrefs({}).terrain?.mode).toBe('traditional');
    const s=normalizeTerrain({height:Infinity, floor:19, ceiling:-100, depth:-9, gamma:NaN,mode:'broken',quality:'ultra'});
    expect(s.height).toBe(.65); expect(s.ceiling).toBe(20); expect(s.depth).toBe(8); expect(s.gamma).toBe(.85); expect(s.quality).toBe('auto');
  });
  it('round trips without changing Traditional or radio preferences', () => {
    const state=createAppState(), before=radioPrefsFromState(state), palette=state.waterfallPalette;
    state.terrain=normalizeTerrain({mode:'3d',gamma:1.4,cleanup:.8,waterfallCleanup:.95});
    const prefs=displayPrefsFromState(state); applyDisplayPrefsToState(prefs,state);
    expect(state.terrain?.gamma).toBe(1.4); expect(state.terrain?.cleanup).toBe(.8);
    expect(state.terrain?.waterfallCleanup).toBe(.95);
    expect(state.waterfallPalette).toBe(palette); expect(radioPrefsFromState(state)).toEqual(before);
  });
  it('adds a continuous palette without altering existing choices', () => {
    expect(normalizeWaterfallPalette('reference')).toBe('reference'); expect(referenceColor(0)).toEqual([1,3,12]); expect(referenceColor(1)).toEqual([255,255,255]);
    for(let i=1;i<=1000;i++) expect(Math.max(...referenceColor(i/1000).map((v,c)=>Math.abs(v-referenceColor((i-1)/1000)[c]!)))).toBeLessThanOrEqual(6);
  });
  it('integrates before display-only peak hold, not in the offline animation', () => {
    const html=readFileSync('../templates/saturn-remote-next.html','utf8');
    const idle=html.slice(html.indexOf('function renderIdleFrame'),html.indexOf('function animationLoop'));
    expect(idle).not.toContain('acceptSpectrumFrame');
    const live=html.slice(html.indexOf('const bins = fftProcessor.transform(iqWindow)'));
    expect(live.indexOf('acceptSpectrumFrame')).toBeLessThan(live.indexOf('processedDisplayBins'));
    const selector=html.slice(html.indexOf('function setTerrainMode'),html.indexOf('function saveTerrainSettings'));
    expect(selector).not.toMatch(/sendTci|setFrequency|connect\(|moxRequested\s*=/);
  });
});


describe('manual 3D range fit', () => {
  it('keeps background subdued and weak signals visible without mutating levels', () => {
    const bins = new Float32Array(4096).fill(-112); bins[10] = -90; bins[11] = -30;
    const before = bins.slice(), range = fitTerrainRange(bins)!;
    expect(range).toEqual({floor: -118, ceiling: -73, gamma: .85});
    expect(bins).toEqual(before);
    expect((bins[10]! - range.floor)/(range.ceiling-range.floor)).toBeGreaterThan(.5);
    const quiet = fitTerrainRange(new Float32Array(100).fill(-140))!;
    expect(quiet.ceiling - quiet.floor).toBe(45);
  });
  it('ignores missing/nonfinite values and refuses a fit without measurements', () => {
    expect(fitTerrainRange(new Float32Array([-1000, NaN, Infinity]))).toBeNull();
    expect(fitTerrainRange(new Float32Array([-1000, NaN, -112]))).toEqual({floor: -118, ceiling: -73, gamma: .85});
  });
});


describe('3D amplitude diagnostics and geometry contract', () => {
  it('reports signed raw levels and clipping without changing samples', async () => {
    const {levelStatistics}=await import('../src/dsp/spectrum-history');
    const raw=new Float32Array([-150,-140,-120,-100,-40,-30,NaN,-1000]);
    const before=raw.slice();
    expect(levelStatistics(raw,-140,-40)).toEqual({samples:6,min:-150,median:-120,p95:-40,max:-30,
      normalizedMedian:.2,clampedNormalizedMedian:.2,clippedFloorPercent:100/3,clippedCeilingPercent:100/3});
    expect(raw).toEqual(before);
    expect(levelStatistics(new Float32Array([-1000,NaN]),-140,-40)).toBeNull();
    expect(normalizeTerrain({gridOpacity:4}).gridOpacity).toBe(1);
    expect(normalizeTerrain({gridOpacity:NaN}).gridOpacity).toBe(0);
  });
  it('uses high precision scalar sampling and an outline without skirts', () => {
    const source=readFileSync('src/render/terrain.ts','utf8');
    expect(source.match(/uniform highp sampler2D levels;/g)).toHaveLength(2);
    expect(source).toContain('gl.drawArrays(gl.LINE_STRIP, 0, this.columns)');
    expect(source).not.toContain('skirt');
    expect(source).not.toContain('readPixels');
  });
});


describe('Reference Blue / Rainbow presentation', () => {
  it('retains the previous dark palette exactly while lifting low-level blue detail', () => {
    expect(referenceColor(.12,'reference-dark')).toEqual([6,13,24]);
    expect(referenceColor(.3,'reference-dark')).toEqual([24,48,80]);
    expect(referenceColor(.62,'reference-dark')).toEqual([75,160,102]);
    const background=referenceColor(.06), dark=referenceColor(.06,'reference-dark');
    expect(background[2]).toBeGreaterThan(dark[2]!+70);
    expect(background[2]).toBeGreaterThan(background[0]! * 10);
    expect(referenceColor(.06)[0]).toBeLessThan(10);
    expect(referenceColor(.28)[2]).toBe(245);
    expect(normalizeTerrain({palette:'reference-dark'}).palette).toBe('reference-dark');
    expect(normalizeWaterfallPalette('reference-dark')).toBe('reference-dark');
  });
});
