#!/usr/bin/env node
// Compiled production worker + actual Remote page. Synthetic local IQ only.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFileSync,writeFileSync,mkdtempSync,mkdirSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname,resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromeSession,pause} from './lib/chrome-session.mjs';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const output=process.argv.find(a=>a.startsWith('--output='))?.slice(9)||mkdtempSync(join(tmpdir(),'saturn-worker-validation-'));
mkdirSync(output,{recursive:true});
const bundle=readFileSync(resolve(root,'dist/saturn-remote-next.js'));
const hooks=`
window.__displayTest={state,producer:displayFftProducer,offerDisplayFft,resetDisplayHistory,shiftDisplayForCenterChange,
 resetDisplayTransport,animationLoop,displayPipelineSnapshot,fft:()=>fftProcessor,
 measurement(on){rxSpectrumAccumulator=on?{shouldSample:()=>true,addFrame:bins=>window.__measurementBins=bins.slice()}:null;},
 frame(stream=0,negative=false){const n=4096,b=new ArrayBuffer(64+n*8),v=new DataView(b),iq=new Float32Array(b,64);
 v.setUint32(4,stream===1?192000:384000,true);v.setUint32(20,n*2,true);v.setUint32(24,stream,true);
 for(let i=0;i<n;i++){const p=i*2*Math.PI*100/n;iq[i*2]=.5*Math.cos(p);iq[i*2+1]=.5*Math.sin(negative?-p:p)}
 window.__rawPacket=b;handleIqFrame(b,stream===1?"tx":"rx");return iq.byteLength;},
 expected(){const iq=new Float32Array(window.__rawPacket,64);return new FftProcessor(4096).transform(iq)},
 expectedMeasurement(){return Array.from(_visibleBinsForDisplay(this.expected(),1));},
 inspect(){const r=displayFftProducer.latest();return {diag:displayFftProducer.diagnostics(),sequence:r?.sequence,
 bins:r?Array.from(r.bins):null,rawBytes:window.__rawPacket?.byteLength,rendered:state.lastRenderedIqFrameVersion,
 measurement:window.__measurementBins?Array.from(window.__measurementBins):null,settings:rxSpectrumCurrentSettings()};}
};
`;
const template=readFileSync(resolve(root,'../templates/saturn-remote-next.html'),'utf8');
let html=template.replace('window.saturnGetRxLatencyText = rxLatencyDiagnosticText;','window.saturnGetRxLatencyText = rxLatencyDiagnosticText;'+hooks);
html=html.replace('<head>',`<head><script>localStorage.clear();window.__socket={readyState:1,bufferedAmount:0,send(){},close(){}};if(location.search.includes('block_worker'))window.Worker=function(){throw Error('fixture CSP blocks Worker')};</script>`);
const server=createServer((req,res)=>{
 if(req.url?.startsWith('/remote-assets/remote-next.js')){res.setHeader('Content-Type','text/javascript');res.end(bundle)}
 else if(req.url==='/'||req.url?.startsWith('/?')){res.setHeader('Content-Type','text/html');res.end(html)}
 else{res.writeHead(404);res.end('offline fixture')}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const checks=[];const check=(name,condition)=>{assert.ok(condition,name);checks.push(name)};
try{await chromeSession(output,async({call,evaluate})=>{
 const load=async(query='')=>{
  await call('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/?${query}`});
  for(let i=0;i<100;i++){await pause(50);if(await evaluate(`typeof window.__displayTest==='object'`))break}
  check('actual Remote initialized '+query,await evaluate(`typeof window.__displayTest==='object'`));
  await evaluate(`(()=>{saturnAdoptWsDiagText('dds:0,14200000;modulation:0,USB;',__socket);const s=__displayTest.state;s.connected=true;s.iqStreaming=true;s.bridgeReady=true;})()`);
 };
 const waitFor=async(expression)=>{for(let i=0;i<100;i++){if(await evaluate(expression))return;await pause(20)}throw Error('deadline '+expression)};
 await load('display_fft=worker');
 await evaluate(`__displayTest.frame()`);
 await waitFor(`__displayTest.producer.latest()!==null`);
 const first=await evaluate(`__displayTest.inspect()`),expected=await evaluate(`Array.from(__displayTest.expected())`);
 check('compiled worker returns exact canonical shifted FFT',JSON.stringify(first.bins)===JSON.stringify(expected));
 check('radio buffer remains attached',first.rawBytes===64+4096*8);
 check('worker pool is exactly four fixed backing stores',first.diag.backingAllocations===4&&first.diag.retainedTransferCapacityBytes===393216);
 await waitFor(`__displayTest.inspect().rendered===__displayTest.inspect().sequence`);
 check('RAF consumes worker result',true);
 await evaluate(`__displayTest.frame();__displayTest.shiftDisplayForCenterChange(7200000);__displayTest.state.dds=7200000`);
 await pause(100);
 check('tune clears completion without a subsequent packet',await evaluate(`__displayTest.producer.latest()===null`));
 check('tune discards retained old-frequency input',await evaluate(`__displayTest.state.iqPackets.length===0`));
 await evaluate(`__displayTest.frame()`);await waitFor(`__displayTest.producer.latest()!==null`);
 const tuned=await evaluate(`__displayTest.inspect()`);
 check('fresh tuning context can produce again',tuned.diag.completed>first.diag.completed);
 // Synchronous burst: queued requests must collapse before worker callbacks run.
 await evaluate(`(()=>{for(let i=0;i<300;i++)__displayTest.frame(0,!!(i%2));})()`);
 const burst=await evaluate(`__displayTest.inspect()`);
 check('burst pending depth bounded',burst.diag.inFlight<=1&&burst.diag.pending<=1&&burst.diag.queueHighWater===1);
 await waitFor(`__displayTest.producer.latest()?.sequence===__displayTest.state.iqFrameVersion`);
 const after=await evaluate(`__displayTest.inspect()`);
 check('burst processes newest sequence rather than FIFO backlog',after.diag.coalesced>250);
 check('burst never grows transfer pool',after.diag.backingAllocations===4);
 await evaluate(`__displayTest.state.displayPaused=true;__displayTest.frame()`);await pause(100);
 check('pause invalidates completed result',await evaluate(`__displayTest.producer.latest()===null`));
 await evaluate(`__displayTest.state.displayPaused=false;__displayTest.frame()`);await waitFor(`__displayTest.producer.latest()!==null`);
 await evaluate(`__displayTest.state.moxRequested=true;for(let i=0;i<10;i++)__displayTest.frame(1);`);
 await waitFor(`__displayTest.producer.latest()!==null`);
 check('TX display retains only its latest snapshot',await evaluate(`__displayTest.state.displayIqSource==='tx'&&__displayTest.state.iqPackets.length===1`));
 check('TX worker spectrum matches canonical complex FFT',JSON.stringify((await evaluate(`__displayTest.inspect()`)).bins)===JSON.stringify(await evaluate(`Array.from(__displayTest.expected())`)));
 await evaluate(`__displayTest.state.moxRequested=false;__displayTest.resetDisplayHistory();__displayTest.frame()`);
 await waitFor(`__displayTest.producer.latest()!==null`);
 const serverBefore=await evaluate(`__displayTest.producer.diagnostics().submitted`);
 await evaluate(`(()=>{const s=__displayTest.state;s.displayRenderSource='server';s.displayServerBins=new Float32Array(4096).fill(-100);s.displayServerFftSize=4096;s.displayServerCenterHz=s.dds;s.displayServerRowVersion++;__displayTest.frame()})()`);
 await waitFor(`__displayTest.state.lastRenderedServerRowVersion===__displayTest.state.displayServerRowVersion`);
 check('server spectrum bypasses local worker',await evaluate(`__displayTest.producer.diagnostics().submitted===${serverBefore}&&__displayTest.displayPipelineSnapshot().processing==='server-spectrum'`));
 await evaluate(`__displayTest.state.displayRenderSource='iq';__displayTest.state.displayServerBins=null;__displayTest.measurement(true);__displayTest.frame()`);
 await waitFor(`!!window.__measurementBins`);
 const measured=await evaluate(`__displayTest.inspect()`);
 check('RX Measure runs canonical raw FFT instead of coalesced display result',JSON.stringify(measured.measurement)===JSON.stringify(await evaluate(`__displayTest.expectedMeasurement()`)));
 check('measurement settings retain full raw source and FFT size',measured.settings.source==='raw-rx-iq'&&measured.settings.fftSize===4096);
 check('display labels measurement bypass explicitly',await evaluate(`__displayTest.displayPipelineSnapshot().processing==='main-raw-measurement'`));
 await evaluate(`__displayTest.measurement(false);__displayTest.frame()`);await waitFor(`__displayTest.producer.latest()!==null`);
 await evaluate(`__displayTest.resetDisplayTransport('fixture disconnect');__displayTest.state.connected=false`);await pause(100);
 check('disconnect invalidates old completion',await evaluate(`__displayTest.producer.latest()===null`));
 await load();check('default retains daily main display pending target-browser review',await evaluate(`!__displayTest.producer.available`));
 await load('display_fft=main');await evaluate(`__displayTest.frame()`);await waitFor(`__displayTest.state.lastRenderedIqFrameVersion===__displayTest.state.iqFrameVersion`);
 check('explicit main fallback renders without worker allocation',await evaluate(`__displayTest.producer.diagnostics().backingAllocations===0`));
 await load('display_fft=worker&block_worker=1');await evaluate(`__displayTest.frame()`);await waitFor(`__displayTest.state.lastRenderedIqFrameVersion===__displayTest.state.iqFrameVersion`);
 check('worker creation failure falls back to a working display',await evaluate(`!__displayTest.producer.available&&__displayTest.producer.fallbackReason.includes('unavailable')`));
 await call('Page.captureScreenshot',{format:'png'}).then(({data})=>writeFileSync(join(output,'display-worker-fallback.png'),Buffer.from(data,'base64')));
});
writeFileSync(join(output,'checks.json'),JSON.stringify({format:'saturn-display-worker-browser-checks-v1',checks,scope:'compiled worker and actual Remote; synthetic IQ; no G2/RF'},null,2)+'\n');
console.log(`${checks.length} display worker browser checks passed: ${output}`);
}finally{server.close()}
