// Actual template/IIFE in isolated Chrome. No live bridge or RF commands.
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,dirname,join} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {spawn} from 'node:child_process';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const capture=resolve(process.argv[2]??'');
assert(process.argv[2],'usage: node scripts/validate-signal-monitor.mjs OFFLINE_REPORT.json');
assert(Array.isArray(JSON.parse(readFileSync(capture,'utf8'))));
const output=mkdtempSync(join(tmpdir(),'saturn-signal-monitor-browser-'));
const bundle=readFileSync(join(root,'dist/saturn-remote-next.js'),'utf8');
const source=readFileSync(resolve(root,'../templates/saturn-remote-next.html'),'utf8');
const tag=/<script\s+src="\/remote-assets\/remote-next\.js[^"]*"[\s\S]*?<\/script>/;
assert(tag.test(source));
const page=join(output,'remote.html');
writeFileSync(page,source.replace(tag,`<script>${bundle.replaceAll('</script','<\\/script')}</script>`));
const empty=join(output,'empty.json');writeFileSync(empty,'[]');
const oversized=join(output,'oversized.json');writeFileSync(oversized,' '.repeat(4_000_001));
const chrome=spawn(process.env.CHROMIUM||'google-chrome',['--headless=new','--no-sandbox',
  '--disable-dev-shm-usage','--no-first-run','--disable-background-networking','--disable-extensions',
  '--disable-sync','--mute-audio','--allow-file-access-from-files','--remote-debugging-pipe',
  `--user-data-dir=${join(output,'profile')}`,'about:blank'],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let id=0,buffer='';const pending=new Map();
chrome.stdio[4].on('data',data=>{buffer+=data;let end;while((end=buffer.indexOf('\0'))>=0){
  const raw=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!raw)continue;
  const message=JSON.parse(raw),p=pending.get(message.id);if(!p)continue;pending.delete(message.id);
  message.error?p.reject(new Error(JSON.stringify(message.error))):p.resolve(message.result);
}});
chrome.on('error',error=>{for(const p of pending.values())p.reject(error);});
function command(method,params={},sessionId){return new Promise((resolveCall,rejectCall)=>{
  const sequence=++id;pending.set(sequence,{resolve:resolveCall,reject:rejectCall});
  chrome.stdio[3].write(JSON.stringify({id:sequence,method,params,...(sessionId?{sessionId}:{})})+'\0');
});}
const pause=ms=>new Promise(done=>setTimeout(done,ms));
const watchdog=setTimeout(()=>{chrome.kill();process.exitCode=2;},30000);
try {
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await command('Target.attachToTarget',{targetId,flatten:true});
  const call=(method,params={})=>command(method,params,sessionId);
  const evaluate=async expression=>{const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true});
    if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;};
  await call('Page.enable');await call('Network.enable');
  await call('Network.setBlockedURLs',{urls:['http://*','https://*','ws://*','wss://*']});
  await call('Page.addScriptToEvaluateOnNewDocument',{source:`
    window.__diagSockets=0;window.fetch=async()=>new Response('{}',{status:404});
    window.WebSocket=class{constructor(){window.__diagSockets++;throw new Error('network forbidden in offline Monitor test');}};`});
  await call('Page.navigate',{url:pathToFileURL(page).href});
  let ready=false;
  for(let i=0;i<100;i++){ready=await evaluate(`document.querySelector('[data-ps-signal-monitor]')?.dataset.mounted==='true'`);if(ready)break;await pause(50);}
  assert(ready,'actual template Monitor mounted');
  const load=async file=>{const {root:doc}=await call('DOM.getDocument');const {nodeId}=await call('DOM.querySelector',{
    nodeId:doc.nodeId,selector:'[data-ps-signal-file]'});await call('DOM.setFileInputFiles',{nodeId,files:[file]});};
  await load(capture);
  let summary='';for(let i=0;i<100;i++){summary=await evaluate(`document.querySelector('[data-ps-signal-summary]').textContent`);if(summary.includes('PreWdspPcm'))break;await pause(30);}
  assert(summary.includes('Imported capture — NOT live'));assert(summary.includes('700 Hz:'));assert(summary.includes('overload Unknown'));
  await evaluate(`document.querySelector('[data-ps-signal-popout]').click()`);
  const {targetInfos}=await command('Target.getTargets');
  assert(targetInfos.some(t=>t.title==='Saturn PS Monitor — imported evidence'),'read-only pop-out opened');
  await load(oversized);
  for(let i=0;i<100;i++){summary=await evaluate(`document.querySelector('[data-ps-signal-summary]').textContent`);if(summary.includes('exceeds 4 MB'))break;await pause(30);}
  assert(summary.includes('exceeds 4 MB'));
  await evaluate(`document.querySelector('[data-ps-signal-popout]').click()`);
  const afterOversized=await command('Target.getTargets');
  const newPopup=afterOversized.targetInfos.find(t=>t.title==='Saturn PS Monitor — imported evidence'&&!targetInfos.some(old=>old.targetId===t.targetId));
  assert(newPopup,'fresh error-state pop-out');
  const popupSession=await command('Target.attachToTarget',{targetId:newPopup.targetId,flatten:true});
  const popupText=await command('Runtime.evaluate',{expression:'document.body.textContent',returnByValue:true},popupSession.sessionId);
  assert(popupText.result.value.includes('exceeds 4 MB'));assert(!popupText.result.value.includes('PreWdspPcm'));
  await load(empty);
  for(let i=0;i<100;i++){summary=await evaluate(`document.querySelector('[data-ps-signal-summary]').textContent`);if(summary.includes('empty evidence'))break;await pause(30);}
  assert(summary.includes('empty evidence'));assert.equal(await evaluate('window.__diagSockets'),0);
  const result={actualTemplate:true,actualBundle:true,importedLevels:true,readOnlyPopout:true,emptyEvidenceRejected:true,
    oversizedPanelAndPopoutAgree:true,socketAttempts:0,rfCommandsSent:0,output};
  writeFileSync(join(output,'result.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result));
} finally {clearTimeout(watchdog);await command('Browser.close').catch(()=>{});chrome.kill();}
