// Actual installed UI/proxy, isolated Chrome; receive only. Credentials via stdin.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
const credentials=JSON.parse(await new Promise(resolve=>{let s='';process.stdin.setEncoding('utf8');process.stdin.on('data',b=>s+=b);process.stdin.on('end',()=>resolve(s));}));
const root=dirname(fileURLToPath(import.meta.url)),out=mkdtempSync(join(root,'live-ui-'));
const profile=mkdtempSync(join(tmpdir(),'saturn-ui-auth-'));
const origin='https://192.168.0.139:8443';
const chrome=spawn('google-chrome',['--headless=new','--no-sandbox','--disable-dev-shm-usage','--ignore-certificate-errors',
  '--no-first-run','--mute-audio','--use-angle=swiftshader','--enable-unsafe-swiftshader','--window-size=1440,1200',
  '--remote-debugging-pipe','--disable-background-timer-throttling',`--user-data-dir=${profile}`,'about:blank'],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let next=0,buffer='',session;const pending=new Map(),errors=[];
function cmd(method,params={},sid=session){return new Promise((resolve,reject)=>{const id=++next;pending.set(id,{resolve,reject});chrome.stdio[3].write(JSON.stringify({id,method,params,...(sid?{sessionId:sid}:{})})+'\0');});}
chrome.stdio[4].on('data',d=>{buffer+=d;let end;while((end=buffer.indexOf('\0'))>=0){const raw=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!raw)continue;const m=JSON.parse(raw),p=pending.get(m.id);
  if(p){pending.delete(m.id);m.error?p.reject(Error(JSON.stringify(m.error))):p.resolve(m.result);}
  else if(m.method==='Fetch.requestPaused')void cmd('Fetch.continueRequest',{requestId:m.params.requestId},m.sessionId);
  else if(m.method==='Fetch.authRequired')void cmd('Fetch.continueWithAuth',{requestId:m.params.requestId,authChallengeResponse:m.params.authChallenge.origin===origin?{response:'ProvideCredentials',...credentials}:{response:'CancelAuth'}},m.sessionId);
  else if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.exception?.description||m.params.exceptionDetails.text);
}});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function evaluate(expression){const r=await cmd('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;}
async function until(expression){for(let i=0;i<300;i++){if(await evaluate(expression))return;await sleep(100);}throw Error('UI deadline: '+expression);}
const watchdog=setTimeout(()=>chrome.kill(),90000);
try{
  const target=await cmd('Target.createTarget',{url:'about:blank'},null);session=(await cmd('Target.attachToTarget',{targetId:target.targetId,flatten:true},null)).sessionId;
  await cmd('Page.enable');await cmd('Runtime.enable');await cmd('Fetch.enable',{handleAuthRequests:true});
  await cmd('Emulation.setDeviceMetricsOverride',{width:1440,height:1200,deviceScaleFactor:1,mobile:false});
  await cmd('Page.navigate',{url:origin+'/remote-next?transport=split'});
  await until(`typeof state!=='undefined'&&typeof managedPcm!=='undefined'&&document.getElementById('go-live-btn')`);
  await until(`document.getElementById('event-log').textContent.includes('Saturn Remote loaded')`);
  await evaluate(`document.getElementById('control-tab-ps').click();if(!state.connected)document.getElementById('go-live-btn').click()`);
  await until('state.connected&&state.bridgeReady');
  await until(`document.querySelector('[data-correction-legend]').textContent.includes('Phase correction')`);
  const before=await evaluate(`({iq:state.rxIqFrameVersion||0,audio:rxOpus.pcmFrames+rxOpus.opusFrames})`);await sleep(6000);
  const after=await evaluate(`({iq:state.rxIqFrameVersion||0,audio:rxOpus.pcmFrames+rxOpus.opusFrames})`);
  assert(after.iq-before.iq>80&&after.audio-before.audio>80,'Receive IQ/audio did not advance');
  assert(await evaluate(`!state.txLocalRequestActive&&managedPcm.status().sentFrames===0`),'Verifier unexpectedly requested local TX');
  const txSession=await evaluate(`({role:state.remoteClientRole,phase:managedPcm.status().phase,reason:managedPcm.status().reason})`);
  if(txSession.role==='operator')assert(txSession.phase!=='fault','Operator negotiation fault');
  const compact=await evaluate(`({hostCollapsed:!document.querySelector('[data-ps-health-summary]').closest('details').open,curveCollapsed:!document.querySelector('[data-correction-summary]').closest('details').open,controlsInPs:!!document.querySelector('#ps-control-body #setup-ps-adaptation'),twoToneCount:document.querySelectorAll('#ps-two-tone-btn').length,owner:managedPcm.status().ownerPhase,role:state.remoteClientRole})`);
  assert(compact.hostCollapsed&&compact.curveCollapsed&&compact.controlsInPs&&compact.twoToneCount===1);
  await evaluate(`document.getElementById('ps-control-panel').scrollIntoView({block:'start'})`);await sleep(200);
  const image=await cmd('Page.captureScreenshot',{format:'png'});writeFileSync(join(out,'ps-tab.png'),Buffer.from(image.data,'base64'));
  await evaluate(`document.querySelector('[data-correction-popout]').click()`);await sleep(400);
  const popup=(await cmd('Target.getTargets')).targetInfos.find(t=>t.title==='Saturn WDSP Correction / Phase Monitor');assert(popup);
  const child=(await cmd('Target.attachToTarget',{targetId:popup.targetId,flatten:true},null)).sessionId;
  await cmd('Emulation.setDeviceMetricsOverride',{width:1200,height:1000,deviceScaleFactor:2,mobile:false},child);await sleep(400);
  const plot=await cmd('Runtime.evaluate',{expression:`({graphs:document.querySelectorAll('canvas').length,details:!!document.querySelector('details:not([open])'),legend:document.body.textContent.includes('Magnitude correction')})`,returnByValue:true},child);
  assert.deepEqual(plot.result.value,{graphs:2,details:true,legend:true});
  const graph=await cmd('Page.captureScreenshot',{format:'png'},child);writeFileSync(join(out,'correction-monitor.png'),Buffer.from(graph.data,'base64'));
  await cmd('Target.closeTarget',{targetId:popup.targetId});await evaluate('disconnectBridge()');await until('!state.connected');
  assert.deepEqual(errors,[]);
  const result={passed:true,actualInstalledUi:true,actualTlsProxy:true,receiveOnly:true,pttCommandsSent:0,
    explicitRadioSettingsChanges:0,txSession,operatorTxReadinessTested:txSession.role==='operator',iqFrames:after.iq-before.iq,audioFrames:after.audio-before.audio,compact,
    certificateTrust:'test Chrome exception only; deployed trust unchanged',output:out};
  writeFileSync(join(out,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}catch(error){
  const diagnostic=await evaluate(`({connected:state.connected,ready:state.bridgeReady,target:document.getElementById('bridge-url')?.value,log:document.getElementById('event-log')?.textContent,managed:managedPcm.status()})`).catch(()=>null);
  writeFileSync(join(out,'failure.json'),JSON.stringify({error:String(error),diagnostic,errors},null,2));
  const shot=await cmd('Page.captureScreenshot',{format:'png'}).catch(()=>null);if(shot)writeFileSync(join(out,'failure.png'),Buffer.from(shot.data,'base64'));
  throw error;
}finally{clearTimeout(watchdog);await cmd('Browser.close',{},null).catch(()=>{});chrome.kill();await sleep(200);rmSync(profile,{recursive:true,force:true});}
