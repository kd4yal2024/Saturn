#!/usr/bin/env node
// Actual application + shipped renderer. Synthetic measurements only. No radio.
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { controlled } from './terrain-controlled.mjs';
import { cleanupFixtures } from './terrain-cleanup-fixtures.mjs';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const argument = name => process.argv.find(value => value.startsWith('--'+name+'='))?.slice(name.length+3);
const soakSeconds = Number(argument('soak') || process.env.SATURN_TERRAIN_SOAK_SECONDS || 0);
const output=argument('output') || process.env.SATURN_TERRAIN_OUTPUT || mkdtempSync(join(tmpdir(),'saturn-terrain-'));
mkdirSync(output,{recursive:true});
const bundle=readFileSync(argument('bundle') || join(root,'dist/saturn-remote-next.js'),'utf8');
const source=readFileSync(argument('template') || resolve(root,'../templates/saturn-remote-next.html'),'utf8');
const fixture = `
window.fixtureDone=false;
window.fixtureErrors=[];
window.addEventListener('error',e=>fixtureErrors.push(e.message));
window.addEventListener('unhandledrejection',e=>fixtureErrors.push(String(e.reason)));
window.fetch=async()=>({ok:false,status:404,json:async()=>({})});
window.WebSocket=class { static OPEN=1; constructor(){throw Error('Fixture must never open a socket');} };
`;
const adapter = `
window.fixtureIndex=0;
window.fixtureTimestamp=0;
window.fixtureFeed=(count=1)=>{
  const bins=new Float32Array(4096);
  for(let j=0;j<count;j++) {
    const t=window.fixtureIndex++;
    if(t%180===90 || t%180===91) { window.fixtureTimestamp+=33; continue; }
    for(let i=0;i<bins.length;i++) {
      const x=i/bins.length;
      let db=-129+2.7*Math.sin(i*1.99+t*.31)+1.4*Math.sin(i*.17+t*.47);
      const carrier=(at,level,w)=>level*Math.exp(-Math.pow((x-at)/w,2));
      db+=carrier(.18,64,.0009)+carrier(.53,75,.0012)+carrier(.536,61,.001);
      db+=carrier(.72+.025*Math.sin(t*.025),52,.0011);
      db+=carrier(.36,35+8*Math.sin(t*.35),.016)*(0.75+0.25*Math.sin(i*.8+t*.13));
      db+=carrier(.86,9,.0012);
      if(t%150===35) db+=25;
      if(t%120>=30 && t%120<42) db+=carrier(.62,65,.002);
      if(x>.94 && x<.98) db=-120+20*Math.floor((t%120)/30);
      bins[i]=db;
    }
    acceptSpectrumFrame(bins,window.fixtureTimestamp,t,4096);
    window.fixtureTimestamp+=33;
  }
};
(async()=>{
  await init();
  applyTheme('dark',false);
  state.connected=true; state.iqStreaming=false; state.demoMode=false;
  state.dds=14200000; state.vfoA=14200000; state.sampleRate=48000; state.displayZoom=1; state.frequencyLock=false;
  fixtureFeed(600);
  state.terrain=_next.normalizeTerrain({mode:'3d',diagnostics:true,quality:'balanced',cleanup:0});
  if(!setTerrainMode('3d',false)) throw Error(terrainFailure);
  updateAxes(); updateFilterOverlay(); renderBandEdges();
  const label=document.createElement('div');label.textContent='SYNTHETIC FIXTURE — NOT LIVE RF';label.style.cssText='position:fixed;right:4px;top:2px;z-index:99999;background:#382009;color:#ffe1a1;padding:3px 7px;font:11px monospace';document.body.appendChild(label);
  await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
  renderTerrain(performance.now());
  window.fixtureDone=true;
})().catch(e=>{window.fixtureErrors.push(String(e.stack||e));window.fixtureDone=true;});
`;
let html=source.replace(/<script\s+src="\/remote-assets\/remote-next\.js[^]*?<\/script>/,()=>`<script>${bundle.replaceAll('</script','<\\/script')}</script>`);
html=html.replace('<head>','<head><script>'+fixture+'</script>');
html=html.replace('    void init();',adapter);
const page=join(output,'fixture.html'); writeFileSync(page,html);
const chrome=spawn(process.env.CHROMIUM || 'google-chrome',[
  '--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run',
  '--use-angle=swiftshader','--enable-unsafe-swiftshader','--remote-debugging-pipe',
  '--disable-background-timer-throttling','--disable-renderer-backgrounding',
  `--user-data-dir=${join(output,'profile')}`,'about:blank',
],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let id=0, pending=new Map(), buffer='';
let stderr=''; chrome.stderr.on('data',d=>stderr+=d);
chrome.on('error',e=>{console.error(e);process.exitCode=1;});
chrome.stdio[4].on('data',data=>{
  buffer+=data.toString();let end;
  while((end=buffer.indexOf('\0'))>=0){const raw=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!raw)continue;
    const msg=JSON.parse(raw);if(msg.id){const p=pending.get(msg.id);if(p){pending.delete(msg.id);msg.error?p.reject(Error(JSON.stringify(msg.error))):p.resolve(msg.result);}}
  }
});
function cmd(method,params={},sessionId){return new Promise((resolve,reject)=>{const request={id:++id,method,params};if(sessionId)request.sessionId=sessionId;pending.set(id,{resolve,reject});chrome.stdio[3].write(JSON.stringify(request)+'\0');});}
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const report={synthetic:true,renderer:'Chromium SwiftShader (software WebGL2), not hardware performance',checks:[],screenshots:[],errors:[]};
const watchdog=setTimeout(()=>{console.error('Validator timeout',stderr);chrome.kill();process.exit(1);},(soakSeconds+180)*1000);
try {
  const {targetId}=await cmd('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await cmd('Target.attachToTarget',{targetId,flatten:true});
  const call=(method,params={})=>cmd(method,params,sessionId);
  const evaluate=async expression=>{
    const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
    if(r.exceptionDetails)throw Error(r.exceptionDetails.text+': '+r.exceptionDetails.exception?.description);
    return r.result.value;
  };
  await call('Page.enable');await call('Runtime.enable');
  await call('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false});
  await call('Page.navigate',{url:pathToFileURL(page).href});
  for(let i=0;i<100;i++){await sleep(100);if(await evaluate('window.fixtureDone===true'))break;}
  const errors=await evaluate('window.fixtureErrors');if(errors?.length)throw Error(errors.join('\n'));
  if(!await evaluate('window.fixtureDone && state.terrainActive'))throw Error('3D initialization did not finish: '+await evaluate('document.body.innerText.slice(-1000)'));
  report.checks.push('actual application initializes the shipped 3D renderer through live display adapter');
  if(argument('cleanup')) {
    await cleanupFixtures({evaluate,call,output,report,phase:argument('cleanup')});
  } else if(argument('controlled')) {
    await controlled({evaluate,call,output,report,phase:argument('controlled')});
  } else {
  if(process.env.SATURN_TERRAIN_BASELINE) {
    const baseline=readFileSync(process.env.SATURN_TERRAIN_BASELINE,'utf8');
    const start=baseline.indexOf('    class SpectrumRenderer {'),end=baseline.indexOf('    const spectrumRenderer =',start);
    if(start<0||end<0)throw Error('Invalid frozen Traditional baseline');
    report.traditionalBaseline=await evaluate(`(()=>{
      const baseline=(()=>{${baseline.slice(start,end)};return {SpectrumRenderer,WaterfallRenderer};})();
      const active=state.terrainActive;state.terrainActive=false;
      const host=document.createElement('section');host.style.cssText='position:fixed;left:0;top:0;width:1024px;height:760px;z-index:99998;background:#071017;color:#d8e5f0;font:14px monospace';
      host.innerHTML='<h3 style="margin:10px">SYNTHETIC Traditional baseline / current — pixel comparison</h3><div style="display:flex;justify-content:space-around"><span>Frozen original renderer</span><span>Integrated renderer in Traditional mode</span></div>';
      document.body.appendChild(host);
      const make=(x,y,w,h)=>{const box=document.createElement('div');box.style.cssText='position:absolute;left:'+x+'px;top:'+y+'px;width:'+w+'px;height:'+h+'px';const canvas=document.createElement('canvas');box.appendChild(canvas);host.appendChild(box);return canvas;};
      const oldSpec=new baseline.SpectrumRenderer(make(16,65,480,180)),newSpec=new SpectrumRenderer(make(528,65,480,180));
      const oldWf=new baseline.WaterfallRenderer(make(16,245,480,480)),newWf=new WaterfallRenderer(make(528,245,480,480));
      const bins=new Float32Array(1024);
      for(let t=0;t<600;t++) {
        for(let i=0;i<bins.length;i++) {const x=i/bins.length;bins[i]=-135+2*Math.sin(i*.79+t*.31)+65*Math.exp(-Math.pow((x-.21)/.002,2))+48*Math.exp(-Math.pow((x-.57-.01*Math.sin(t*.03))/.005,2))+25*Math.exp(-Math.pow((x-.7)/.025,2));}
        oldWf.pushLine(bins,-150,-40,'classic');newWf.pushLine(bins,-150,-40,'classic');
      }
      oldSpec.render(bins,-150,-40);newSpec.render(bins,-150,-40);oldWf.render();newWf.render();
      let pixels=0,maxDifference=0;
      for(const [a,b] of [[oldSpec,newSpec],[oldWf,newWf]]) {
        const length=a.canvas.width*a.canvas.height*4,A=new Uint8Array(length),B=new Uint8Array(length);
        a.gl.readPixels(0,0,a.canvas.width,a.canvas.height,a.gl.RGBA,a.gl.UNSIGNED_BYTE,A);b.gl.readPixels(0,0,b.canvas.width,b.canvas.height,b.gl.RGBA,b.gl.UNSIGNED_BYTE,B);
        for(let i=0;i<length;i++)maxDifference=Math.max(maxDifference,Math.abs(A[i]-B[i]));pixels+=length/4;
      }
      window.fixtureBaselineCleanup=()=>{state.terrainActive=active;host.remove();for(const r of [oldSpec,newSpec,oldWf,newWf])r.gl.getExtension('WEBGL_lose_context')?.loseContext();};
      if(maxDifference>0)throw Error('Traditional changed from frozen baseline by '+maxDifference);
      return {pixels,maxDifference};
    })()`);
    report.traditionalBaseline.templateSha256=createHash('sha256').update(baseline).digest('hex');
    const shot=await call('Page.captureScreenshot',{format:'png',clip:{x:0,y:0,width:1024,height:760,scale:1}});
    writeFileSync(join(output,'traditional-comparison.png'),Buffer.from(shot.data,'base64'));
    await evaluate('fixtureBaselineCleanup()');
    report.checks.push('Traditional spectrum/waterfall pixels exactly match the frozen original renderer');
  }

  // Numerical and GPU pixel checks happen only here, never in production RAF.
  report.gpu=await evaluate(`(()=>{
    const r=terrainRenderer, h=spectrumHistory, gl=r.gl;
    r.render(performance.now(),$('spectrum-shell').getBoundingClientRect().height,true);
    const pixel=new Uint8Array(4); gl.readPixels(Math.floor(r.canvas.width*.53),10,1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
    if(gl.getError()!==gl.NO_ERROR)throw Error('WebGL error');
    const canvasRect=r.canvas.getBoundingClientRect();
    const upper=Math.round($('spectrum-shell').getBoundingClientRect().height/canvasRect.height*r.canvas.height);
    const lower=r.canvas.height-upper;
    for(const y of [0,10,Math.floor(lower/3),lower-1])for(const x of [20,Math.floor(r.canvas.width*.18),Math.floor(r.canvas.width*.53),r.canvas.width-10]) {
      const age=Math.max(0,Math.floor((1-(y+1)/lower)*512)),ageHi=Math.min(h.count,Math.max(age+1,Math.ceil((1-y/lower)*512)));
      const lo=Math.floor(x*h.width/r.canvas.width),hi=Math.max(lo+1,Math.floor((x+1)*h.width/r.canvas.width));
      let db=-1000,missing=false;
      for(let a=age;a<ageHi;a++) {
        const row=h.row(a);if(!row){missing=true;continue;}
        for(let bin=lo;bin<hi;bin++){missing=missing||row[bin]<-999;db=Math.max(db,row[bin]);}
      }
      const expected=missing?[20,22,26]:terrainColor(Math.pow(Math.max(0,Math.min(1,(db-state.terrain.floor)/(state.terrain.ceiling-state.terrain.floor))),state.terrain.gamma),state.terrain.palette);
      gl.readPixels(x,y,1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
      if(expected.some((v,i)=>Math.abs(v-pixel[i])>3))throw Error('Numeric waterfall color/chronology mismatch age='+age+' x='+x+' expected='+expected+' got='+Array.from(pixel));
    }
    const before=r.uploadRows; fixtureFeed(1); r.render(performance.now(),$('spectrum-shell').getBoundingClientRect().height,true);
    if(r.uploadRows-before!==1)throw Error('Expected single new-row upload');
    const front=r.project(0,Math.floor(r.columns*.18)), frontHit=r.pick(front[0],front[1]);
    if(!frontHit || Math.abs(frontHit.fraction-Math.floor(r.columns*.18)/(r.columns-1))>1/r.columns)throw Error('Front projected tuning frequency mismatch');
    const checks=[];
    for(const age of [0,3,10,30])for(const col of [100,280,550,800]) {
      const p=r.project(age,col), hit=r.pick(p[0],p[1]);
      if(hit && hit.age===age && Math.abs(hit.fraction-col/(r.columns-1))>1/r.columns)throw Error('Projection/picking frequency mismatch');
    }
    return {pixel:Array.from(pixel),diagnostics:r.diagnostics(),singleRowUploads:r.uploadRows-before};
  })()`);
  report.checks.push('GPU numerical waterfall mapping / chronology agrees within 3 RGB units; single-row uploads; front projection picking within one geometry column');
  report.amplitude=await evaluate(`(()=>{
    const saved={...state.terrain};state.terrain={...saved,floor:-140,ceiling:-40,height:.65,gamma:1,smoothing:0};
    const results=[];
    for(const db of [-120,-100,-80]) {
      spectrumHistory.clear('Known amplitude fixture');
      acceptSpectrumFrame(new Float32Array(4096).fill(db),fixtureTimestamp++,fixtureIndex++,4096);
      const r=terrainRenderer;r.configure(state.terrain);r.render(performance.now(),layoutTerrainCanvas(),true);
      const rect=r.canvas.getBoundingClientRect(),upper=Math.round($('spectrum-shell').getBoundingClientRect().height/rect.height*r.canvas.height),lower=r.canvas.height-upper;
      const bytes=new Uint8Array(upper*4);r.gl.readPixels(Math.floor(r.canvas.width/2),lower,1,upper,r.gl.RGBA,r.gl.UNSIGNED_BYTE,bytes);
      let raised=0;for(let y=0;y<upper;y++)if(bytes[y*4+2]>30)raised=y+1;
      const expected=(db+140)/100*.65*.75/2*upper;
      if(Math.abs(raised-expected)>2)throw Error('GPU height disagrees with dB level: '+db+' height='+raised+' expected='+expected);
      const color=terrainColor((db+140)/100,'reference');
      if(color.some((v,i)=>Math.abs(v-bytes[(raised-1)*4+i])>3))throw Error('GPU color disagrees with sampled dB level');
      results.push({db,raisedPixels:raised,expectedPixels:expected});
    }
    const raw=spectrumHistory.row(0)[0];
    state.terrain.gamma=2;terrainRenderer.configure(state.terrain);terrainRenderer.render(performance.now(),layoutTerrainCanvas(),true);
    if(spectrumHistory.row(0)[0]!==raw)throw Error('Gamma changed raw measurement');
    state.terrain=saved;fixtureFeed(600);terrainRenderer.configure(saved);terrainRenderer.render(performance.now(),layoutTerrainCanvas(),true);
    return results;
  })()`);
  report.checks.push('actual GPU leading-outline heights and colors track known 20 dB amplitude steps within 2 pixels / 3 RGB units');
  report.impulse=await evaluate(`(()=>{
    const r=terrainRenderer;r.configure(state.terrain);r.render(performance.now(),layoutTerrainCanvas(),true);
    const rect=r.canvas.getBoundingClientRect(),upper=Math.round($('spectrum-shell').getBoundingClientRect().height/rect.height*r.canvas.height),lower=r.canvas.height-upper;
    const nearestRows=new Set(Array.from({length:lower},(_,y)=>Math.floor((1-(y+.5)/lower)*512)));
    const age=Array.from({length:490},(_,i)=>i+10).find(a=>!nearestRows.has(a));
    if(age===undefined)throw Error('Impulse fixture must compress history');
    spectrumHistory.clear('Single-row compressed impulse fixture');
    const bins=new Float32Array(4096);
    for(let t=0;t<512;t++) {bins.fill(t===511-age?-60:-130);acceptSpectrumFrame(bins,fixtureTimestamp,fixtureIndex++,4096);fixtureTimestamp+=33;}
    if(spectrumHistory.row(age)[0]!==-60)throw Error('Impulse fixture chronology is incorrect');
    r.render(performance.now(),layoutTerrainCanvas(),true);
    const bytes=new Uint8Array(lower*4);r.gl.readPixels(Math.floor(r.canvas.width/2),0,1,lower,r.gl.RGBA,r.gl.UNSIGNED_BYTE,bytes);
    const rows=[];for(let y=0;y<lower;y++)if(bytes[y*4]>200)rows.push(y);
    if(rows.length<1||rows.length>3)throw Error('Compressed single-row impulse was lost or lingered: '+rows);
    fixtureFeed(600);r.render(performance.now(),layoutTerrainCanvas(),true);
    return {sourceAge:age,visiblePixelRows:rows,nearestSamplingWouldLoseIt:true};
  })()`);
  report.checks.push('a single-row impulse skipped by nearest sampling survives compressed waterfall rendering without a lingering trail');
  report.switches=await evaluate(`(()=>{
    const snapshot=JSON.stringify({radio:currentRadioPrefs(),dds:state.dds,zoom:state.displayZoom,history:spectrumHistory.revision,paused:state.displayPaused});
    const times=[];const renderer=terrainRenderer;
    for(let i=0;i<100;i++) {const t=performance.now();setTerrainMode('traditional',false);setTerrainMode('3d',false);times.push(performance.now()-t);}
    if(renderer!==terrainRenderer)throw Error('Switches recreated GPU cache');
    if(snapshot!==JSON.stringify({radio:currentRadioPrefs(),dds:state.dds,zoom:state.displayZoom,history:spectrumHistory.revision,paused:state.displayPaused}))throw Error('View switch changed operational state');
    return {count:100,maxMs:Math.max(...times),meanMs:times.reduce((a,b)=>a+b,0)/times.length,diagnostics:terrainRenderer.diagnostics()};
  })()`);
  report.checks.push('100 mode cycles preserve radio settings, mapping, history, pause and one renderer cache');
  report.controls=await evaluate(`(()=>{
    const commands=[];const previousSend=sendTci;sendTci=(text)=>{commands.push(text);return false;};
    const raw=new Float32Array(spectrumHistory.latestRaw),revision=spectrumHistory.revision;
    const beforeRadio=JSON.stringify(currentRadioPrefs());
    $('view-traditional').click();$('view-3d').click();
    for(const [id,value] of [['height','0.5'],['cleanup','0.6'],['gridOpacity','0.3'],['elevation','30'],['gamma','1.2'],['smoothing','0'],['palette','ember'],['palette','reference-dark'],['palette','reference']]) {
      $('terrain-'+id).value=value;$('terrain-'+id).dispatchEvent(new Event('input',{bubbles:true}));
    }
    for(const [id,value] of [['floor','-145'],['ceiling','-35'],['depth','96']]) { $('terrain-'+id).value=value;$('terrain-'+id).dispatchEvent(new Event('change',{bubbles:true})); }
    if(state.terrain.floor!==-145 || state.terrain.ceiling!==-35 || state.terrain.depth!==96)throw Error('Numeric 3D controls failed to commit');
    $('display-pause').click();$('view-traditional').click();$('view-3d').click();
    if(!state.displayPaused)throw Error('Switch lost pause');$('display-pause').click();
    $('terrain-cleanupBaseline').value='-129';$('terrain-cleanupBaseline').dispatchEvent(new Event('change',{bubbles:true}));
    if(state.terrain.cleanupBaseline!==-129)throw Error('Manual cleanup baseline failed');
    $('terrain-noise-estimate').click();
    if(state.terrain.cleanupBaseline!==null||spectrumHistory.noiseFloor===null)throw Error('Noise re-estimate failed');
    $('terrain-fit-range').click();
    const fitted=_next.fitTerrainRange(raw);
    if(!fitted || state.terrain.floor!==fitted.floor || state.terrain.ceiling!==fitted.ceiling || state.terrain.gamma!==fitted.gamma)throw Error('One-shot range fit failed');
    if(!$('terrain-fit-status').textContent.includes('Held until'))throw Error('Range fit stability is not explained');
    $('terrain-reset').click();
    if(commands.length)throw Error('Presentation controls issued commands: '+commands);
    if(beforeRadio!==JSON.stringify(currentRadioPrefs()))throw Error('Presentation altered radio state');
    if(revision!==spectrumHistory.revision || raw.some((v,i)=>v!==spectrumHistory.latestRaw[i]))throw Error('Presentation altered measurements');
    sendTci=previousSend;
    state.terrain={...state.terrain,quality:'balanced',diagnostics:true};
    const before=terrainRenderer.diagnostics().draws;
    Object.defineProperty(document,'hidden',{value:true,configurable:true});renderTerrain(performance.now());delete document.hidden;
    if(terrainRenderer.diagnostics().draws!==before)throw Error('Hidden display drew a frame');
    return {commands,measurementsPreserved:true,pausePreserved:true,hiddenGuard:true};
  })()`);
  report.checks.push('actual selector, camera, palette, range fit, reset and pause controls send no commands or measurement edits; hidden guard suppresses drawing');
  await evaluate(`(()=>{
    const shell=$('waterfall-shell'),rect=shell.getBoundingClientRect();
    shell.dispatchEvent(new PointerEvent('pointermove',{clientX:rect.left+10,clientY:rect.top+2}));
    if($('terrain-cursor').hidden)throw Error('Valid waterfall sample cursor missing');
    shell.dispatchEvent(new PointerEvent('pointermove',{clientX:rect.right+20,clientY:rect.top+2}));
    if(!$('terrain-cursor').hidden)throw Error('Cursor mislabeled out-of-span sample');
  })()`);
  report.checks.push('raw cursor appears on sampled data and hides outside the frequency span');
  report.passbandPresentation=await evaluate(`(()=>{
    const mode=state.mode,low=state.filterLow,high=state.filterHigh;
    const results=[];
    for(const selected of ['USB','LSB']) {
      state.mode=selected;state.filterLow=50;state.filterHigh=3050;updateFilterOverlay();
      const coordinates=displayPassbandHz(),element=$('waterfall-filter-window'),label=$('filter-window-label');
      const before={left:element.style.left,width:element.style.width};
      setTerrainMode('traditional',false);updateFilterOverlay();
      if(element.style.left!==before.left||element.style.width!==before.width)throw Error('3D changed passband frequency coordinates');
      setTerrainMode('3d',false);updateFilterOverlay();
      const style=getComputedStyle(element);
      if(style.borderRadius!=='0px'||style.backgroundColor!=='rgba(98, 208, 255, 0.04)'||style.backgroundImage!=='none')throw Error('Passband is not the restrained rectangular overlay');
      if(!element.title.includes('50–3050 Hz')||!label.title.includes('50–3050 Hz'))throw Error('Actual filter values missing from tooltip');
      if(selected==='LSB'&&coordinates.endHz>0)throw Error('LSB passband was recentered');
      results.push({mode:selected,coordinates,...before,fill:style.backgroundColor,label:label.textContent});
    }
    state.mode=mode;state.filterLow=low;state.filterHigh=high;updateFilterOverlay();return results;
  })()`);
  report.tuning=await evaluate(`(()=>{
    const element=$('spectrum-shell'),rect=element.getBoundingClientRect();
    terrainRenderer.configure(state.terrain);terrainRenderer.render(performance.now(),layoutTerrainCanvas(),true);
    const col=Math.floor(terrainRenderer.columns*.18),p=terrainRenderer.project(0,col);
    const x=rect.left+p[0]*rect.width,y=rect.top+p[1]*rect.height;
    const center=state.dds,span=displaySpanHz(),offset=clickTuneCarrierOffsetHz();
    const commands=[],previousSend=sendTci;sendTci=text=>{commands.push(text);return false;};
    const fire=(name,px,py)=>element.dispatchEvent(new PointerEvent(name,{bubbles:true,pointerId:777,pointerType:'mouse',button:0,buttons:name==='pointerup'?0:1,clientX:px,clientY:py}));
    fire('pointerdown',x,y);
    const expectedStart=snapTuneFrequencyHz(center+(col/(terrainRenderer.columns-1)-.5)*span-offset);
    if(Math.abs(state.dds-expectedStart)>10)throw Error('Upper pointer selected wrong frequency');
    if(spectrumHistory.count!==0)throw Error('QSY retained incompatible history');
    fire('pointermove',x+20,y);
    const expectedEnd=snapTuneFrequencyHz(center+(col/(terrainRenderer.columns-1)-.5)*span-offset+20*span/rect.width);
    if(Math.abs(state.dds-expectedEnd)>10)throw Error('Tuning drag stopped after QSY history boundary');
    const tx=$('ptt-btn').getBoundingClientRect();fire('pointerup',x+20,tx.top+tx.height/2);
    if(commands.some(text=>/^(trx|ptt|mox|key):/i.test(text)))throw Error('Display drag issued TX command');
    setFrequency(center,false,{send:false,rememberBand:false});sendTci=previousSend;
    fixtureFeed(600);renderTerrain(performance.now());
    return {expectedStart,expectedEnd,commands,releaseOverTxSafe:true};
  })()`);
  report.checks.push('projected upper pointer tuning and drag survive QSY segmentation; release over TX coordinates issues no TX command');
  for(const scenario of [{name:'desktop',width:1440,height:1000,phone:false},{name:'phone',width:390,height:844,phone:true}]) {
    await call('Emulation.setDeviceMetricsOverride',{width:scenario.width,height:scenario.height,deviceScaleFactor:1,mobile:scenario.phone});
    await evaluate(`applyLayout('${scenario.phone?'phone':'desktop'}',false,false); syncDisplayWorkspaceRatio(); updateAxes(); updateFilterOverlay();`);
    await sleep(200);
    await evaluate(`renderTerrain(performance.now()); document.querySelector('.display-card').scrollIntoView({block:'start'});`);
    await sleep(100);
    const shot=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
    const name=`${scenario.name}.png`;writeFileSync(join(output,name),Buffer.from(shot.data,'base64'));report.screenshots.push(name);
    const layout=await evaluate(`({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,display:$('terrain-canvas').getBoundingClientRect().toJSON(),tx:$('ptt-btn')?.getBoundingClientRect().toJSON()})`);
    if(layout.scrollWidth>scenario.width+1)throw Error('Horizontal overflow on '+scenario.name);
    report[scenario.name]=layout;
  }
  const duration=soakSeconds;
  const soakQuality=['auto','performance','balanced','high'].includes(process.env.SATURN_TERRAIN_SOAK_QUALITY) ? process.env.SATURN_TERRAIN_SOAK_QUALITY : 'auto';
  if(duration>0){
    await call('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false});
    await evaluate(`applyLayout('desktop',false,false);state.terrain.quality='${soakQuality}';window.fixtureTimer=setInterval(()=>fixtureFeed(1),33);`);
    const started=Date.now();report.soak={seconds:duration,quality:soakQuality,samples:[]};
    while(Date.now()-started<duration*1000){await sleep(Math.min(10000,duration*1000-(Date.now()-started)));
      const sample=await evaluate(`({at:performance.now(),diag:terrainDiagnostics(),accepted:spectrumHistory.accepted,historyBytes:spectrumHistory.bytes,errors:fixtureErrors,heap:performance.memory?.usedJSHeapSize})`);
      report.soak.samples.push(sample);writeFileSync(join(output,'progress.json'),JSON.stringify(report,null,2));
      if(sample.errors.length||!sample.diag)throw Error('Soak renderer failed');
      if(report.soak.samples.length%6===0)console.log('Soak',Math.round((Date.now()-started)/1000),'s',sample.diag.fps,'fps');
    }
    await evaluate('clearInterval(fixtureTimer)');
  }
  report.inactiveCache=await evaluate(`(async()=>{
    const old=terrainRenderer,revision=spectrumHistory.revision;setTerrainMode('traditional',false);
    const draws=old.diagnostics().draws;
    await new Promise(r=>setTimeout(r,30500));
    if(terrainRenderer!==null || old.diagnostics().draws!==draws)throw Error('Inactive 3D cache did not stop and expire');
    if(spectrumHistory.revision!==revision)throw Error('Cache expiration lost numerical history');
    const start=performance.now();if(!setTerrainMode('3d',false))throw Error('Expired cache failed to rebuild');
    return {expired:true,inactiveDraws:old.diagnostics().draws-draws,rebuildMs:performance.now()-start};
  })()`);
  report.checks.push('inactive renderer draws stop and the real 30-second cache expiry releases the context; numerical history rebuilds');
  report.pageLifecycle=await evaluate(`(()=>{
    const previous=terrainRenderer,revision=spectrumHistory.revision;
    window.dispatchEvent(new PageTransitionEvent('pagehide',{persisted:true}));
    if(terrainRenderer)throw Error('Page hide retained renderer');
    window.dispatchEvent(new PageTransitionEvent('pageshow',{persisted:true}));
    if(!terrainRenderer || terrainRenderer===previous || spectrumHistory.revision!==revision)throw Error('BFCache restoration failed');
    return {restored:true,historyPreserved:true};
  })()`);
  report.checks.push('pagehide/pageshow handlers restore a fresh renderer and retained history (synthetic lifecycle events)');
  report.context=await evaluate(`(async()=>{
    const revision=spectrumHistory.revision;
    terrainRenderer.gl.getExtension('WEBGL_lose_context').loseContext();
    await new Promise(r=>setTimeout(r,100));
    if(state.terrainActive || !terrainFailure)throw Error('Context loss did not fall back');
    if(spectrumHistory.revision!==revision)throw Error('Context loss destroyed history');
    if(!setTerrainMode('3d',false))throw Error('Explicit recovery failed');
    return {recovered:state.terrainActive,historyPreserved:spectrumHistory.revision===revision};
  })()`);
  report.checks.push('context loss falls back to Traditional; explicit retry rebuilds numerical history');
  report.sharedContextLoss=await evaluate(`(async()=>{
    const revision=spectrumHistory.revision;
    spectrumRenderer.gl?.getExtension('WEBGL_lose_context').loseContext();
    waterfallRenderer.gl?.getExtension('WEBGL_lose_context').loseContext();
    terrainRenderer.gl.getExtension('WEBGL_lose_context').loseContext();
    await new Promise(r=>setTimeout(r,150));
    if(state.terrainActive || spectrumRenderer.backend!=='canvas2d' || waterfallRenderer.backend!=='canvas2d')throw Error('Shared context loss did not recover usable Traditional Canvas2D');
    if(spectrumHistory.revision!==revision)throw Error('Shared context loss lost numerical history');
    const expected=waterfallRenderer.colorForDb(spectrumHistory.row(0)[0],state.waterfallFloorDb,state.waterfallCeilingDb,state.waterfallPalette,state.waterfallContrast);
    const actual=waterfallRenderer.ctx.getImageData(0,0,1,1).data;
    if(expected.some((v,i)=>Math.abs(v-actual[i])>1))throw Error('Canvas2D recovery failed to rebuild measured history');
    if(!setTerrainMode('3d',false))throw Error('3D did not recover after shared context loss');
    return {traditionalCanvas2D:true,historyPreserved:true,recovered3D:true};
  })()`);
  report.checks.push('simulated loss of all WebGL contexts recovers Traditional Canvas2D with numerical history; 3D can be retried');
  report.unsupported=await evaluate(`(()=>{
    const revision=spectrumHistory.revision;setTerrainMode('traditional',false);disposeTerrain();
    const original=HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext=function(kind,...args){return this.id==='terrain-canvas' && kind==='webgl2' ? null : original.call(this,kind,...args);};
    const result=setTerrainMode('3d',false);
    HTMLCanvasElement.prototype.getContext=original;
    if(result || state.terrainActive || spectrumHistory.revision!==revision)throw Error('Unsupported WebGL2 fallback failed');
    return {traditional:!state.terrainActive,reason:terrainFailure,historyPreserved:spectrumHistory.revision===revision};
  })()`);
  report.checks.push('unsupported WebGL2 stays Traditional with a visible reason and retained history');
  }
  report.errors=await evaluate('fixtureErrors');if(report.errors.length)throw Error(report.errors.join('\n'));
  report.pass=true;
}catch(error){report.pass=false;report.errors.push(String(error.stack||error));process.exitCode=1;}
finally{writeFileSync(join(output,'result.json'),JSON.stringify(report,null,2));clearTimeout(watchdog);await cmd('Browser.close').catch(()=>{});chrome.kill();}
console.log(JSON.stringify(report,null,2));console.log('Output:',output);
