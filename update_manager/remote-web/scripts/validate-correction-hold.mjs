#!/usr/bin/env node
// Shipped monitor and panel markup in Chrome. Synthetic curves; no radio.
import {createServer} from 'node:http';
import {readFileSync,writeFileSync,mkdtempSync,mkdirSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
import {chromeSession,pause} from './lib/chrome-session.mjs';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const output=process.argv.find(x=>x.startsWith('--output='))?.slice(9)||mkdtempSync(join(tmpdir(),'saturn-correction-hold-'));
mkdirSync(output,{recursive:true});
const panel=readFileSync(join(root,'../templates/saturn-remote-next.html'),'utf8').match(/<section class="setup-section" id="ps-correction-monitor"[\s\S]*?<\/section>/)?.[0];assert.ok(panel);
const bundle=readFileSync(join(root,'dist/saturn-remote-next.js'));
const fixture={format:'saturn-ps-correction-v1',boot:'10',serial:'1',snapshotNs:'250000000',publishedNs:'300000000',sourcePin:'synthetic-not-rf',backend:'DirectXdma',centerHz:7200000,commandedWatts:10,attenuationCommandDb:25,psRequested:true,keyed:true,nativeApplied:true,calibrations:2,nativeState:7,nativeSamples:4096,phaseReferenceDeg:10,diagnosticDrops:'0',curveAge:'unknown',rfImdQualified:false,
 measured:Array.from({length:1024},(_,i)=>{const x=i/1023;return[x,1+.1*x*x,Math.cos((10+5*x)*Math.PI/180),Math.sin((10+5*x)*Math.PI/180)];}),correction:Array.from({length:512},(_,i)=>[i/511,1+.1*(i/511)**2,i/511,5*(i/511)])};
const html=`<!doctype html><html><head><meta charset="utf-8"><style>body{background:#07111f;color:#d8e7f7;font:14px system-ui}section{width:1100px}pre{white-space:pre-wrap}</style></head><body>${panel}<script>
window.__sockets=0;window.WebSocket=class{constructor(){window.__sockets++;throw Error('No radio in fixture')}};
window.__exports=[];const create=URL.createObjectURL.bind(URL);URL.createObjectURL=b=>{__exports.push(b);return create(b)};
window.__copy=[];Object.defineProperty(navigator,'clipboard',{value:{writeText:async t=>__copy.push(t)}});
const open=window.open.bind(window);window.open=(...a)=>window.__popup=open(...a);</script><script src="/bundle.js"></script><script>
const api=window.SaturnRemoteNext||window.SaturnRemoteNextBundle;
window.m=new api.CorrectionMonitorState();window.draw=api.mountCorrectionMonitor(document.getElementById('ps-correction-monitor'),m);
window.fixture=${JSON.stringify(fixture)};window.feed=(r,t)=>m.observe('saturn_ps_correction:'+JSON.stringify(r)+';',1,t);
m.reset(1);m.observe('saturn_ps_correction_caps:wdsp-ampview-v1;',1,50);draw(true,true,80);m.request('1',1,100);m.observe('saturn_ps_spectrum_clock:1,10,100000000;',1,120);window.__ready=true;</script></body></html>`;
const server=createServer((req,res)=>{if(req.url==='/bundle.js'){res.setHeader('Content-Type','text/javascript');res.end(bundle)}else if(req.url==='/'){res.setHeader('Content-Type','text/html');res.end(html)}else{res.writeHead(404);res.end()}});
await new Promise(done=>server.listen(0,'127.0.0.1',done));const checks=[];const check=(name,value)=>{assert.ok(value,name);checks.push(name)};
try{
 await chromeSession(output,async({call,command,evaluate})=>{
  await call('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/`});
  for(let i=0;i<100;i++){if(await evaluate('window.__ready===true').catch(()=>false))break;await pause(30)}
  check('Shipped monitor mounted',await evaluate('window.__ready===true'));
  await evaluate(`document.querySelector('[data-correction-popout]').click();feed(fixture,400);draw(true,true,400)`);
  check('Separate pop-out without opener',await evaluate('!!window.__popup&&!__popup.closed&&__popup.opener===null'));
  check('Live curves reach both views',await evaluate(`document.querySelector('[data-correction-status]').textContent.includes('LIVE')&&__popup.document.querySelector('pre').textContent.includes('Calibration counter 2')`));
  await evaluate(`m.setTransmitting(false);feed({...fixture,serial:'2',snapshotNs:'450000000',publishedNs:'460000000',keyed:false,nativeApplied:false,nativeSamples:0,calibrations:0,nativeState:0,measured:[],correction:[]},470);draw(true,false,480)`);
  check('Empty cleanup preserves held curves in both views',await evaluate(`document.querySelector('[data-correction-status]').textContent.includes('RX / held last valid TX curve')&&__popup.document.querySelector('p').textContent.includes('RX / held last valid TX curve')&&document.querySelector('[data-correction-summary]').textContent.includes('Calibration counter 2')&&__popup.document.querySelector('pre').textContent===document.querySelector('[data-correction-summary]').textContent`));
  check('Current status still reports correction off',await evaluate('m.status(1,480,true,false).nativeApplied===false&&m.status(1,480,true,false).nativeSamples===0'));
  check('Magnitude and phase canvases rendered',await evaluate(`Array.from(__popup.document.querySelectorAll('canvas')).every(c=>c.width>500&&c.height>200)`));
  await evaluate(`document.querySelector('[data-correction-export]').click();__popup.document.querySelector('[data-correction-export]').click()`);
  const exports=await evaluate('Promise.all(__exports.map(async b=>JSON.parse(await b.text())))');
  check('Both JSON exports match the held display',exports.length===2&&exports.every(r=>r.frame.serial==='1'&&r.frame.correction.length===512&&r.displayStatus.includes('held last valid TX curve')));
  await evaluate(`document.querySelector('[data-correction-copy]').click();Object.defineProperty(__popup.navigator,'clipboard',{value:{writeText:async t=>__copy.push(t)}});__popup.document.querySelector('[data-correction-copy]').click()`);
  check('Both copy buttons retain held results',await evaluate(`__copy.length===2&&__copy.every(t=>t.includes('held last valid TX curve')&&t.includes('Calibration counter 2'))`));
  await evaluate('draw(true,false,60_000)');
  check('Results remain through a long RX inspection',await evaluate(`__popup.document.querySelector('pre').textContent.includes('Calibration counter 2')&&__popup.document.querySelector('p').textContent.includes('not live')`));
  const target=(await command('Target.getTargets')).targetInfos.find(t=>t.title==='Saturn WDSP Correction / Phase Monitor');assert.ok(target);
  const {sessionId}=await command('Target.attachToTarget',{targetId:target.targetId,flatten:true});
  const shot=await command('Page.captureScreenshot',{format:'png'},sessionId);writeFileSync(join(output,'held-correction-phase.png'),Buffer.from(shot.data,'base64'));
  check('No radio connection created',await evaluate('__sockets===0'));
  await evaluate('m.reset(2);draw(false,false,60_010)');
  check('New connection clears historical display',await evaluate(`!m.frame&&__popup.document.querySelector('pre').textContent.includes('Waiting for real WDSP')`));
 },{timeoutMs:60000});
 const result={checks,passed:checks.length,synthetic:true,rfTestPerformed:false};writeFileSync(join(output,'result.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({output,...result},null,2));
}finally{await new Promise(done=>server.close(done))}
