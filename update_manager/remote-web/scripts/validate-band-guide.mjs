#!/usr/bin/env node
// Real production page + bundled EiBi data, offline; no radio, WebSocket or mic.
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv.find(arg => arg.startsWith('--output='))?.slice(9) || mkdtempSync(join(tmpdir(), 'saturn-band-guide-'));
mkdirSync(output, { recursive: true });
const source = readFileSync(process.argv.find(arg => arg.startsWith('--template='))?.slice(11) || resolve(root, '../templates/saturn-remote-next.html'), 'utf8');
const bundle = readFileSync(process.argv.find(arg => arg.startsWith('--bundle='))?.slice(9) || resolve(root, 'dist/saturn-remote-next.js'));
assert.equal(source.split('    void init();').length,2,'one production initialization marker');
const html = source.replace('<head>', `<head><script>
if(!sessionStorage.getItem('band-fixture-seeded')){localStorage.clear();sessionStorage.setItem('band-fixture-seeded','1');localStorage.setItem('saturn.remote.freqLock','0')}
const NativeDate = Date;
window.Date = class extends NativeDate {
  constructor(...args) { super(...(args.length ? args : ['2026-11-17T12:00:00Z'])); }
  static now() { return NativeDate.parse('2026-11-17T12:00:00Z'); }
};
window.__sockets=0;window.__media=0;
const NativeWebSocket=window.WebSocket;
window.WebSocket=class extends NativeWebSocket{constructor(...args){super(...args);window.__sockets++}};
if(navigator.mediaDevices?.getUserMedia){const capture=navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);navigator.mediaDevices.getUserMedia=(...args)=>{window.__media++;return capture(...args)}};
</script>`).replace('    void init();',`    void init().then(() => {
  window.__bandGuide = { state, repaint: renderBandEdges, guide: panadapterBandGuide,
    setView: (hz, span = 100000) => {
      setFrequency(hz, false, { send: false, rememberBand: false });
      state.sampleRate = 384000; state.displayZoom = 384000 / span;
      updateAxes(); renderBandEdges();
    } };
  window.__bandPageReady = true;
});`);
let corruptCsv = false; let csvRequests = 0;
const catalogue = readFileSync(resolve(root, '../templates/saturn-swl.csv'));
const server = createServer(async (req, res) => {
  if (req.url === '/remote-assets/swl.csv') {
    csvRequests++; res.setHeader('Content-Type','text/csv; charset=utf-8');
    if (corruptCsv === 'xss') res.end('6050;0000-2400;;USA;<img src=x onerror=window.__swlXss=1>;E;NAm;;1;;');
    else if (corruptCsv) res.end('<html>not a schedule</html>'); else res.end(catalogue);
  }
  else if (req.url?.startsWith('/remote-assets/remote-next.js')) { res.setHeader('Content-Type','text/javascript'); res.end(bundle); }
  else if (req.url === '/') { res.setHeader('Content-Type','text/html'); res.end(html); }
  else { res.writeHead(404); res.end('offline browser fixture'); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const chrome = spawn(process.env.CHROMIUM || 'google-chrome', [
  '--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run',
  '--disable-background-networking','--disable-extensions','--disable-sync','--disable-popup-blocking',
  '--mute-audio','--window-size=1600,1100','--remote-debugging-pipe',`--user-data-dir=${join(output,'profile')}`,'about:blank',
], { stdio: ['ignore','ignore','pipe','pipe','pipe'] });
let next = 0, buffer = '', stderr = '';
const pending = new Map(), checks = [];
chrome.stderr.on('data', data => { stderr += data; });
chrome.stdio[4].on('data', data => {
  buffer += data;
  for (let end; (end = buffer.indexOf('\0')) >= 0;) {
    const raw = buffer.slice(0,end); buffer = buffer.slice(end+1); if (!raw) continue;
    const message = JSON.parse(raw), task = pending.get(message.id);
    if (task) { pending.delete(message.id); message.error ? task.reject(Error(JSON.stringify(message.error))) : task.resolve(message.result); }
  }
});
function command(method, params = {}, sessionId) {
  return new Promise((resolve,reject) => {
    const id = ++next;pending.set(id,{resolve,reject});
    chrome.stdio[3].write(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})})+'\0');
  });
}
const pause = ms => new Promise(done => setTimeout(done,ms));
const watchdog = setTimeout(() => { console.error(stderr.slice(-1000)); chrome.kill(); process.exit(2); }, 120000);
let session;
const evaluate = async expression => {
  const result = await command('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true},session);
  if(result.exceptionDetails)throw Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
  return result.result.value;
};
const check = (name,value) => { assert.ok(value,name); checks.push(name); };


const until = async (expression, name) => {
  for (let i=0; i<200; i++) { if (await evaluate(expression).catch(()=>false)) return; await pause(25); }
  throw Error(`Timed out: ${name}`);
};
const setView = async (hz, span) => { await evaluate(`window.__bandGuide.setView(${hz}, ${span})`); await pause(80); };
const screenshot = async name => {
  await evaluate(`document.getElementById('spectrum-shell').scrollIntoView({block:'center'})`);
  await pause(150);
  const image = await command('Page.captureScreenshot',{format:'png'},session);
  writeFileSync(join(output,name),Buffer.from(image.data,'base64'));
};
try {
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  ({sessionId:session}=await command('Target.attachToTarget',{targetId,flatten:true}));
  await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false},session);
  const url=`http://127.0.0.1:${server.address().port}/`;
  await command('Page.navigate',{url},session);
  await until('window.__bandPageReady===true','production initialization');
  await until(`document.getElementById('swl-schedule-status').textContent.includes('entries')`,'real bundled CSV');
  check('Current EiBi snapshot loads exactly once',csvRequests===1);
  await setView(7125000,100000);
  check('Real panadapter shows the 40m data / phone boundary',await evaluate(`(()=>{
    const nodes=[...document.querySelectorAll('#spectrum-shell .band-guide-segment')];
    return nodes.some(n=>n.textContent.includes('Data / CW')) && nodes.some(n=>n.textContent.includes('Phone / CW'))
      && nodes.some(n=>n.classList.contains('band-use-phone')&&Math.abs(parseFloat(n.style.left)-50)<0.001);
  })()`));
  await screenshot('ham-40m-desktop.png');
  await setView(7250000,100000);
  check('Overlapping Ham 40m and SWL 41m occupy separate tracks',await evaluate(`(()=>{
    const nodes=[...document.querySelectorAll('#spectrum-shell .band-guide-segment')];
    const ham=nodes.find(n=>n.textContent.includes('40m'));
    const swl=nodes.find(n=>n.textContent.includes('SWL 41m'));
    return ham&&swl&&ham.parentElement.style.top!==swl.parentElement.style.top;
  })()`));
  await setView(10000000,20000);
  await until(`Array.from(document.querySelectorAll('.swl-station-label')).some(n=>n.title.includes('WWV'))`,'WWV schedule');
  check('Station labels show real scheduled entries and UTC details',await evaluate(`Array.from(document.querySelectorAll('.swl-station-label')).some(n=>n.title.includes('WWV')&&n.title.includes('0000-2400 UTC')&&n.title.includes('not a signal detection'))`));
  check('Annotation node count stays bounded',await evaluate(`document.querySelectorAll('.swl-station-tick').length<=96&&document.querySelectorAll('.swl-station-label').length<=24`));
  const mutations=await evaluate(`new Promise(resolve=>{
    let count=0;const layer=document.querySelector('#spectrum-shell .band-guide-layer');
    const observer=new MutationObserver(records=>{count+=records.length});observer.observe(layer,{subtree:true,childList:true,attributes:true});
    for(let i=0;i<1000;i++)window.__bandGuide.repaint();
    requestAnimationFrame(()=>requestAnimationFrame(()=>{observer.disconnect();resolve(count)}));
  })`);
  check('1000 unchanged UI updates cause zero annotation DOM mutations',mutations===0);
  await evaluate(`for(let i=0;i<200;i++)window.__bandGuide.guide.update({low:i%2?7100000:14000000,high:i%2?7200000:14350000,width:1000,segments:true,stations:false,scheduledOnly:true});window.__bandGuide.repaint()`);
  await pause(80);
  check('Newest view wins without retaining obsolete annotation work',await evaluate(`Array.from(document.querySelectorAll('#spectrum-shell .band-guide-segment')).every(n=>!n.textContent.includes('20m')&&!n.textContent.includes('40m'))`));
  await setView(6000000,100000);
  await screenshot('swl-49m-desktop.png');
  await evaluate(`document.documentElement.dataset.theme='light'`);
  await screenshot('swl-49m-light.png');
  await evaluate(`document.documentElement.dataset.theme='dark'`);
  await command('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true},session);
  await pause(200);
  await setView(6000000,100000);
  check('Phone labels remain inside the panadapter',await evaluate(`(()=>{
    const r=document.getElementById('spectrum-shell').getBoundingClientRect();
    return [...document.querySelectorAll('#spectrum-shell .swl-station-label')].every(n=>{const b=n.getBoundingClientRect();return b.left>=r.left-1&&b.right<=r.right+1});
  })()`));
  await screenshot('swl-49m-phone.png');
  await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false},session);
  await evaluate(`document.getElementById('view-3d').click()`);await pause(250);
  check('3D view retains the frequency annotation surface',await evaluate(`document.documentElement.dataset.terrain==='true'&&document.querySelector('#spectrum-shell .band-guide-layer').getBoundingClientRect().width>0`));
  await screenshot('swl-49m-3d.png');
  await evaluate(`(()=>{const n=document.getElementById('display-show-swl-stations');n.checked=false;n.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await pause(100);
  check('Station display toggle applies without disabling band guidance',await evaluate(`document.querySelector('.swl-station-layer').hidden&&document.querySelectorAll('.band-guide-segment').length>0`));
  await command('Page.reload',{},session);await pause(250);
  await until('window.__bandPageReady===true','reload after saving preferences');
  check('Display options survive a page reload',await evaluate(`window.__bandGuide.state.showSwlStations===false&&document.getElementById('display-show-swl-stations').checked===false`));
  corruptCsv='xss';
  await setView(6050000,20000);
  await evaluate(`(()=>{const n=document.getElementById('display-show-swl-stations');n.checked=true;n.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await until(`document.querySelector('.swl-station-label')?.textContent.includes('<img')`,'literal CSV station name');
  check('CSV station names render as text, never executable HTML',await evaluate(`!document.querySelector('.swl-station-layer img')&&window.__swlXss===undefined`));
  corruptCsv=true;
  await command('Page.reload',{},session);await pause(250);
  await until('window.__bandPageReady===true','reload with malformed CSV');
  await until(`document.getElementById('swl-schedule-status').textContent.includes('No usable')`,'malformed CSV handled');
  check('CSV failure leaves band guidance and the radio UI available',await evaluate(`document.querySelectorAll('.band-guide-segment').length>0&&window.__bandPageReady===true`));
  check('No Bridge/WebSocket or microphone was opened',await evaluate('window.__sockets===0&&window.__media===0'));
  const report={status:'passed',checks,csvRequests,domMutationsFor1000UnchangedUpdates:mutations,physicalRfTested:false,output};
  writeFileSync(join(output,'browser-validation.json'),JSON.stringify(report,null,2)+'\n');
  console.log(JSON.stringify(report,null,2));
} catch (error) {
  const page=await evaluate(`({status:document.getElementById('swl-schedule-status')?.textContent,labels:[...document.querySelectorAll('.swl-station-label')].map(n=>({text:n.textContent,title:n.title})),view:window.__bandGuide?.guide?.view})`).catch(()=>null);
  writeFileSync(join(output,'browser-failure.json'),JSON.stringify({error:String(error),checks,page,stderr},null,2));
  throw error;
} finally {
  clearTimeout(watchdog); chrome.kill(); server.close();
}
