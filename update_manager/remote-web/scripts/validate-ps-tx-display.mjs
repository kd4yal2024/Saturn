// Real template/IIFE + native fixture TX IQ. All network and RF access blocked.
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,dirname,join} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {spawn} from 'node:child_process';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
assert(process.argv[2],'explicit ordinary-file native fixture TX frame required');
const native=readFileSync(process.argv[2]);
assert(native.length>64&&native.readUInt32LE(24)===3&&native.readUInt32LE(4)===192000);
const output=mkdtempSync(join(tmpdir(),'saturn-ps-live-display-browser-'));
const template=readFileSync(resolve(root,'../templates/saturn-remote-next.html'),'utf8');
const bundle=readFileSync(join(root,'dist/saturn-remote-next.js'),'utf8');
const tag=/<script\s+src="\/remote-assets\/remote-next\.js[^"]*"[\s\S]*?<\/script>/;
assert(tag.test(template));
const page=join(output,'remote.html');
writeFileSync(page,template.replace(tag,`<script>${bundle.replaceAll('</script','<\\/script')}</script>`));
const chrome=spawn(process.env.CHROMIUM||'google-chrome',['--headless=new','--no-sandbox',
  '--disable-dev-shm-usage','--disable-background-networking','--no-first-run','--disable-extensions',
  '--mute-audio','--allow-file-access-from-files','--remote-debugging-pipe',
  `--user-data-dir=${join(output,'profile')}`,'about:blank'],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let id=0,buffer='';const pending=new Map();
chrome.stdio[4].on('data',d=>{buffer+=d;let end;while((end=buffer.indexOf('\0'))>=0){
  const raw=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!raw)continue;
  const message=JSON.parse(raw),p=pending.get(message.id);if(!p)continue;pending.delete(message.id);
  message.error?p.reject(new Error(JSON.stringify(message.error))):p.resolve(message.result);
}});
function command(method,params={},sessionId){return new Promise((ok,fail)=>{const n=++id;pending.set(n,{resolve:ok,reject:fail});
  chrome.stdio[3].write(JSON.stringify({id:n,method,params,...(sessionId?{sessionId}:{})})+'\0');});}
const pause=ms=>new Promise(done=>setTimeout(done,ms));
const watchdog=setTimeout(()=>{chrome.kill();process.exitCode=2;},45000);
try {
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await command('Target.attachToTarget',{targetId,flatten:true});
  const call=(m,p={})=>command(m,p,sessionId);
  const evaluate=async expression=>{const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true});
    if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;};
  const until=async expression=>{for(let n=0;n<100;n++){if(await evaluate(expression))return;await pause(50);}
    throw new Error(expression+' '+JSON.stringify(await evaluate(`({probe:window.__txProbe,log:document.getElementById('event-log')?.textContent?.slice(-2500)})`)));};
  await call('Page.enable');await call('Network.enable');
  await call('Network.setBlockedURLs',{urls:['http://*','https://*','ws://*','wss://*']});
  await call('Page.addScriptToEvaluateOnNewDocument',{source:`
    window.__txProbe={commands:[],sockets:[],fft:[],micRequests:0,errors:[]};
    window.fetch=async()=>new Response('{}',{status:404});
    Object.defineProperty(navigator,'mediaDevices',{value:{getUserMedia(){window.__txProbe.micRequests++;return Promise.reject(new Error('offline mic forbidden'));}}});
    window.addEventListener('error',e=>window.__txProbe.errors.push(e.message));
    window.WebSocket=class {
      static CONNECTING=0;static OPEN=1;static CLOSING=2;static CLOSED=3;
      constructor(url){this.url=String(url);this.readyState=0;this.bufferedAmount=0;this.listeners={};window.__txProbe.sockets.push(this);
        setTimeout(()=>{this.readyState=1;this.emit('open',{});if(!this.url.includes('/saturn/media'))setTimeout(()=>this.text(
          'protocol:SaturnBridge,2.0;device:ANAN-G2;receive_only:false;trx_count:1;channel_count:2;vfo_limits:10000,61440000;'+
          'saturn_satp_supported:false;saturn_satp_enabled:false;remote_client_role:0,operator,1;remote_tx_rf_enabled:0,true;'+
          'dds:0,7200000;vfo:0,0,7200000;iq_samplerate:384000;modulation:0,LSB;tx_enable:0,true;ready;'),10);},0);}
      emit(type,event){this['on'+type]?.(event);for(const f of this.listeners[type]||[])f(event);}
      text(data){this.emit('message',{data});}
      addEventListener(t,f){(this.listeners[t]??=[]).push(f);}
      removeEventListener(t,f){this.listeners[t]=(this.listeners[t]||[]).filter(x=>x!==f);}
      close(){if(this.readyState===3)return;this.readyState=3;this.emit('close',{code:1000,wasClean:true,reason:''});}
      send(data){if(typeof data!=='string')throw new Error('No TX audio expected');window.__txProbe.commands.push(data);
        const control=window.__txProbe.sockets.find(s=>!s.url.includes('/saturn/media'));
        if(data.includes('tx_two_tone:0,true;'))setTimeout(()=>control.text('tx_two_tone_result:0,true,accepted,Configured,true;tx_two_tone:0,true;'),0);
        if(data.includes('tx_two_tone:0,false;'))setTimeout(()=>control.text('tx_two_tone_result:0,false,accepted,Stopping,false;tx_two_tone:0,false;'),0);
        if(data.includes('trx:0,true'))setTimeout(()=>control.text('trx:0,true;'),0);
        if(data.includes('trx:0,false;'))setTimeout(()=>control.text('trx:0,false;'),0);
      }
    };
    window.__txProbe.emitFrame=(rx=false)=>{const bytes=Uint8Array.from(atob(${JSON.stringify(native.toString('base64'))}),c=>c.charCodeAt(0));
      if(rx){const v=new DataView(bytes.buffer);v.setUint32(24,0,true);v.setUint32(4,384000,true);}
      const socket=window.__txProbe.sockets.find(s=>s.url.includes('/saturn/media'))||window.__txProbe.sockets[0];
      socket.emit('message',{data:bytes.buffer});};
  `});
  const reports=[];
  for(const width of [1440,390]) {
    await call('Emulation.setDeviceMetricsOverride',{width,height:960,deviceScaleFactor:1,mobile:width<600});
    await call('Page.navigate',{url:pathToFileURL(page).href});
    await until(`!!document.getElementById('control-tab-ps')&&typeof window.saturnConnectBridgeDiagnostic==='function'`);
    await evaluate(`(()=>{const p=window.SaturnRemoteNext.FftProcessor.prototype,original=p.transform;p.transform=function(iq){
      const bins=original.call(this,iq),tones=[700,1900].map(hz=>{const i=Math.round(bins.length/2+hz*bins.length/192000);
        return Math.max(...bins.slice(i-2,i+3));});window.__txProbe.fft.push({samples:iq.length,tones});return bins;};
      document.getElementById('control-tab-ps').click();window.saturnConnectBridgeDiagnostic();})()`);
    await until(`!document.getElementById('ps-two-tone-btn').disabled`);
    assert(!(await evaluate(`window.__txProbe.commands`)).some(s=>s.includes('trx:0,true')));
    await evaluate(`document.getElementById('tx-arm-btn').click();document.getElementById('ps-two-tone-btn').click()`);
    await until(`document.getElementById('ps-two-tone-btn').getAttribute('aria-pressed')==='true'`);
    for(let n=0;n<6;n++){await evaluate(`window.__txProbe.emitFrame()`);await pause(100);}
    await until(`window.__txProbe.fft.some(f=>f.tones.every(v=>v>-30))`);
    assert((await evaluate(`document.getElementById('ps-tx-display-status').textContent`)).includes('Live TX baseband'));
    await until(`document.getElementById('ps-tx-display-status').textContent.includes('stale')`);
    await evaluate(`window.__txProbe.emitFrame()`);await pause(100);
    await evaluate(`document.getElementById('ps-two-tone-btn').click()`);
    await until(`document.getElementById('ps-two-tone-btn').getAttribute('aria-pressed')==='false'`);
    await pause(200);const before=await evaluate(`window.__txProbe.fft.length`);
    await evaluate(`window.__txProbe.emitFrame()`);await pause(200);
    assert.equal(await evaluate(`window.__txProbe.fft.length`),before,'released TX tail must not render');
    await evaluate(`window.__txProbe.emitFrame(true)`);await pause(200);
    assert(await evaluate(`window.__txProbe.fft.length`)>before,'RX rendering must resume');
    const r=await evaluate(`({commands:window.__txProbe.commands,fft:window.__txProbe.fft,micRequests:window.__txProbe.micRequests,errors:window.__txProbe.errors})`);
    assert(r.commands.some(s=>s.includes('tx_two_tone:0,true;'))&&r.commands.some(s=>s.includes('trx:0,true')));
    assert(r.commands.some(s=>s.includes('tx_two_tone:0,false;'))&&r.commands.some(s=>s.includes('trx:0,false;')));
    assert.equal(r.micRequests,0);assert.deepEqual(r.errors,[]);reports.push({width,...r});
  }
  const result={actualTemplate:true,actualBundle:true,nativeFixtureFrame:true,psButtonSharedAction:true,
    actualFftBothTones:true,staleAndLateTailChecked:true,rxRestored:true,rfCommandsReachedRadio:0,output,reports};
  writeFileSync(join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({output,passed:true,widths:reports.map(r=>r.width)}));
}finally{clearTimeout(watchdog);await command('Browser.close').catch(()=>{});chrome.kill();}
