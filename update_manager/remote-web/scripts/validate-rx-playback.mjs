#!/usr/bin/env node
// Actual Remote + AudioWorklet, deterministic paired-packet delivery. No radio,
// real WebSocket, microphone, device reconfiguration or RF commands.
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv.find(a => a.startsWith('--output='))?.slice(9) || mkdtempSync(join(tmpdir(), 'saturn-rx-playback-'));
mkdirSync(output, { recursive: true });
const source = readFileSync(resolve(root, '../templates/saturn-remote-next.html'), 'utf8');
const bundle = readFileSync(resolve(root, 'dist/saturn-remote-next.js'));
const worker = `
const start = performance.now(); let sequence = 0;
function send() {
  const due = sequence === 0 ? 0 : Math.ceil(sequence / 2) * 40;
  const delay = due - (performance.now() - start);
  if (delay > 0.3) { setTimeout(send, delay); return; }
  const b = new ArrayBuffer(64 + 960 * 2 * 4), v = new DataView(b), f = new Float32Array(b, 64);
  v.setUint32(4,48000,true);v.setUint32(20,1920,true);v.setUint32(24,1,true);
  v.setUint32(28,2,true);v.setUint32(32,++sequence,true);
  for(let i=0;i<960;i++){const x=0.05*Math.sin(2*Math.PI*500*(i+sequence*960)/48000);f[i*2]=x;f[i*2+1]=-x;}
  postMessage(b,[b]); if(sequence < 180) setTimeout(send,0);
} send();`;
assert.equal(source.split('    void init();').length, 2);
const html = source.replace('<head>', `<head><script>
localStorage.clear();window.__realSockets=0;window.__mic=0;
const NativeWS=window.WebSocket;window.WebSocket=class extends NativeWS{constructor(...args){super(...args);window.__realSockets++}};
if(navigator.mediaDevices?.getUserMedia)navigator.mediaDevices.getUserMedia=()=>{window.__mic++;throw Error('No microphone in RX fixture')};
</script>`).replace('    void init();', `    void init().then(() => {
window.__rxReady=true;
window.__runRxBurst=async()=>{
  const sent=[];
  state.ws={readyState:1,bufferedAmount:0,send:c=>sent.push(c)};
  state.connected=true;state.bridgeReady=true;state.remoteClientRole='viewer';
  await startRxAudio();
  if(state.audioWorkletMode!=='msg')throw Error('Actual default worklet did not start');
  const blob=new Blob([${JSON.stringify(worker)}],{type:'text/javascript'});
  const url=URL.createObjectURL(blob), sender=new Worker(url);
  let delivered=0;sender.onmessage=e=>{delivered++;handleAudioFrame(e.data)};
  await new Promise(done=>setTimeout(done,3000));sender.terminate();URL.revokeObjectURL(url);
  const result={delivered,contextState:state.audioCtx.state,contextTime:state.audioCtx.currentTime,
    worklet:state.audioWorkletMode,underruns:state.rxWorkletUnderruns,
    overflows:state.rxWorkletOverflows,playout:state.rxPlaybackBuffer,
    adaptiveTargetMs:state.rxAdaptiveRateTargetQueueMs,
    readout:document.getElementById('setup-network-playback-status').textContent,
    diagnostics:rxLatencyDiagnosticText(),sent,realSockets:window.__realSockets,microphones:window.__mic};
  state.connected=false;state.bridgeReady=false;await stopRxAudio(false,true);return result;
};
});`);
const server = createServer((req, res) => {
  if(req.url?.startsWith('/remote-assets/remote-next.js')){res.setHeader('Content-Type','text/javascript');res.end(bundle)}
  else if(req.url?.startsWith('/remote-assets/swl.csv')){res.setHeader('Content-Type','text/csv');res.end(readFileSync(resolve(root,'../templates/saturn-swl.csv')))}
  else if(req.url?.startsWith('/?')){res.setHeader('Content-Type','text/html');res.end(html)}
  else {res.writeHead(404);res.end('offline fixture')}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const chrome=spawn(process.env.CHROMIUM||'google-chrome',[
  '--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run',
  '--disable-background-networking','--disable-extensions','--disable-sync','--mute-audio',
  '--autoplay-policy=no-user-gesture-required','--remote-debugging-pipe',`--user-data-dir=${join(output,'profile')}`,'about:blank',
],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let id=0,buffer='',stderr='',session;const pending=new Map();
chrome.stderr.on('data',d=>{stderr+=d});
chrome.stdio[4].on('data',d=>{buffer+=d;for(let at;(at=buffer.indexOf('\0'))>=0;){
  const raw=buffer.slice(0,at);buffer=buffer.slice(at+1);if(!raw)continue;
  const m=JSON.parse(raw),p=pending.get(m.id);if(p){pending.delete(m.id);m.error?p.reject(Error(JSON.stringify(m.error))):p.resolve(m.result)}
}});
const command=(method,params={},sessionId)=>new Promise((resolve,reject)=>{
  const next=++id;pending.set(next,{resolve,reject});chrome.stdio[3].write(JSON.stringify({id:next,method,params,...(sessionId?{sessionId}:{})})+'\0');
});
const evaluate=async expression=>{const r=await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true},session);
  if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;};
const pause=ms=>new Promise(done=>setTimeout(done,ms));
const watchdog=setTimeout(()=>{console.error(stderr.slice(-2000));chrome.kill();process.exit(2)},60000);
const results=[];
try {
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  ({sessionId:session}=await command('Target.attachToTarget',{targetId,flatten:true}));
  // Repeat reverse order to avoid attributing browser warm-up to buffering.
  for(const enabled of [false,true,true,false]){
    await command('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/?rx_audio_codec=pcm${enabled?'':'&rx_playback_buffer=off'}`},session);
    let ready=false;for(let i=0;i<100;i++){await pause(50);if(await evaluate('window.__rxReady===true').catch(()=>false)){ready=true;break}}
    assert.ok(ready,'Actual page initialization');
    const r=await evaluate('window.__runRxBurst()');r.enabled=enabled;results.push(r);
    writeFileSync(join(output,'runs.json'),JSON.stringify(results,null,2)+'\n');
    assert.equal(r.contextState,'running');assert.ok(r.contextTime>2.5);assert.ok(r.delivered>100);
    assert.equal(r.playout.enabled,enabled);assert.ok(r.playout.maxQueueMs<=120.01);
    assert.equal(r.realSockets,0);assert.equal(r.microphones,0);
    assert.ok(!r.sent.some(c=>/trx:0,true|tx_two_tone:0,true/.test(c)));
    assert.ok(r.diagnostics.includes('RX playback buffer:'));
    assert.ok(r.readout.includes('cap 120 ms'));
    if(enabled)assert.ok(r.adaptiveTargetMs>=48&&r.adaptiveTargetMs<=80);
  }
  for(const [a,b] of [[results[0],results[1]],[results[3],results[2]]]){
    assert.ok(a.underruns>0,'Immediate playback must reproduce burst underruns');
    assert.ok(b.underruns<a.underruns/2,'Bounded playback must reduce underruns in both run orders');
    assert.equal(b.overflows,0);
  }
  const report={format:'saturn-rx-playback-browser-v1',status:'passed',stimulus:'48 kHz stereo PCM, 20 ms packets paired at 40 ms boundaries',results,physicalRadioAccessed:false};
  writeFileSync(join(output,'browser-validation.json'),JSON.stringify(report,null,2)+'\n');
  console.log(JSON.stringify({status:'passed',runs:results.map(r=>({enabled:r.enabled,underruns:r.underruns,overflows:r.overflows,targetMs:r.playout.targetMs})),output},null,2));
}finally{clearTimeout(watchdog);chrome.kill();server.close()}
