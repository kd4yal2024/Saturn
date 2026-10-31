import { describe,expect,it,vi } from 'vitest';
import { RfMonitorState,parseRfSpectrum,rfMeasurementText,copyRfMeasurements,type RfSpectrum } from '../src/puresignal/rf-monitor';
import { readFileSync } from 'node:fs';
export function fixture(overrides:Partial<RfSpectrum>={}):RfSpectrum {
  return {format:'saturn-rf-spectrum-v1',sourcePin:'offline-test',sourceBoot:'1',firmwareBuild:'not supplied by this observer',boot:'10',serial:'1',generation:'1',config:'1',txEpoch:'1',backend:'DirectXdma',source:'ReturnedRfIq',
    observedFirstNs:'200000000',observedLastNs:'285000000',publishedNs:'300000000',rateHz:192000,fftSize:16384,binHz:11.71875,rbwHz:23.49,
    window:'Blackman-Harris 4-term',centerHz:7200000,attenuationCommandDb:31,commandedWatts:5,psRequested:false,overload:'unknown',continuity:'contiguous',
    diagnosticDrops:'0',reference:'weaker fundamental; not PEP',rfImdQualified:false,reason:'relative estimate; RF path and overload coverage require verification',
    componentPeak:.4,clippedComponents:0,toneBalanceDb:.1,toneHz:[700,1900],toneDbfs:[-20,-20.1],referenceToneDbfs:[-12,-12],
    products:[-500,3100,-1700,4300,-2900,5500].map((offsetHz,i)=>({order:[3,3,5,5,7,7][i]!,offsetHz,dbc:i<2?-35:null,upperBoundDbc:i>=2?-62:null,reason:'relative estimate'})),
    firstBinHz:-7992.1875,binsDbfs:Array(1365).fill(-100) as number[],...overrides};
}
function send(s:RfMonitorState,r=fixture(),now=400){s.observe(`saturn_ps_spectrum:${JSON.stringify(r).replaceAll(';','\\u003b')};`,1,now);}
function armed(){const s=new RfMonitorState();s.reset(1);s.observe('saturn_ps_spectrum_caps:raw-pairs-v1;',1,80);s.current(90,true,true);s.request('1',1,100);s.observe('saturn_ps_spectrum_clock:1,10,100000000;',1,120);return s;}
describe('read-only RF/IMD measurement contract',()=>{
  it('labels timing in held/copied spectra and rejects non-two-tone IMD numbers',()=>{
    const timing=fixture({stimulus:'saturn-timing-multisine-v1',
      requestedStimulus:{version:'saturn-timing-multisine-v1',seed:0x53465031,pcmRateHz:48000,repeatMs:1000},
      reason:'timing multisine; two-tone IMD unavailable',
      products:fixture().products.map(p=>({...p,dbc:null,upperBoundDbc:null,reason:'two-tone IMD unavailable'}))});
    const parsed=parseRfSpectrum(JSON.stringify(timing));
    const text=rfMeasurementText(parsed);
    expect(text).toContain('Timing Measurement');expect(text).toContain('1000 ms repeat');
    expect(text).toContain('Two-tone IMD unavailable');expect(text).not.toContain('IMD3 lower');
    expect(text).not.toContain('Reference-channel tones');expect(text).toContain('Overload unknown');
    expect(()=>parseRfSpectrum(JSON.stringify({...timing,products:fixture().products}))).toThrow('may not publish IMD');
    expect(()=>parseRfSpectrum(JSON.stringify({...timing,requestedStimulus:undefined}))).toThrow('waveform identity');
    expect(()=>parseRfSpectrum(JSON.stringify({...timing,stimulus:'unknown'}))).toThrow('Unknown RF stimulus');
    const state=armed();send(state,timing);expect(state.saveBaseline(410)).toBe(false);
    state.baseline=fixture();expect(state.baselineMatches(timing)).toBe(false);
  });
  function copyFixture(writeText?: (text:string)=>Promise<void>, legacyCopy=false) {
    const field={value:'',readOnly:false,setAttribute:vi.fn(),style:{cssText:''},focus:vi.fn(),select:vi.fn(),remove:vi.fn()};
    const focus=vi.fn();
    const doc={defaultView:{navigator:{clipboard:writeText?{writeText}:undefined}},activeElement:{focus},createElement:vi.fn(()=>field),execCommand:vi.fn(()=>legacyCopy)};
    const feedback={ownerDocument:doc,textContent:'',querySelector:vi.fn(()=>null),append:vi.fn()};
    const view={status:{textContent:'RX / held capture — not live'},summary:{textContent:rfMeasurementText(fixture())}};
    return {field,feedback,view,doc,focus,run:()=>copyRfMeasurements(view as unknown as Parameters<typeof copyRfMeasurements>[0],feedback as unknown as HTMLElement)};
  }
  it('copies a frozen displayed snapshot and its qualifications, not a later live update',async()=>{
    let finish!:()=>void;
    const write=vi.fn((_text:string)=>new Promise<void>(resolve=>{finish=resolve;}));const f=copyFixture(write);
    const done=f.run();f.view.status.textContent='LIVE RF';f.view.summary.textContent='new frame';finish();await done;
    expect(write.mock.calls[0]?.[0]).toContain('held capture — not live');
    expect(write.mock.calls[0]?.[0]).toContain('Overload unknown');
    expect(write.mock.calls[0]?.[0]).toContain('not calibrated RF IMD');
    expect(write.mock.calls[0]?.[0]).toContain('Copied at ');
    expect(f.feedback.textContent).toBe('Measurements copied.');expect(f.doc.createElement).not.toHaveBeenCalled();
  });
  it('uses the clicked document fallback when modern clipboard rejects',async()=>{
    const f=copyFixture(async()=>{throw new Error('denied');},true);await f.run();
    expect(f.doc.execCommand).toHaveBeenCalledWith('copy');expect(f.field.select).toHaveBeenCalled();
    expect(f.field.remove).toHaveBeenCalled();expect(f.focus).toHaveBeenCalled();expect(f.feedback.textContent).toBe('Measurements copied.');
  });
  it('keeps selected text available and reports manual copy when neither API works',async()=>{
    const f=copyFixture();f.view.status.textContent='WAITING / STALE';await f.run();
    expect(f.field.value).toContain('WAITING / STALE');expect(f.field.value).toContain('-35.0 dBc');
    expect(f.field.select).toHaveBeenCalled();expect(f.field.remove).not.toHaveBeenCalled();
    expect(f.feedback.textContent).toContain('Ctrl+C');expect(f.feedback.textContent).not.toContain('Measurements copied.');
  });
  it('retains manual copy even if the legacy API throws',async()=>{
    const f=copyFixture();f.doc.execCommand.mockImplementation(()=>{throw new Error('unsupported');});await f.run();
    expect(f.field.remove).not.toHaveBeenCalled();expect(f.feedback.textContent).toContain('Clipboard unavailable');
  });
  it('keeps RF source, weaker-tone dBc, original time and unknown overload explicit',()=>{
    const r=parseRfSpectrum(JSON.stringify(fixture()));const t=rfMeasurementText(r);
    expect(t).toContain('Overload unknown');expect(t).toContain('not PEP');expect(t).toContain('-35.0 dBc');expect(t).toContain('floor estimate');
    expect(t).toContain('not calibrated RF IMD');expect(r.rfImdQualified).toBe(false);
  });
  it('rejects digital data, invalid markers, nonfinite values and oversized input',()=>{
    for(const r of [fixture({source:'WdspIq' as 'ReturnedRfIq'}),fixture({observedFirstNs:'999999999'}),fixture({binHz:10}),fixture({rfImdQualified:true as false}),fixture({toneHz:[700,2000]})])expect(()=>parseRfSpectrum(JSON.stringify(r))).toThrow();
    expect(()=>parseRfSpectrum(' '.repeat(24001))).toThrow();
  });
  it('accepts fresh capture, rejects queued stale data and clock exchanges with unbounded RTT',()=>{
    const s=armed();send(s);expect(s.current(400,true,true)).toBe(true);
    send(s,fixture({serial:'2'}),1100);expect(s.note).toContain('Stale');
    expect(s.current(1100,true,true)).toBe(false);
    const slow=new RfMonitorState();slow.current(0,true,true);slow.request('2',1,10);slow.observe('saturn_ps_spectrum_clock:2,10,100000000;',1,900);send(slow);expect(slow.frame).toBeNull();
  });
  it('invalidates timing on consumer stall and rejects delayed old-key output after release/re-arm',()=>{
    const s=armed();send(s);expect(s.current(400,true,true)).toBe(true);
    s.setTransmitting(false);s.current(410,true,false);s.current(420,true,true);
    send(s,fixture({serial:'2'}),430);expect(s.current(430,true,true)).toBe(false);
    s.request('2',1,440);s.observe('saturn_ps_spectrum_clock:2,10,440000000;',1,450);
    send(s,fixture({serial:'3'}),460);expect(s.current(460,true,true)).toBe(false);
    send(s,fixture({serial:'4',observedFirstNs:'500000000',observedLastNs:'585000000',publishedNs:'600000000',txEpoch:'2'}),610);
    expect(s.current(610,true,true)).toBe(true);
    expect(s.current(1500,true,true)).toBe(false);expect(s.note).toContain('stalled');
  });
  it('holds only PS-off baselines, suppresses mismatched overlays and retires old-session packets',()=>{
    const s=armed();send(s);expect(s.saveBaseline()).toBe(true);expect(s.baselineMatches(fixture())).toBe(true);
    expect(s.baselineMatches(fixture({commandedWatts:10}))).toBe(false);expect(s.baselineMatches(fixture({centerHz:14167000}))).toBe(false);
    send(s,fixture({serial:'2',psRequested:true}));expect(s.saveBaseline()).toBe(false);
    s.observe(`saturn_ps_spectrum:${JSON.stringify(fixture({serial:'999'}))};`,2,500);expect(s.frame).toBeNull();
  });
  it('mounts a separate visible canvas, markers and pop-out, without a radio command path',()=>{
    const html=readFileSync(new URL('../../templates/saturn-remote-next.html',import.meta.url),'utf8');
    expect(html).toContain('id="ps-rf-monitor"');expect(html).toContain('data-rf-canvas');expect(html).toContain('Pop out RF / IMD');
    const src=readFileSync(new URL('../src/puresignal/rf-monitor.ts',import.meta.url),'utf8');
    expect(src).not.toContain('sendTci(');expect(src).not.toContain('new WebSocket');
  });
});
