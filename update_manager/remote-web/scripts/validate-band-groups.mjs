#!/usr/bin/env node
// Real Chromium and shipped Ham/SWL controls, persistence, keyboard and phone layout.
// No Bridge connection, audio capture or RF action is performed by this audit.
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv.find(arg => arg.startsWith('--output='))?.slice(9) || mkdtempSync(join(tmpdir(), 'saturn-band-groups-'));
mkdirSync(output, { recursive: true });
const source = readFileSync(resolve(root, '../templates/saturn-remote-next.html'), 'utf8');
const bundle = readFileSync(resolve(root, 'dist/saturn-remote-next.js'));
const html = source.replace('<head>', `<head><script>
if(!sessionStorage.getItem('band-fixture-seeded')){localStorage.clear();sessionStorage.setItem('band-fixture-seeded','1');localStorage.setItem('saturn.remote.freqLock','0')}
window.__sockets=0;window.__media=0;
const NativeWebSocket=window.WebSocket;
window.WebSocket=class extends NativeWebSocket{constructor(...args){super(...args);window.__sockets++}};
if(navigator.mediaDevices?.getUserMedia){const capture=navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);navigator.mediaDevices.getUserMedia=(...args)=>{window.__media++;return capture(...args)}};
</script>`);
const server = createServer((req, res) => {
  if (req.url?.startsWith('/remote-assets/remote-next.js')) { res.setHeader('Content-Type','text/javascript'); res.end(bundle); }
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

const selectGroup = async group => {
  await evaluate(`document.getElementById('band-tab-${group}').click()`);
  await pause(100);
};
const selectedGroup = group => evaluate(`(() => {
  const tabs = [...document.querySelectorAll('#band-group-tabs [role="tab"]')];
  const panel = document.getElementById('band-group-panel');
  return tabs.length === 2 && tabs.every(tab => {
    const selected = tab.dataset.bandTab === '${group}';
    return tab.getAttribute('aria-selected') === String(selected) && tab.tabIndex === (selected ? 0 : -1);
  }) && panel.getAttribute('aria-labelledby') === 'band-tab-${group}';
})()`);
const clickBand = async (group, name) => {
  await evaluate(`document.querySelector('#band-grid [data-band-group="${group}"][data-band="${name}"]').click()`);
  await pause(100);
};
const attenuation = async db => evaluate(`(()=>{const el=document.getElementById('rx-attenuation');el.value='${db}';el.dispatchEvent(new Event('change',{bubbles:true}));})()`);
const ready = async () => {
  for(let i=0;i<100;i++){await pause(50);if(await evaluate(`document.readyState==='complete' && !!document.getElementById('rx-eq-band-10')`).catch(()=>false))return true;}
  return false;
};
try {
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  ({sessionId:session}=await command('Target.attachToTarget',{targetId,flatten:true}));
  await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false},session);
  await command('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/`},session);
  check('Actual radio runtime initializes',await ready());
  check('Ham HF is the initial tab and all twelve buttons remain',await selectedGroup('ham') && await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===12`));
  check('Band tabs share the RX tab appearance and replace the visible dropdown',await evaluate(`(() => {
    const tab = document.getElementById('band-tab-ham');
    const rx = document.getElementById('control-tab-rx');
    return tab.getClientRects().length && document.getElementById('band-tab-swl').getClientRects().length
      && !document.getElementById('band-group').getClientRects().length
      && ['backgroundColor', 'borderColor', 'color', 'minHeight'].every(key => getComputedStyle(tab)[key] === getComputedStyle(rx)[key]);
  })()`));
  await evaluate(`window.__beforeBank=JSON.stringify([state.vfoA,state.vfoB,state.mode,state.rxAttenuationDb,state.txDrive,state.bandMemoryGroup])`);
  await selectGroup('swl');
  check('SWL click moves the selected highlight and tab stop',await selectedGroup('swl'));
  check('Switching bank changes neither tuning nor operating settings',await evaluate(`window.__beforeBank===JSON.stringify([state.vfoA,state.vfoB,state.mode,state.rxAttenuationDb,state.txDrive,state.bandMemoryGroup])`));
  check('All fifteen SWL presets replace the Ham buttons',await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===15 && [...document.querySelectorAll('#band-grid .band-btn:not([hidden])')].every(b=>b.dataset.bandGroup==='swl')`));
  await clickBand('swl','6000000');
  check('49m recalls AM at 6 MHz with the live highlight',await evaluate(`state.vfoA===6000000 && state.mode==='AM' && document.querySelector('#band-grid .band-btn.active').textContent==='49m' && document.querySelector('#band-grid .band-btn.active').getAttribute('aria-checked')==='true'`));
  await evaluate(`document.querySelector('#mode-grid [data-mode="SAM"]').click()`);await attenuation(20);
  await selectGroup('ham');await clickBand('ham','7150000');
  await evaluate(`document.querySelector('#mode-grid [data-mode="LSB"]').click()`);await attenuation(0);
  await selectGroup('swl');await clickBand('swl','6000000');
  check('SWL saved mode and attenuation recall independently',await evaluate(`state.vfoA===6000000 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await selectGroup('ham');await clickBand('ham','7150000');
  check('Ham saved mode and attenuation still recall',await evaluate(`state.vfoA===7150000 && state.mode==='LSB' && state.rxAttenuationDb===0`));
  await evaluate(`setFrequency(3950000);document.querySelector('#mode-grid [data-mode="LSB"]').click()`);await attenuation(10);
  await selectGroup('swl');await clickBand('swl','3950000');
  check('Overlapping 75m SWL does not inherit 80m Ham memory',await evaluate(`state.vfoA===3950000 && state.mode==='AM'`));
  await evaluate(`document.querySelector('#mode-grid [data-mode="SAM"]').click()`);await attenuation(20);
  await selectGroup('ham');await clickBand('ham','3800000');
  check('80m retains its frequency, LSB and attenuation after the overlapping SWL visit',await evaluate(`state.vfoA===3950000 && state.mode==='LSB' && state.rxAttenuationDb===10`));
  await selectGroup('swl');await clickBand('swl','3950000');
  check('Overlapping 75m independently retains SAM and attenuation',await evaluate(`state.vfoA===3950000 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await command('Page.reload',{},session);check('Reload restores the remembered SWL tab',await ready() && await selectedGroup('swl') && await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===15`));
  await evaluate(`document.getElementById('band-tab-swl').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true,cancelable:true}))`);
  check('Left arrow activates and focuses Ham HF',await selectedGroup('ham') && await evaluate(`document.activeElement.id==='band-tab-ham'`));
  await evaluate(`document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true,cancelable:true}))`);
  check('End activates and focuses SWL',await selectedGroup('swl') && await evaluate(`document.activeElement.id==='band-tab-swl'`));
  await clickBand('swl','3950000');
  check('SWL frequency/mode/settings survive a page reload',await evaluate(`state.vfoA===3950000 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await evaluate(`document.querySelector('#band-grid .band-btn.active').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}))`);await pause(100);
  check('Keyboard navigation selects a visible SWL band, never a hidden Ham button',await evaluate(`state.vfoA===25900000 && document.activeElement.dataset.bandGroup==='swl' && document.activeElement.getAttribute('aria-checked')==='true'`));
  await evaluate(`document.getElementById('band-group-tabs').scrollIntoView({block:'center'})`);
  const desktop=await command('Page.captureScreenshot',{format:'png'},session);writeFileSync(join(output,'swl-desktop.png'),Buffer.from(desktop.data,'base64'));
  await command('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true},session);await pause(300);
  await evaluate(`document.getElementById('band-group-tabs').scrollIntoView({block:'center'})`);
  check('SWL remains usable on a phone without overflowing its panel',await evaluate(`(()=>{const s=document.getElementById('band-group-tabs'),g=document.getElementById('band-grid'),r=s.getBoundingClientRect();return r.width>0 && r.right<=innerWidth && r.left>=0 && g.scrollWidth<=g.clientWidth+1 && [...g.querySelectorAll('.band-btn')].some(b=>!b.hidden && b.getClientRects().length)})()`));
  await selectGroup('ham');check('Phone can return to the Ham bank',await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===12`));
  await selectGroup('swl');
  const mobile=await command('Page.captureScreenshot',{format:'png'},session);writeFileSync(join(output,'swl-phone.png'),Buffer.from(mobile.data,'base64'));
  check('Band navigation opened no connection, microphone or TX session',await evaluate(`window.__sockets===0 && window.__media===0 && state.txPhase==='rx' && !state.twoToneEnabled`));
  writeFileSync(join(output,'report.json'),JSON.stringify({passed:true,checks,source:'actual shipped Remote runtime; offline browser, no radio connected'},null,2)+'\n');
  console.log(`Ham/SWL band panel: ${checks.length} checks passed. ${output}`);
} catch(error){
  writeFileSync(join(output,'report.json'),JSON.stringify({passed:false,checks,error:String(error)},null,2)+'\n');throw error;
} finally { clearTimeout(watchdog);chrome.kill();server.close(); }
