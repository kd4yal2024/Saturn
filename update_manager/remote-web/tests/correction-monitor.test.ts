import {describe,it,expect} from 'vitest';
import {readFileSync} from 'node:fs';
import {parseCorrection,phaseDegrees,wrapPhase,correctionSummary,CorrectionMonitorState,type CorrectionFrame} from '../src/puresignal/correction-monitor';
export function correctionFixture(overrides:Partial<CorrectionFrame>={}):CorrectionFrame {
  return {format:'saturn-ps-correction-v1',boot:'10',serial:'1',snapshotNs:'250000000',publishedNs:'300000000',sourcePin:'synthetic-not-rf',backend:'DirectXdma',
    centerHz:7200000,commandedWatts:5,attenuationCommandDb:31,psRequested:true,keyed:true,nativeApplied:true,calibrations:1,nativeState:7,
    nativeSamples:4096,phaseReferenceDeg:10,diagnosticDrops:'3',curveAge:'unknown',rfImdQualified:false,
    measured:Array.from({length:1024},(_,i)=>{const x=i/1023;return[x,1+.1*x*x,Math.cos((10+5*x)*Math.PI/180),Math.sin((10+5*x)*Math.PI/180)];}),
    correction:Array.from({length:512},(_,i)=>[i/511,1+.1*(i/511)**2,i/511,5*(i/511)]),...overrides};
}
const wire=(r:CorrectionFrame)=>`saturn_ps_correction:${JSON.stringify(r)};`;
function ready(){const m=new CorrectionMonitorState();m.reset(1);m.observe('saturn_ps_correction_caps:wdsp-ampview-v1;',1,50);m.current(80,true,true);m.request('1',1,100);m.observe('saturn_ps_spectrum_clock:1,10,100000000;',1,120);return m;}
describe('WDSP correction / phase monitor',()=>{
  it('preserves dimensionless native diagnostics and rejects inconsistent or malformed arrays',()=>{
    const nativeInfo=Array(16).fill(0);nativeInfo[4]=152;nativeInfo[5]=1;nativeInfo[6]=2;nativeInfo[15]=7;
    expect(parseCorrection(JSON.stringify(correctionFixture({nativeInfo}))).nativeInfo).toEqual(nativeInfo);
    for(const info of [[],nativeInfo.slice(1),[...nativeInfo,0],nativeInfo.map((n,i)=>i===4?NaN:n),nativeInfo.map((n,i)=>i===5?99:n)])
      expect(()=>parseCorrection(JSON.stringify(correctionFixture({nativeInfo:info})))).toThrow();
  });
  it('uses native magnitude ratios and degree-valued phase; subtracts reference and wraps',()=>{
    const r=parseCorrection(JSON.stringify(correctionFixture()));expect(r.correction).toHaveLength(512);
    expect(phaseDegrees(0,1,10)).toBeCloseTo(80);expect(wrapPhase(190)).toBe(-170);expect(wrapPhase(-190)).toBe(170);
    expect(phaseDegrees(Math.cos(-179*Math.PI/180),Math.sin(-179*Math.PI/180),5)).toBeCloseTo(176);
    const text=correctionSummary(r);expect(text).toContain('inverse-gain convention');expect(text).toContain('Curve age/configuration and overload coverage unknown');expect(text).toContain('1024/4096');
  });
  it('represents no fit with empty arrays, not invented unity or zero-phase curves',()=>{
    const r=correctionFixture({nativeSamples:0,measured:[],correction:[],nativeApplied:false,calibrations:0});
    expect(parseCorrection(JSON.stringify(r)).correction).toEqual([]);
    expect(()=>parseCorrection(JSON.stringify({...r,correction:correctionFixture().correction}))).toThrow();
  });
  it('rejects malformed counts, unordered curves, wrong sources and oversized frames',()=>{
    for(const changes of [{format:'rf-spectrum'},{nativeSamples:4097},{curveAge:'fresh'},{rfImdQualified:true},{snapshotNs:'999999999'},{measured:[[0,1,0,1]]},{psRequested:'true'},{centerHz:0}])
      expect(()=>parseCorrection(JSON.stringify({...correctionFixture(),...changes}))).toThrow();
    const r=correctionFixture();r.correction[3]![0]=.9;expect(()=>parseCorrection(JSON.stringify(r))).toThrow();
    expect(()=>parseCorrection(' '.repeat(160001))).toThrow();
  });
  it('does not mistake a fresh snapshot for a recent fit or a PS enable request',()=>{
    const m=ready();m.observe(wire(correctionFixture({nativeApplied:false,psRequested:true})),1,400);
    expect(m.current(400,true,true)).toBe(true);expect(m.frame?.nativeApplied).toBe(false);expect(m.frame?.curveAge).toBe('unknown');
  });
  it('rejects queued old snapshots, replayed serials and another host boot',()=>{
    const m=ready();m.observe(wire(correctionFixture()),1,400);m.observe(wire(correctionFixture({serial:'0'})),1,450);expect(m.frame?.serial).toBe('1');
    m.observe(wire(correctionFixture({serial:'2',boot:'11'})),1,460);expect(m.frame?.serial).toBe('1');
    m.observe(wire(correctionFixture({serial:'3'})),1,1600);expect(m.frame?.serial).toBe('1');expect(m.note).toContain('Delayed');
  });
  it('invalidates timing across a stalled consumer and disconnect/reconnect',()=>{
    const m=ready();m.observe(wire(correctionFixture()),1,400);expect(m.current(400,true,true)).toBe(true);
    expect(m.current(1800,true,true)).toBe(false);expect(m.note).toContain('stalled');
    m.request('2',2,1810);expect(m.frame).toBeNull();expect(m.capability).toBe(false);expect(m.current(1820,false,false)).toBe(false);
  });
  it('requires a post-key clock barrier before labeling a new TX live',()=>{
    const m=ready();m.observe(wire(correctionFixture()),1,400);expect(m.current(400,true,true)).toBe(true);
    m.setTransmitting(false);m.current(410,true,false);expect(m.current(420,true,true)).toBe(false);
    m.request('2',1,440);m.observe('saturn_ps_spectrum_clock:2,10,440000000;',1,450);
    m.observe(wire(correctionFixture({serial:'2'})),1,460);expect(m.current(460,true,true)).toBe(false);
    m.observe(wire(correctionFixture({serial:'3',snapshotNs:'500000000',publishedNs:'510000000'})),1,520);expect(m.current(520,true,true)).toBe(true);
  });
  it('does not accept an unbounded timing exchange',()=>{
    const m=new CorrectionMonitorState();m.request('1',1,100);m.observe('saturn_ps_spectrum_clock:1,10,100000000;',1,800);m.observe(wire(correctionFixture()),1,810);expect(m.frame).toBeNull();
  });
  it('holds the last keyed curve through release, native cleanup and a long RX inspection',()=>{
    const m=ready();m.observe(wire(correctionFixture()),1,400);
    // Release can arrive before another paint. A fresh empty cleanup readback
    // remains authoritative for PS status, but must not erase the plots.
    m.setTransmitting(false);
    m.observe(wire(correctionFixture({serial:'2',snapshotNs:'450000000',publishedNs:'460000000',
      keyed:false,nativeApplied:false,nativeSamples:0,calibrations:0,nativeState:0,measured:[],correction:[]})),1,470);
    const display=m.display(480,true,false);
    expect(display.live).toBe(false);expect(display.label).toContain('RX / held last valid TX curve');
    expect(display.frame?.serial).toBe('1');expect(display.frame?.correction).toHaveLength(512);
    expect(m.status(1,480,true,false)?.nativeApplied).toBe(false);
    expect(m.status(1,480,true,false)?.nativeSamples).toBe(0);
    expect(m.display(60_000,true,false).frame).toBe(display.frame);
    expect(m.display(60_010,false,false).label).toContain('DISCONNECTED / held');
    expect(correctionSummary(display.frame!)).toContain('At snapshot');
  });
  it('does not overwrite held TX results with a later RX curve or admit a delayed frame',()=>{
    const m=ready();m.observe(wire(correctionFixture()),1,400);m.setTransmitting(false);
    m.observe(wire(correctionFixture({serial:'2',keyed:false,centerHz:14_200_000})),1,450);
    expect(m.frame?.centerHz).toBe(14_200_000);
    expect(m.display(460,true,false).frame?.centerHz).toBe(7_200_000);
    m.observe(wire(correctionFixture({serial:'3',nativeSamples:0,measured:[],correction:[]})),1,1800);
    expect(m.display(1800,true,false).frame?.serial).toBe('1');
  });
  it('replaces held results only with a fresh curve from the next keyed interval',()=>{
    const m=ready();m.observe(wire(correctionFixture()),1,400);m.setTransmitting(false);m.current(410,true,false);
    expect(m.display(420,true,true).live).toBe(false);
    m.request('2',1,440);m.observe('saturn_ps_spectrum_clock:2,10,440000000;',1,450);
    m.observe(wire(correctionFixture({serial:'2',snapshotNs:'500000000',publishedNs:'510000000',
      nativeSamples:0,measured:[],correction:[],nativeApplied:false,calibrations:0,nativeState:0})),1,520);
    const empty=m.display(520,true,true);
    expect(empty.frame?.nativeSamples).toBe(0);expect(empty.label).toContain('NO CALIBRATION CURVE');
    m.observe(wire(correctionFixture({serial:'3',snapshotNs:'530000000',publishedNs:'540000000',calibrations:2})),1,550);
    expect(m.display(550,true,true).live).toBe(true);
    m.setTransmitting(false);
    expect(m.display(560,true,false).frame?.serial).toBe('3');
    expect(m.display(560,true,false).frame?.calibrations).toBe(2);
  });
  it('never promotes a pre-key curve to held TX evidence and clears history on reconnection',()=>{
    const m=new CorrectionMonitorState();m.request('1',1,100);
    m.observe('saturn_ps_correction_caps:wdsp-ampview-v1;saturn_ps_spectrum_clock:1,10,100000000;',1,120);
    m.observe(wire(correctionFixture()),1,400);m.current(410,true,true);
    m.setTransmitting(false);
    m.observe(wire(correctionFixture({serial:'2',nativeSamples:0,measured:[],correction:[],keyed:false})),1,420);
    expect(m.display(430,true,false).frame?.nativeSamples).toBe(0);
    const good=ready();good.observe(wire(correctionFixture()),1,400);good.setTransmitting(false);
    expect(good.display(410,true,false).frame?.correction).toHaveLength(512);
    good.reset(2);expect(good.display(420,true,false).frame).toBeNull();
  });
  it('has separate phase canvas/pop-out/copy, without a transmit or calibration command route',()=>{
    const src=readFileSync(new URL('../src/puresignal/correction-monitor.ts',import.meta.url),'utf8');
    expect(src).not.toContain('sendTci(');expect(src).not.toContain('new WebSocket');
    const html=readFileSync(new URL('../../templates/saturn-remote-next.html',import.meta.url),'utf8');
    expect(html).toContain('data-correction-phase');expect(html).toContain('Pop out Correction / Phase');expect(html).toContain('data-correction-copy');
  });
});
