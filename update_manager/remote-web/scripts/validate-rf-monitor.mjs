// Real template/bundle, Rust-generated SYNTHETIC spectrum fixture, blocked network.
// Mock timing is explicitly rebased for lifecycle tests; this is NOT RF evidence.
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,dirname,join} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {spawn} from 'node:child_process';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
assert(process.argv[2],'explicit synthetic Rust spectrum fixture required');
const raw=readFileSync(process.argv[2],'utf8').trim();
const fixture=JSON.parse(raw.slice('saturn_ps_spectrum:'.length,-1));
assert.equal(fixture.format,'saturn-rf-spectrum-v1');assert.equal(fixture.rfImdQualified,false);
const correctionRaw=process.argv[3]?readFileSync(process.argv[3],'utf8').trim():null;
const correction=correctionRaw?JSON.parse(correctionRaw.slice('saturn_ps_correction:'.length,-1)):null;
if(correction){assert.equal(correction.format,'saturn-ps-correction-v1');assert.equal(correction.nativeSamples,4096);assert.equal(correction.correction.length,512);}
const output=mkdtempSync(join(tmpdir(),'saturn-rf-monitor-browser-'));
const source=readFileSync(resolve(root,'../templates/saturn-remote-next.html'),'utf8');
const bundle=readFileSync(join(root,'dist/saturn-remote-next.js'),'utf8');
const page=join(output,'remote.html');
writeFileSync(page,source.replace(/<script\s+src="\/remote-assets\/remote-next\.js[^"]*"[\s\S]*?<\/script>/,
  `<script>${bundle.replaceAll('</script','<\\/script')}</script>`));
const chrome=spawn(process.env.CHROMIUM||'google-chrome',['--headless=new','--no-sandbox','--disable-dev-shm-usage',
  '--disable-background-networking','--no-first-run','--disable-extensions','--mute-audio','--allow-file-access-from-files',
  '--remote-debugging-pipe',`--user-data-dir=${join(output,'profile')}`,'about:blank'],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let sequence=0,buffer='';const pending=new Map();
chrome.stdio[4].on('data',d=>{buffer+=d;let end;while((end=buffer.indexOf('\0'))>=0){const raw=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!raw)continue;
  const message=JSON.parse(raw),p=pending.get(message.id);if(!p)continue;pending.delete(message.id);message.error?p.reject(new Error(JSON.stringify(message.error))):p.resolve(message.result);}});
function command(method,params={},sessionId){return new Promise((ok,fail)=>{const id=++sequence;pending.set(id,{resolve:ok,reject:fail});
  chrome.stdio[3].write(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})})+'\0');});}
const pause=ms=>new Promise(r=>setTimeout(r,ms));const watchdog=setTimeout(()=>{chrome.kill();process.exitCode=2;},45000);
try{
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await command('Target.attachToTarget',{targetId,flatten:true});
  const call=(m,p={})=>command(m,p,sessionId);
  const evaluate=async expression=>{const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true});
    if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;};
  const until=async expression=>{for(let i=0;i<100;i++){if(await evaluate(expression))return;await pause(40);}throw new Error(expression+': '+JSON.stringify(await evaluate('window.__rfProbe')));};
  await call('Page.enable');await call('Network.enable');await call('Network.setBlockedURLs',{urls:['http://*','https://*','ws://*','wss://*']});
  await call('Page.addScriptToEvaluateOnNewDocument',{source:`
    window.__rfProbe={sockets:[],commands:[],errors:[],serial:0,stream:false};
    window.fetch=async()=>new Response('{}',{status:404});
    window.addEventListener('error',e=>window.__rfProbe.errors.push(e.message));
    window.WebSocket=class{
      static OPEN=1;static CONNECTING=0;static CLOSING=2;static CLOSED=3;
      constructor(url){this.url=String(url);this.listeners={};this.readyState=0;this.bufferedAmount=0;window.__rfProbe.sockets.push(this);
        setTimeout(()=>{this.readyState=1;this.emit('open',{});if(!this.url.includes('/saturn/media'))setTimeout(()=>this.text(
          'protocol:SaturnBridge,2.0;device:ANAN-G2;receive_only:false;trx_count:1;channel_count:2;'+
          'saturn_satp_supported:false;saturn_satp_enabled:false;remote_client_role:0,operator,1;remote_tx_rf_enabled:0,true;'+
          'dds:0,7200000;vfo:0,0,7200000;iq_samplerate:384000;modulation:0,LSB;tx_enable:0,true;saturn_ps_spectrum_caps:raw-pairs-v1;saturn_ps_correction_caps:${correction?'wdsp-ampview-v1':'unavailable'};ready;'),10);},0);}
      emit(t,e){this['on'+t]?.(e);for(const f of this.listeners[t]||[])f(e);}text(data){this.emit('message',{data});}
      addEventListener(t,f){(this.listeners[t]??=[]).push(f);}removeEventListener(t,f){this.listeners[t]=(this.listeners[t]||[]).filter(v=>v!==f);}
      close(){this.readyState=3;this.emit('close',{code:1000,wasClean:true,reason:''});}
      send(data){if(typeof data!=='string')throw new Error('No TX media permitted');window.__rfProbe.commands.push(data);
        for(const part of data.split(';'))if(part.startsWith('saturn_ping:')){const [nonce,sent]=part.slice(12).split(',');
          const control=window.__rfProbe.sockets.find(s=>!s.url.includes('/saturn/media'));const at=Math.floor(performance.now()*1e6);
          setTimeout(()=>control.text('saturn_pong:'+nonce+','+sent+';saturn_ps_spectrum_clock:'+nonce+',${fixture.boot},'+at+';'),1);}}
    };
    window.__rfProbe.emit=()=>{const r=${JSON.stringify(fixture)};const now=performance.now();r.serial=String(++window.__rfProbe.serial);
      if(window.__rfProbe.timing){r.stimulus='saturn-timing-multisine-v1';
        r.requestedStimulus={version:r.stimulus,seed:0x53465031,pcmRateHz:48000,repeatMs:1000};
        r.reason='timing multisine; two-tone IMD unavailable';
        r.products=r.products.map(p=>({...p,dbc:null,upperBoundDbc:null,reason:r.reason}));}
      r.observedFirstNs=String(Math.floor((now-100)*1e6));r.observedLastNs=String(Math.floor((now-15)*1e6));r.publishedNs=String(Math.floor(now*1e6));
      const control=window.__rfProbe.sockets.find(s=>!s.url.includes('/saturn/media'));
      control.text('saturn_ps_spectrum:'+JSON.stringify(r).replaceAll(';','\\\\u003b')+';');
      const native=${JSON.stringify(correction)};
      if(native){native.boot=r.boot;native.serial=r.serial;native.snapshotNs=String(Math.floor((now-20)*1e6));native.publishedNs=r.publishedNs;
        if(window.__rfProbe.curveEmpty){native.nativeSamples=0;native.measured=[];native.correction=[];native.nativeApplied=false;}
        control.text('saturn_ps_correction:'+JSON.stringify(native)+';');}
    };
    setInterval(()=>{if(window.__rfProbe.stream)window.__rfProbe.emit();},100);
  `});
  const reports=[];
  for(const width of [1440,390]){
    await call('Emulation.setDeviceMetricsOverride',{width,height:960,deviceScaleFactor:1,mobile:width<600});
    await call('Page.navigate',{url:pathToFileURL(page).href});
    await until(`typeof window.saturnConnectBridgeDiagnostic==='function'`);
    await evaluate(`document.getElementById('control-tab-ps').click();window.saturnConnectBridgeDiagnostic()`);
    await until(`window.__rfProbe.sockets.some(s=>s.readyState===1)`);
    await pause(700);
    await evaluate(`window.__rfProbe.sockets.find(s=>!s.url.includes('/saturn/media')).text('tx_state:0,keyed;trx:0,true;');window.__rfProbe.stream=true;`);
    await until(`document.querySelector('[data-rf-status]').textContent.startsWith('LIVE RF')`);
    const text=await evaluate(`document.querySelector('[data-rf-summary]').textContent`);
    assert(text.includes('IMD3 lower'));assert(text.includes('-40.0 dBc'));assert(text.includes('Overload unknown'));assert(text.includes('not calibrated RF IMD'));
    await evaluate(`document.querySelector('[data-rf-save]').click()`);
    await until(`document.querySelector('[data-rf-summary]').textContent.includes('amber overlay')`);
    await evaluate(`document.querySelector('[data-rf-popout]').click()`);
    const {targetInfos}=await command('Target.getTargets');const popup=targetInfos.find(t=>t.title==='Saturn RF Feedback / IMD Monitor');assert(popup);
    const popupSession=await command('Target.attachToTarget',{targetId:popup.targetId,flatten:true});
    await pause(250);
    const popupText=await command('Runtime.evaluate',{expression:'document.body.textContent',returnByValue:true},popupSession.sessionId);
    assert(popupText.result.value.includes('-40.0 dBc'));
    // Exercise each actual button in its own document. Clipboard behavior is
    // injected, not claimed as OS clipboard permission/trust qualification.
    const copyCheck=`(async()=>{
      Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{window.__copiedRf=text;}}});
      document.querySelector('[data-rf-copy]').click();await new Promise(r=>setTimeout(r,30));
      const modern=window.__copiedRf.includes('LIVE RF')&&window.__copiedRf.includes('-40.0 dBc')&&window.__copiedRf.includes('not calibrated RF IMD')&&document.querySelector('[data-rf-copy-status]').textContent==='Measurements copied.';
      navigator.clipboard.writeText=async()=>{throw new Error('denied for test');};
      document.execCommand=()=>{window.__legacyRf=document.querySelector('[data-rf-copy-fallback]').value;return true;};
      document.querySelector('[data-rf-copy]').click();await new Promise(r=>setTimeout(r,30));
      const legacy=window.__legacyRf.includes('Overload unknown')&&!document.querySelector('[data-rf-copy-fallback]');
      document.execCommand=()=>false;document.querySelector('[data-rf-copy]').click();await new Promise(r=>setTimeout(r,30));
      const field=document.querySelector('[data-rf-copy-fallback]');
      const manual=field&&field.selectionStart===0&&field.selectionEnd===field.value.length&&document.querySelector('[data-rf-copy-status]').textContent.includes('Ctrl+C');
      field?.remove();document.querySelector('[data-rf-copy-status]').textContent='';
      return {modern,legacy,manual:!!manual};
    })()`;
    assert.deepEqual(await evaluate(copyCheck),{modern:true,legacy:true,manual:true});
    const childCopyResult=await command('Runtime.evaluate',{expression:copyCheck,returnByValue:true,awaitPromise:true,userGesture:true},popupSession.sessionId);
    assert.deepEqual(childCopyResult.result.value,{modern:true,legacy:true,manual:true});
    await evaluate(`(()=>{const banner=document.createElement('div');banner.textContent='OFFLINE SYNTHETIC UI FIXTURE — NOT RF';banner.style.cssText='position:fixed;top:45px;left:8px;z-index:99999;padding:3px;background:#75480a;color:white;font:12px monospace';document.body.append(banner);})()`);
    await command('Runtime.evaluate',{expression:`document.querySelector('h2').textContent='OFFLINE SYNTHETIC — NOT RF · RF/IMD Monitor';`},popupSession.sessionId);
    assert(await evaluate(`(()=>{const c=document.querySelector('[data-rf-canvas]'),b=document.querySelector('[data-rf-popout]');return getComputedStyle(c).position==='relative'&&c.getBoundingClientRect().height===260&&c.getBoundingClientRect().bottom<=b.getBoundingClientRect().top;})()`),'RF canvas must occupy visible document flow, not inherit absolute panadapter positioning');
    const popupImage=await command('Page.captureScreenshot',{format:'png'},popupSession.sessionId);writeFileSync(join(output,`rf-popout-${width}.png`),Buffer.from(popupImage.data,'base64'));
    let correctionPopup=null;
    if(correction){
      await until(`document.querySelector('[data-correction-status]').textContent.startsWith('LIVE native readback')`);
      assert((await evaluate(`document.querySelector('[data-correction-summary]').textContent`)).includes('Curve age/configuration and overload coverage unknown'));
      await evaluate(`document.querySelector('[data-correction-popout]').click()`);
      const targets=await command('Target.getTargets');correctionPopup=targets.targetInfos.find(t=>t.title==='Saturn WDSP Correction / Phase Monitor');assert(correctionPopup);
      const cs=await command('Target.attachToTarget',{targetId:correctionPopup.targetId,flatten:true});
      await pause(250);
      const check=await command('Runtime.evaluate',{expression:`(async()=>{
        document.querySelector('h2').textContent='OFFLINE REAL-WDSP SYNTHETIC REPLAY — NOT RF';
        const canvases=[...document.querySelectorAll('canvas')];
        const colored=canvases.every(canvas=>{const p=canvas.getContext('2d').getImageData(0,0,canvas.width,canvas.height).data;let n=0;for(let i=0;i<p.length;i+=4)if((p[i]>160&&p[i+1]<150)||(p[i+1]>160&&p[i+2]<180))n++;return n>100;});
        Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{window.__copiedCorrection=text;}}});
        document.querySelector('[data-correction-copy]').click();await new Promise(r=>setTimeout(r,30));
        return{twoGraphs:canvases.length===2,colored,copy:window.__copiedCorrection.includes('Correction / Phase')&&window.__copiedCorrection.includes('Phase = atan2')&&window.__copiedCorrection.includes('not proof of RF improvement')};
      })()`,returnByValue:true,awaitPromise:true,userGesture:true},cs.sessionId);
      assert.deepEqual(check.result.value,{twoGraphs:true,colored:true,copy:true});
      const image=await command('Page.captureScreenshot',{format:'png'},cs.sessionId);writeFileSync(join(output,`correction-popout-${width}.png`),Buffer.from(image.data,'base64'));
      await evaluate(`document.querySelector('[data-correction-phase]').scrollIntoView({block:'center'});`);await pause(100);
      const mainImage=await call('Page.captureScreenshot',{format:'png'});writeFileSync(join(output,`correction-monitor-${width}.png`),Buffer.from(mainImage.data,'base64'));
      await evaluate(`window.__rfProbe.curveEmpty=true`);
      await until(`document.querySelector('[data-correction-status]').textContent.startsWith('NO CALIBRATION CURVE')`);
      await evaluate(`window.__rfProbe.curveEmpty=false`);
      await until(`document.querySelector('[data-correction-status]').textContent.startsWith('LIVE native readback')`);
    }
    await evaluate(`document.querySelector('[data-rf-canvas]').scrollIntoView({block:'center'});`);
    await pause(100);const image=await call('Page.captureScreenshot',{format:'png'});writeFileSync(join(output,`rf-monitor-${width}.png`),Buffer.from(image.data,'base64'));
    // A requested timing stimulus suppresses IMD in both actual documents,
    // including copy and RX-held views. Synthetic spectrum, not an RF claim.
    await evaluate(`window.__rfProbe.timing=true;window.__rfProbe.emit()`);
    await until(`document.querySelector('[data-rf-summary]').textContent.includes('Timing Measurement')`);
    assert(!(await evaluate(`document.querySelector('[data-rf-summary]').textContent`)).includes('IMD3 lower'));
    assert.equal(await evaluate(`document.querySelector('[data-rf-save]').disabled`),true);
    const timingCopy=await command('Runtime.evaluate',{expression:`(async()=>{
      await new Promise(r=>setTimeout(r,150));
      Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{window.__timingCopy=text;}}});
      document.querySelector('[data-rf-copy]').click();await new Promise(r=>setTimeout(r,20));
      return window.__timingCopy;
    })()`,returnByValue:true,awaitPromise:true,userGesture:true},popupSession.sessionId);
    assert(timingCopy.result.value.includes('Timing Measurement')&&timingCopy.result.value.includes('Two-tone IMD unavailable'));
    assert(!timingCopy.result.value.includes('IMD3 lower'));
    const timingImage=await command('Page.captureScreenshot',{format:'png'},popupSession.sessionId);
    writeFileSync(join(output,`timing-popout-${width}.png`),Buffer.from(timingImage.data,'base64'));
    await evaluate(`window.__rfProbe.stream=false`);await pause(correction?1600:950);
    assert((await evaluate(`document.querySelector('[data-rf-status]').textContent`)).includes('STALE'));
    if(correction)assert((await evaluate(`document.querySelector('[data-correction-status]').textContent`)).includes('STALE'));
    await evaluate(`window.__rfProbe.sockets.find(s=>!s.url.includes('/saturn/media')).text('tx_state:0,rx;trx:0,false;');window.__rfProbe.emit()`);
    await pause(250);assert((await evaluate(`document.querySelector('[data-rf-status]').textContent`)).includes('not live'));
    assert((await evaluate(`document.querySelector('[data-rf-summary]').textContent`)).includes('Two-tone IMD unavailable'));
    if(correction){assert((await evaluate(`document.querySelector('[data-correction-status]').textContent`)).includes('not live'));await command('Target.closeTarget',{targetId:correctionPopup.targetId});}
    const result=await evaluate(`({commands:window.__rfProbe.commands,errors:window.__rfProbe.errors})`);
    assert(!result.commands.some(s=>s.includes('trx:0,true')));assert.deepEqual(result.errors,[]);reports.push({width,...result});
    await command('Target.closeTarget',{targetId:popup.targetId});
  }
  const result={actualTemplate:true,actualBundle:true,syntheticRustSpectrum:true,syntheticTimingRebased:true,liveTrace:true,
    markers:true,imdPopout:true,timingImdSuppressed:true,timingPopoutCopy:true,copyBothViews:true,clipboardAndManualFallback:true,baselineOverlay:true,nativeCorrectionReplay:!!correction,phaseAndMagnitudeGraphs:!!correction,staleAndReleaseChecked:true,radioCommandsSent:0,output,reports};
  writeFileSync(join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({passed:true,output,widths:reports.map(r=>r.width)}));
}finally{clearTimeout(watchdog);await command('Browser.close').catch(()=>{});chrome.kill();}
