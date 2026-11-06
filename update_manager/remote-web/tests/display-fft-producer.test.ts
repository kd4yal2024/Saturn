import { describe,it,expect,vi } from 'vitest';
vi.mock('../src/display/fft-worker?worker&inline',()=>({default:class{constructor(){throw Error('inject a port')}}}));
import { DisplayFftProducer,type WorkerPort } from '../src/display/fft-producer';
import { FftProcessor } from '../src/dsp/fft';
import type { DisplayRequest,FftJob,FftReply } from '../src/display/fft-protocol';
import { planDisplayIqWindow } from '../src/display/iq-window';

function rig() {
  let now=100,id=0;
  const timers=new Map<number,{at:number;run:()=>void}>();
  const sent:FftJob[]=[],mainBuffers:ArrayBuffer[]=[];
  let terminated=false;
  const port:WorkerPort={onmessage:null,onerror:null,terminate(){terminated=true},postMessage(job,transfer){
    mainBuffers.push(...transfer);
    sent.push(structuredClone(job,{transfer}));
  }};
  const producer=new DisplayFftProducer({createWorker:()=>port,now:()=>now,
    schedule:(run,delay)=>{timers.set(++id,{at:now+delay,run});return id},cancel:id=>{timers.delete(id)}});
  function advance(ms:number){now+=ms;for(const [id,t] of [...timers])if(t.at<=now){timers.delete(id);t.run()}}
  function reply(error?:string) {
    const job=sent.shift()!;
    const bins=new FftProcessor(job.fftSize).transform(new Float32Array(job.input,0,job.floatCount));
    new Float32Array(job.output,0,job.fftSize).set(bins);
    const result:FftReply={...job,kind:'result',fftMs:2,...(error?{error}:{})};
    const transferred=structuredClone(result,{transfer:[job.input,job.output]});
    port.onmessage?.({data:transferred} as MessageEvent<FftReply>);
    expect(job.input.byteLength).toBe(0);expect(job.output.byteLength).toBe(0);
    return bins;
  }
  function request(sequence=1,key='rx:14200000'):DisplayRequest {
    const input=new Float32Array(128);for(let i=0;i<64;i++){input[2*i]=Math.cos(i*Math.PI/4);input[2*i+1]=Math.sin(i*Math.PI/4)}
    return {key,sequence,fftSize:64,floatCount:128,newestIngressMs:now,oldestIngressMs:now-10,intervalMs:0,
      write:destination=>{destination.set(input);return input.length}};
  }
  return {producer,port,sent,mainBuffers,advance,reply,request,timers,isTerminated:()=>terminated};
}

describe('bounded display FFT ownership',()=>{
  it('transfers and returns both buffers; renders identical FFT without detaching raw input',()=>{
    const r=rig(),req=r.request();r.producer.request(req);
    expect(r.mainBuffers.map(b=>b.byteLength)).toEqual([0,0]);
    const expected=r.reply(),latest=r.producer.latest()!;
    expect(latest.bins).toEqual(expected);expect(latest.sequence).toBe(1);
    r.producer.drawn(latest,130);r.producer.drawn(latest,140);
    expect(r.producer.diagnostics()).toMatchObject({drawn:1,slotCount:2,ready:1,freeSlots:1,retainedTransferCapacityBytes:393216,backingAllocations:4});
  });
  it('bounds an arbitrarily slow worker to one flight and newest pending metadata',()=>{
    const r=rig();r.producer.request(r.request());
    for(let seq=2;seq<=1000;seq++)r.producer.request(r.request(seq));
    expect(r.sent).toHaveLength(1);
    expect(r.producer.diagnostics()).toMatchObject({inFlight:1,pending:1,coalesced:998,queueHighWater:1});
    r.reply();expect(r.sent).toHaveLength(1);
    expect(r.producer.latest()?.sequence).toBe(1); // no starvation while newest computes
    r.reply();expect(r.producer.latest()?.sequence).toBe(1000);
    expect(r.producer.diagnostics()).toMatchObject({submitted:2,backingAllocations:4,pending:0});
  });
  it('does not reuse the latest spectrum slot while the other slot is in flight',()=>{
    const r=rig();r.producer.request(r.request());r.reply();
    const latest=r.producer.latest()!,copy=latest.bins.slice();
    r.producer.request(r.request(2));expect(latest.bins.byteLength).toBe(256);expect(latest.bins).toEqual(copy);
    r.reply();r.producer.request(r.request(3));
    expect(latest.bins.byteLength).toBe(0); // documented borrowed lifetime ended on reply
    r.reply();expect(r.producer.diagnostics()).toMatchObject({backingAllocations:4,completed:3});
  });
  it('invalidates old tuning immediately without needing another packet',()=>{
    const r=rig();r.producer.request(r.request());r.producer.context('rx:7200000');r.reply();
    expect(r.producer.latest()).toBeNull();expect(r.producer.diagnostics()).toMatchObject({obsolete:1,freeSlots:2});
    r.producer.request(r.request(2,'rx:7200000'));r.reply();expect(r.producer.latest()?.sequence).toBe(2);
  });
  it('snapshots pending metadata and preserves its original window age',()=>{
    const r=rig();r.producer.request(r.request());const pending=r.request(2);r.producer.request(pending);
    pending.sequence=999;pending.newestIngressMs=999;r.advance(50);r.reply();r.reply();
    const latest=r.producer.latest()!;expect(latest.sequence).toBe(2);expect(latest.newestIngressMs).toBe(100);
    r.producer.drawn(latest,200);expect(r.producer.diagnostics()).toMatchObject({newestIngressToDrawMs:{max:100},oldestIngressToDrawMs:{max:110}});
  });
  it('schedules production independently of redraw and coalesces while rate limited',()=>{
    const r=rig(),first=r.request();first.intervalMs=33;r.producer.request(first);r.reply();
    for(let i=2;i<=9;i++){const req=r.request(i);req.intervalMs=33;r.producer.request(req)}
    expect(r.sent).toHaveLength(0);r.advance(33);expect(r.sent).toHaveLength(1);r.reply();
    expect(r.producer.latest()?.sequence).toBe(9);expect(r.producer.diagnostics()).toMatchObject({drawn:0,submitted:2});
  });
  it('recycles stale work before dispatching the new generation',()=>{
    const r=rig();r.producer.request(r.request());r.producer.request(r.request(2,'tx:7200000'));
    r.reply();expect(r.producer.latest()).toBeNull();r.reply();expect(r.producer.latest()?.sequence).toBe(2);
  });
  it('times out and terminates a stopped worker with an explicit main fallback',()=>{
    const r=rig();r.producer.request(r.request());r.advance(2001);
    expect(r.isTerminated()).toBe(true);expect(r.producer.available).toBe(false);
    expect(r.producer.diagnostics()).toMatchObject({engine:'main',slotCount:0,inFlight:0,pending:0});
    expect(r.producer.fallbackReason).toContain('deadline');
  });
  it('fails closed on malformed or error replies and does not strand pending timers',()=>{
    for(const kind of ['malformed','error','worker-error']){
      const r=rig();r.producer.request(r.request());
      if(kind==='error')r.reply('bad input');
      else if(kind==='worker-error')r.port.onerror?.({} as ErrorEvent);
      else r.port.onmessage?.({data:{kind:'result',id:999}} as MessageEvent<FftReply>);
      expect(r.producer.available).toBe(false);expect(r.isTerminated()).toBe(true);expect(r.timers.size).toBe(0);
    }
  });
  it('explicit main selection never creates or transfers worker buffers',()=>{
    const factory=vi.fn();const producer=new DisplayFftProducer({enabled:false,createWorker:factory});
    expect(producer.request(rig().request())).toBe(false);expect(factory).not.toHaveBeenCalled();
  });
  it('disposes all work; late callbacks cannot publish',()=>{
    const r=rig();r.producer.request(r.request());r.producer.request(r.request(2));
    r.producer.dispose();r.reply();expect(r.producer.latest()).toBeNull();expect(r.timers.size).toBe(0);expect(r.isTerminated()).toBe(true);
  });
  it('rejects unsupported sizes without silently reducing resolution',()=>{
    const r=rig();expect(r.producer.request({...r.request(),fftSize:32768,floatCount:65536})).toBe(false);
    expect(r.producer.fallbackReason).toContain('Unsupported');expect(r.sent).toHaveLength(0);
  });
});

describe('display IQ provenance',()=>{
  it('retains packet times and exact tail order when fresh packets replace history',()=>{
    const old=Float32Array.of(1,2,3,4,5,6),fresh=Float32Array.of(7,8,9,10),packets=[old,fresh];
    const plan=planDisplayIqWindow(packets,4,false,p=>p===old?10:100)!;
    packets.splice(0,2,Float32Array.of(99,99));
    const dest=new Float32Array(8);expect(plan.write(dest)).toBe(8);expect([...dest]).toEqual([3,4,5,6,7,8,9,10]);
    expect(plan).toMatchObject({oldestIngressMs:10,newestIngressMs:100});expect(old.byteLength).toBe(24);
  });
  it('never concatenates TX with previous RX or TX packets',()=>{
    const packets=[new Float32Array(100),Float32Array.of(1,2,3,4)];
    const plan=planDisplayIqWindow(packets,16384,true,()=>50)!;
    const dest=new Float32Array(4);plan.write(dest);expect([...dest]).toEqual([1,2,3,4]);expect(plan.floatCount).toBe(4);
  });
});
