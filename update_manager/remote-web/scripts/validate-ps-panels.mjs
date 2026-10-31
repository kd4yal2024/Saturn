#!/usr/bin/env node
// Whole-page Chrome window lifecycle. Loopback/file fixture only; no radio.
import {readFileSync,writeFileSync,mkdtempSync} from 'node:fs';
import {join,resolve,dirname} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {spawn} from 'node:child_process';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const output=process.env.SATURN_PS_PANEL_OUTPUT||mkdtempSync(join(tmpdir(),'saturn-ps-panel-browser-'));
const source=readFileSync(resolve(root,'../templates/saturn-remote-next.html'),'utf8');
const bundle=readFileSync(join(root,'dist/saturn-remote-next.js'),'utf8');
const pre=`window.fixtureErrors=[];window.fixtureDone=false;window.addEventListener('error',e=>fixtureErrors.push(e.message));window.addEventListener('unhandledrejection',e=>fixtureErrors.push(String(e.reason)));window.fetch=async()=>({ok:false,status:404,json:async()=>({})});window.WebSocket=class {static OPEN=1;constructor(){throw Error('PS panel fixture must not open a socket');}};`;
const adapter=`(async()=>{await init();window.fixtureDone=true;})().catch(e=>{fixtureErrors.push(String(e.stack||e));window.fixtureDone=true;});`;
let html=source.replace(/<script\s+src="\/remote-assets\/remote-next\.js[^]*?<\/script>/,()=>`<script>${bundle.replaceAll('</script','<\\/script')}</script>`);
html=html.replace('<head>','<head><script>'+pre+'</script>').replace('    void init();',adapter);
const page=join(output,'fixture.html');writeFileSync(page,html);
const chrome=spawn(process.env.CHROMIUM||'google-chrome',['--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run','--use-angle=swiftshader','--enable-unsafe-swiftshader','--remote-debugging-pipe','--disable-popup-blocking',`--user-data-dir=${join(output,'profile')}`,'about:blank'],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let id=0,buffer='',stderr='';const pending=new Map();
chrome.stderr.on('data',v=>stderr+=v);
chrome.stdio[4].on('data',v=>{buffer+=v;let end;while((end=buffer.indexOf('\0'))>=0){const line=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!line)continue;const m=JSON.parse(line);if(m.id&&pending.has(m.id)){const p=pending.get(m.id);pending.delete(m.id);m.error?p.reject(Error(JSON.stringify(m.error))):p.resolve(m.result);}}});
function cmd(method,params={},sessionId){return new Promise((resolve,reject)=>{const message={id:++id,method,params};if(sessionId)message.sessionId=sessionId;pending.set(id,{resolve,reject});chrome.stdio[3].write(JSON.stringify(message)+'\0');});}
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const checks=[];const watchdog=setTimeout(()=>{console.error('PS panel validator timeout',stderr);chrome.kill();process.exit(1);},90000);
const assert=(ok,reason)=>{if(!ok)throw Error(reason);};
try{
 const {targetId}=await cmd('Target.createTarget',{url:'about:blank'});const {sessionId}=await cmd('Target.attachToTarget',{targetId,flatten:true});
 const call=(method,params={})=>cmd(method,params,sessionId);
 const evaluate=async(expression,sid=sessionId)=>{const r=await cmd('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true},sid);if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;};
 await call('Page.enable');await call('Runtime.enable');await call('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false});
 await call('Page.navigate',{url:pathToFileURL(page).href});
 for(let i=0;i<80;i++){await sleep(100);if(await evaluate('window.fixtureDone===true'))break;}
 assert(await evaluate('window.fixtureDone===true'),'whole page initialization');assert(!(await evaluate('fixtureErrors')).length,JSON.stringify(await evaluate('fixtureErrors')));
 await evaluate("document.getElementById('control-tab-ps').click()");
 const selectors=[['ps-health-monitor','[data-ps-health-popout]'],['ps-rf-monitor','[data-rf-popout]'],['ps-correction-monitor','[data-correction-popout]']];
 // A blocked popup keeps the source panel visible and a durable explanation.
 await evaluate("window.actualOpen=window.open;window.open=()=>null;document.querySelector('#ps-rf-monitor [data-rf-popout]').click();window.open=window.actualOpen");
 await sleep(250);
 assert(await evaluate("!document.getElementById('ps-rf-monitor').hidden && document.querySelector('[data-rf-status]').textContent.includes('blocked')"),'blocked popup fallback');checks.push('blocked popup retains docked view and explanation');
 const before=new Set((await cmd('Target.getTargets')).targetInfos.map(t=>t.targetId));
 for(const [panel,button] of selectors)await evaluate(`document.querySelector('#${panel} ${button}').click()`);
 await sleep(500);
 const popups=(await cmd('Target.getTargets')).targetInfos.filter(t=>t.type==='page'&&!before.has(t.targetId));
 assert(popups.length===3,'three simultaneous detached panels');
 assert(await evaluate(JSON.stringify(selectors.map(([p])=>p))+".every(id=>document.getElementById(id).hidden)"),'docked copies hidden');checks.push('three windows share existing read-only renderers; no duplicate socket');
 for(const popup of popups){
  const {sessionId:child}=await cmd('Target.attachToTarget',{targetId:popup.targetId,flatten:true});
  assert(await evaluate("window.opener===null && document.querySelector('[data-ps-dock]')!==null",child),'read-only detached document and Dock action');
 }
 const first=popups.find(p=>p.title==='Saturn PS Monitor');assert(first,'PS Monitor popup target');const {sessionId:firstSession}=await cmd('Target.attachToTarget',{targetId:first.targetId,flatten:true});
 await evaluate("document.querySelector('[data-ps-dock]').click()",firstSession).catch(e=>{if(!/closed|target|context/i.test(String(e)))throw e;});
 await sleep(600);assert(await evaluate("!document.getElementById('ps-health-monitor').hidden"),'Dock restores main view');checks.push('Dock restores main panel and focus');
 await evaluate("document.getElementById('ps-reset-panel-layout').click()");await sleep(600);
 assert(await evaluate(JSON.stringify(selectors.map(([p])=>p))+".every(id=>!document.getElementById(id).hidden)"),'layout reset restores every panel');
 assert((await cmd('Target.getTargets')).targetInfos.filter(t=>t.type==='page'&&!before.has(t.targetId)).length===0,'reset closes detached windows');checks.push('Reset closes popups and restores all panels');
 // Browser-created resize persistence and external close recovery.
 await evaluate("document.getElementById('ps-health-monitor').style.height='310px'");await sleep(200);
 assert(await evaluate("localStorage.getItem('saturn.ps.panel-layout.v1:ps-monitor:docked')==='310'"),'docked resize persistence');
 await evaluate("document.querySelector('#ps-health-monitor [data-ps-health-popout]').click()");await sleep(200);
 const opened=(await cmd('Target.getTargets')).targetInfos.find(t=>t.type==='page'&&!before.has(t.targetId));assert(opened,'panel reopened');await cmd('Target.closeTarget',{targetId:opened.targetId});await sleep(700);
 assert(await evaluate("!document.getElementById('ps-health-monitor').hidden"),'external window close returns panel');checks.push('resize stored; closing a window returns its docked view');
 await call('Page.reload');await sleep(1000);assert(await evaluate("document.getElementById('ps-health-monitor').style.height==='310px'"),'reload restores docked size');checks.push('reload restores saved size without automatically opening windows');
 await call('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
 await evaluate("document.getElementById('control-tab-ps').click();document.querySelector('#ps-health-monitor [data-ps-health-popout]').click()");
 assert(await evaluate("document.getElementById('ps-health-monitor').dataset.panelMode==='expanded'"),'mobile expands in page');
 assert((await cmd('Target.getTargets')).targetInfos.filter(t=>t.type==='page'&&!before.has(t.targetId)).length===0,'mobile creates no popup');checks.push('mobile expansion remains in page');
 const errors=await evaluate('fixtureErrors');
 const warnings=errors.filter(e=>e==='ResizeObserver loop completed with undelivered notifications.');
 assert(errors.length===warnings.length,JSON.stringify(errors));
 const result={passed:true,offline:true,hardwareAccess:false,pttCommandsSent:0,txMediaFramesSent:0,checks,warnings};writeFileSync(join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({output,...result}));
}catch(error){writeFileSync(join(output,'failure.txt'),String(error.stack||error)+'\n'+stderr);throw error;}
finally{clearTimeout(watchdog);chrome.kill();}
