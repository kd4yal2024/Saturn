#!/usr/bin/env node
// Actual Remote runtime, deterministic local IQ/PCM replay. No live Bridge/RF.
// Instrumented allocation counters count JS-visible typed-array backing stores,
// not all JS objects, browser-native WebSocket allocations or total GC activity.
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { readFileSync,writeFileSync,mkdirSync,mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname,resolve,join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromeSession,pause } from './lib/chrome-session.mjs';
import { startDisplayBridge,readProcessCpu } from './lib/display-bridge-replay.mjs';
import { execFileSync } from 'node:child_process';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const output=process.argv.find(a=>a.startsWith('--output='))?.slice(9)||mkdtempSync(join(tmpdir(),'saturn-display-profile-'));
const seconds=Number(process.argv.find(a=>a.startsWith('--seconds='))?.slice(10)||12);
const workloads=(process.argv.find(a=>a.startsWith('--workloads='))?.slice(12)||'2d,3d,interactive').split(',');
const quality=process.argv.find(a=>a.startsWith('--quality='))?.slice(10)||'performance';
if(!['auto','performance','balanced','high'].includes(quality))throw Error('Unsupported terrain quality');
const engines=(process.argv.find(a=>a.startsWith('--engines='))?.slice(10)||'main').split(',');
const bridgeBinary=process.argv.find(a=>a.startsWith('--bridge-test-binary='))?.slice(21);
if(!engines.length||engines.some(e=>!['main','worker','spectrum'].includes(e)))throw Error('engines must be main, worker and/or spectrum');
if(engines.includes('spectrum')&&!bridgeBinary)throw Error('spectrum comparisons require --bridge-test-binary (actual Rust TCI/spectrum worker)');
const fixedFft=Number(process.argv.find(a=>a.startsWith('--fft-size='))?.slice(11)||0);
if(!Number.isFinite(fixedFft)||(fixedFft&&![256,512,1024,2048,4096,8192,16384].includes(fixedFft)))throw Error('Unsupported fixed FFT size');
const zoom=Number(process.argv.find(a=>a.startsWith('--zoom='))?.slice(7)||1);
if(!Number.isFinite(zoom)||zoom<1||zoom>32)throw Error('zoom must be 1..32');
const clockTicks=bridgeBinary?Number(execFileSync('getconf',['CLK_TCK'],{encoding:'utf8'}).trim()):null;
if(!workloads.length||workloads.some(w=>!['2d','3d','interactive'].includes(w)))throw Error('Unsupported workload');
if(!Number.isFinite(seconds)||seconds<6||seconds>60)throw Error('seconds must be 6..60');
mkdirSync(output,{recursive:true});
const template=readFileSync(resolve(root,'../templates/saturn-remote-next.html'),'utf8');
const bundle=readFileSync(resolve(root,'dist/saturn-remote-next.js'));
const instrumentation=`<script>
localStorage.clear();localStorage.setItem('saturn.remote.freqLock','0');localStorage.setItem('saturn.remote.displayZoom',${JSON.stringify(zoom)});
window.__allocation={bytes:0,arrays:0};
const OriginalF32=Float32Array;
window.Float32Array=new Proxy(OriginalF32,{construct(target,args){const out=Reflect.construct(target,args);if(!(args[0] instanceof ArrayBuffer)){window.__allocation.bytes+=out.byteLength;window.__allocation.arrays++}return out}});
window.__longTasks=[];try{new PerformanceObserver(list=>{for(const task of list.getEntries())window.__longTasks.push(task.duration)}).observe({type:'longtask',buffered:false})}catch{}
window.__visibility=[];document.addEventListener('visibilitychange',()=>window.__visibility.push({at:performance.now(),state:document.visibilityState}));
window.__beats=[];let expected=performance.now()+10;setInterval(()=>{const now=performance.now();window.__beats.push(Math.max(0,now-expected));expected=now+10},10);
window.__fakeSocket={readyState:1,bufferedAmount:0,send(){},close(){this.readyState=3}};
const nativeOpen=window.open.bind(window);window.open=(...args)=>window.__profilePopup=nativeOpen(...args);
</script>`;
const hooks=`
// Fixture-only matched cadence: production defaults are not altered. Otherwise
// raw/worker terrain's 30 Hz and server terrain's 10 Hz are different workloads.
${bridgeBinary ? 'displayRenderIntervalMs=()=>state.terrainActive?100:33;' : ''}
${fixedFft ? `displayFftTargetSize=()=>${fixedFft};` : ''}
window.__profileSpectrum=null;
const originalProfileDraw=drawDisplayBins;
drawDisplayBins=(bins,...args)=>{
 if(!window.__profileSpectrum&&window.__profileCollect){
  const peak=hz=>{const bin=Math.round(bins.length/2+hz*bins.length/384000);let db=-Infinity;for(let i=bin-1;i<=bin+1;i++)db=Math.max(db,bins[i]??-Infinity);return db};
  window.__profileSpectrum={fftSize:bins.length,positive1500Db:peak(1500),negative4500Db:peak(-4500)};
 }
 return originalProfileDraw(bins,...args);
};
window.__profileConnect=async url=>{
 const socket=new WebSocket(url);socket.binaryType='arraybuffer';window.__profileSocket=socket;
 window.__resetProfileWire=()=>window.__wire={bytes:0,iqBytes:0,spectrumBytes:0,audioBytes:0,iqFrames:0,spectrumRows:0,audioFrames:0};window.__resetProfileWire();
 socket.onmessage=({data})=>{
  if(typeof data==='string')window.saturnAdoptWsDiagText(data,socket);
  else {const type=new DataView(data).getUint32(24,true);window.__wire.bytes+=data.byteLength;if(type===0){window.__wire.iqFrames++;window.__wire.iqBytes+=data.byteLength}if(type===16){window.__wire.spectrumRows++;window.__wire.spectrumBytes+=data.byteLength}if(type===1){window.__wire.audioFrames++;window.__wire.audioBytes+=data.byteLength}window.saturnAdoptWsDiagFrame(data,socket)}
 };
 for(let i=0;i<100;i++){if(state.bridgeReady)break;await new Promise(r=>setTimeout(r,50))}
 if(!state.bridgeReady)throw Error('Loopback Bridge ready deadline');
 await startRxAudio();sendTci('rx_audio_codec:0,pcm;audio_samplerate:48000;audio_channels:2;audio_start:0;');
 requestDisplayTransport('profile',true);sendIqStart();
};
window.__profileStart=async()=>{
  window.saturnAdoptWsDiagSocketIgnored=undefined;
  window.saturnAdoptWsDiagText('dds:0,14200000;vfo:0,0,14200000;modulation:0,USB;iq_samplerate:384000;remote_client_role:0,operator,1;',window.__fakeSocket);
  state.bridgeReady=true;state.connected=true;state.iqStreaming=true;
  await startRxAudio();
};
window.__profileSnapshot=()=>({display:displayPipelineSnapshot(),terrain:terrainDiagnostics(),
  audio:{rate:state.audioSampleRate,channels:state.audioChannels,mode:state.audioWorkletMode,context:state.audioCtx?.state,underruns:state.rxWorkletUnderruns,frames:state.audioFramesPlayed,queueMs:audioLeadMs()},
  source:{iqFrames:state.iqFrameVersion,rate:state.sampleRate,displaySource:state.displayRenderSource},
  geometry:{fftSize:displayProfileDiagnostics().displayFftSize,targetFftSize:displayFftTargetSize(),intervalMs:displayRenderIntervalMs(),terrainQuality:state.terrain?.quality,zoom:effectiveDisplayZoom(),canvasWidth:terrainRenderer?.canvas.width,mesh:terrainDiagnostics().geometry,backingStore:terrainDiagnostics().backingStore},
  terrainActive:state.terrainActive,gpuRenderer:(()=>{const gl=terrainRenderer?.gl||spectrumRenderer.gl;if(!gl)return 'Canvas2D';const ext=gl.getExtension('WEBGL_debug_renderer_info');return gl.getParameter(ext?.UNMASKED_RENDERER_WEBGL||gl.RENDERER)})(),worker:typeof displayFftProducer!=='undefined'?displayFftProducer.diagnostics():null});
window.__profileTiming={fft:[],draw:[],age:[],terrain:[],meters:[]};
const profileRecord=recordBrowserTimingSample;
recordBrowserTimingSample=(bucket,value)=>{profileRecord(bucket,value);if(window.__profileCollect){const kind=bucket===displayTimingBuckets.fft?'fft':bucket===displayTimingBuckets.drawCpu?'draw':bucket===displayTimingBuckets.arrivalToDraw?'age':bucket===displayTimingBuckets.terrainCpu?'terrain':bucket===displayTimingBuckets.metersCpu?'meters':null;if(kind)window.__profileTiming[kind].push(value)}};
window.__profileMode=mode=>{state.terrain={...state.terrain,quality:${JSON.stringify(quality)}};return setTerrainMode(mode,false)};
window.__profileTune=hz=>setFrequency(hz,false,{rememberBand:false});
`;
const html=template.replace('<head>','<head>'+instrumentation).replace('window.saturnGetRxLatencyText = rxLatencyDiagnosticText;','window.saturnGetRxLatencyText = rxLatencyDiagnosticText;'+hooks);
const server=createServer((req,res)=>{
  if(req.url?.startsWith('/remote-assets/remote-next.js')){res.setHeader('Content-Type','text/javascript');res.end(bundle)}
  else if(req.url?.startsWith('/?')||req.url==='/'){res.setHeader('Content-Type','text/html');res.end(html)}
  else{res.writeHead(404);res.end('local performance replay')}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const makeReplay=`(()=>{
const source = new Worker(URL.createObjectURL(new Blob(['('+(${replaySource.toString()}).toString()+')()'],{type:'text/javascript'})));
window.__replay=source;window.__replayStats={};window.__replayDelivery=[];
source.onmessage=({data})=>{
 if(data.kind==='stats'){window.__replayStats=data;return}
 window.__replayDelivery.push(Math.max(0,performance.timeOrigin+performance.now()-data.sentAt));
 window.saturnAdoptWsDiagFrame(data.buffer,window.__fakeSocket);
 source.postMessage({kind:data.kind,buffer:data.buffer},[data.buffer]);
};
})()`;
function replaySource(){
 const pools={iq:[],audio:[]},sequence={iq:0,audio:0},dropped={iq:0,audio:0};
 function packet(samples,channels,type,rate){const buffer=new ArrayBuffer(64+samples*channels*4),v=new DataView(buffer);v.setUint32(4,rate,true);v.setUint32(8,20,true);v.setUint32(16,samples,true);v.setUint32(20,samples*channels,true);v.setUint32(24,type,true);v.setUint32(28,channels,true);const f=new Float32Array(buffer,64);for(let i=0;i<samples;i++){const a=2*Math.PI*1300*i/rate;f[i*channels]=.1*Math.cos(a);if(channels===2)f[i*2+1]=.1*Math.sin(a)}return buffer}
 for(let i=0;i<8;i++)pools.iq.push(packet(12800,2,0,384000));
 for(let i=0;i<16;i++)pools.audio.push(packet(960,2,1,48000));
 onmessage=({data})=>pools[data.kind].push(data.buffer);
 const send=kind=>{const buffer=pools[kind].pop();if(!buffer){dropped[kind]++;return}new DataView(buffer).setUint32(32,++sequence[kind],true);postMessage({kind,buffer,sentAt:performance.timeOrigin+performance.now()},[buffer])};
 setInterval(()=>send('iq'),1000/30);setInterval(()=>send('audio'),20);
 setInterval(()=>postMessage({kind:'stats',sequence,dropped,available:{iq:pools.iq.length,audio:pools.audio.length}}),1000);
}
const summarize=values=>{const v=values.filter(Number.isFinite).sort((a,b)=>a-b);const at=p=>v.length?v[Math.min(v.length-1,Math.ceil(v.length*p)-1)]:null;return {count:v.length,p50:at(.5),p95:at(.95),p99:at(.99),max:v.at(-1)??null}};
const results=[];let browserIdentity;let activeBridge=null;
try{
await chromeSession(output,async({command,call,evaluate})=>{
 browserIdentity=await command('Browser.getVersion');
 for(const engine of engines)for(const workload of workloads){
  await call('Emulation.setDeviceMetricsOverride',{width:1440,height:960,deviceScaleFactor:1,mobile:false});
  if(bridgeBinary)activeBridge=await startDisplayBridge(bridgeBinary,join(output,engine+'-'+workload+'-bridge'));
  await call('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/?display_fft=${engine==='worker'?'worker':'main'}&display_transport=${engine==='spectrum'?'spectrum':'iq'}&rx_audio_codec=pcm`});
  let ready=false;for(let i=0;i<100;i++){await pause(50);if(await evaluate(`!!window.__profileStart && !!document.getElementById('rx-eq-band-10')`).catch(()=>false)){ready=true;break}}
  if(!ready)throw Error('Remote runtime failed to mount');
  if(activeBridge)await evaluate(`window.__profileConnect(${JSON.stringify(activeBridge.url)})`);
  else {await evaluate('window.__profileStart()');await evaluate(makeReplay)}
  if(workload!=='2d')await evaluate(`window.__profileMode('3d')`);
  await pause(2000);
  const bridgeCpuStart=activeBridge?readProcessCpu(activeBridge.pid):null;const hostStart=performance.now();
  const start=await evaluate(`(()=>{window.__allocation={bytes:0,arrays:0};window.__beats=[];window.__longTasks=[];window.__visibility=[];window.__replayDelivery=[];window.__profileTiming={fft:[],draw:[],age:[],terrain:[],meters:[]};window.__profileCollect=true;window.__profileSpectrum=null;window.__resetProfileWire?.();return {at:performance.now(),snapshot:window.__profileSnapshot(),heap:performance.memory?.usedJSHeapSize??null}})()`);
  const events=[];
  for(let elapsed=0;elapsed<seconds;elapsed++){
    await pause(1000);
    if(workload==='interactive'){
      if([1,3,7].includes(elapsed)){await evaluate(`window.__profileTune(${14200000+elapsed*1000})`);events.push({second:elapsed+1,action:'tune'})}
      if(elapsed===2){await call('Emulation.setDeviceMetricsOverride',{width:1100,height:780,deviceScaleFactor:1,mobile:false});events.push({second:3,action:'resize-small'})}
      if(elapsed===4){await evaluate(`document.getElementById('header-setup-btn').click()`);events.push({second:5,action:'settings-popout'})}
      if(elapsed===6){await evaluate(`window.__profilePopup?.document.querySelector('[data-page="transmit"]')?.click()`);events.push({second:7,action:'settings-transmit-page'})}
      if(elapsed===8){await evaluate(`window.__profilePopup?.close()`);events.push({second:9,action:'close-popout'})}
      if(elapsed===9){await call('Emulation.setDeviceMetricsOverride',{width:1640,height:1000,deviceScaleFactor:1,mobile:false});events.push({second:10,action:'resize-large'})}
    }
  }
  const end=await evaluate(`({at:performance.now(),snapshot:window.__profileSnapshot(),allocation:window.__allocation,beats:window.__beats,longTasks:window.__longTasks,heap:performance.memory?.usedJSHeapSize??null,visibility:document.visibilityState,visibilityEvents:window.__visibility,fullWindowTimings:window.__profileTiming,replayDelivery:window.__replayDelivery,replay:window.__replayStats,wire:window.__wire,spectrumProbe:window.__profileSpectrum})`);
  const hostSeconds=(performance.now()-hostStart)/1000;const bridgeCpuEnd=activeBridge?readProcessCpu(activeBridge.pid):null;
  if(bridgeCpuStart&&bridgeCpuEnd.startTicks!==bridgeCpuStart.startTicks)throw Error('Bridge process identity changed');
  const duration=(end.at-start.at)/1000;
  results.push({engine,workload,quality,durationSeconds:duration,events,spectrumProbe:end.spectrumProbe,wire:end.wire,bridgeCpuOneCorePercent:bridgeCpuStart?100*(bridgeCpuEnd.ticks-bridgeCpuStart.ticks)/clockTicks/hostSeconds:null,start:start.snapshot,end:end.snapshot,
    mainBackingAllocationBytesPerSecond:end.allocation.bytes/duration,mainBackingAllocationArraysPerSecond:end.allocation.arrays/duration,
    fullWindowTimings:{fftMs:summarize(end.fullWindowTimings.fft),drawCpuMs:summarize(end.fullWindowTimings.draw),newestIngressToDrawMs:summarize(end.fullWindowTimings.age),terrainCallCpuMs:summarize(end.fullWindowTimings.terrain),meterCallCpuMs:summarize(end.fullWindowTimings.meters)},replay:end.replay,replayDeliveryMs:summarize(end.replayDelivery),heartbeatLagMs:summarize(end.beats),longTasksMs:summarize(end.longTasks),heapChangeBytes:start.heap===null||end.heap===null?null:end.heap-start.heap,
    audioUnderrunDelta:typeof end.snapshot.audio.underruns==='number'&&typeof start.snapshot.audio.underruns==='number'?end.snapshot.audio.underruns-start.snapshot.audio.underruns:null,
    audioFramesDelta:end.snapshot.audio.frames-start.snapshot.audio.frames,visibility:end.visibility,visibilityEvents:end.visibilityEvents});
  if(engine==='worker' && end.snapshot.worker?.engine!=='worker')throw Error('Requested worker did not run: '+JSON.stringify(end.snapshot.worker));
  if(engine==='spectrum'&&(end.snapshot.source.displaySource!=='server'||!(end.wire?.spectrumRows>0)))throw Error('Server spectrum path not exercised');
  if(bridgeBinary&&(!Number.isFinite(end.spectrumProbe?.positive1500Db)||!Number.isFinite(end.spectrumProbe?.negative4500Db)||Math.abs(end.spectrumProbe.positive1500Db+22.5)>.4||Math.abs(end.spectrumProbe.negative4500Db+32.04)>.4))throw Error('Synthetic tone levels/orientation differ from the shared source');
  if(end.snapshot.source.rate!==384000)throw Error('Display comparison changed IQ rate');
  if(!(results.at(-1).audioFramesDelta>0))throw Error('Audio did not advance; invalid performance baseline');
  await call('Page.captureScreenshot',{format:'png'}).then(({data})=>writeFileSync(join(output,engine+'-'+workload+'.png'),Buffer.from(data,'base64')));
  await evaluate(`window.__profilePopup?.close();window.__replay?.terminate();window.__profileSocket?.close()`);
  if(activeBridge){await pause(150);results.at(-1).bridge=await activeBridge.stop();activeBridge=null}
 }
},{timeoutMs:(seconds+4)*1000*engines.length*workloads.length+30000});
const report={browserIdentity,format:'saturn-display-performance-v1',source:bridgeBinary?'actual Remote + Rust TCI queues/framing and spectrum worker on loopback; synthetic IQ/PCM, no hardware/DSP/TLS/proxy':'actual Remote runtime; independent, bounded worker supplies local IQ/PCM replay; no radio',fixedFft:fixedFft||null,cadence:bridgeBinary?'matched fixture cadence: 33 ms 2D / 100 ms terrain on every path':'production cadence',bridgeCpuScope:'process utime+stime ticks over host measurement window, one core = 100%; Linux loopback fixture only, no WDSP/acquisition/TLS; optimized or debug build depends on explicit binary',bridgeTestBinarySha256:bridgeBinary?createHash('sha256').update(readFileSync(bridgeBinary)).digest('hex'):null,profilerSha256:createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex'),templateSha256:createHash('sha256').update(template).digest('hex'),bundleSha256:createHash('sha256').update(bundle).digest('hex'),allocationScope:'instrumented main-realm Float32Array backing allocations; excludes objects, native allocations and worker allocations',timingScope:'browser ingress to draw submission; not RF acquisition age or GPU presentation latency; fullWindowTimings.fftMs is main-thread only, worker FFT uses bounded recent producer statistics, server FFT histogram includes warmup',results};
report.matchedComparisons=workloads.map(workload=>{
 const cases=results.filter(r=>r.workload===workload);
 const settings=cases.map(r=>({...r.end.geometry,audioRate:r.end.audio.rate,audioChannels:r.end.audio.channels}));
 const same=settings.every(s=>s.fftSize===s.targetFftSize&&JSON.stringify(s)===JSON.stringify(settings[0]));
 return {workload,matchedGeometry:same,settings,engines:cases.map(r=>r.engine)};
});
writeFileSync(join(output,'report.json'),JSON.stringify(report,null,2)+'\n');
if(bridgeBinary&&report.matchedComparisons.some(c=>!c.matchedGeometry))throw Error('Unequal resolution/cadence/geometry; comparison report retained');
console.log(JSON.stringify(results.map(r=>({engine:r.engine,workload:r.workload,allocationMBps:r.mainBackingAllocationBytesPerSecond/1e6,heartbeatP99:r.heartbeatLagMs.p99,underruns:r.audioUnderrunDelta,frames:r.audioFramesDelta,fft:r.end.display.fftMs,age:r.end.display.newestIqArrivalToDrawMs})),null,2));
console.log(output);
}finally{if(activeBridge)await activeBridge.stop();server.close()}
