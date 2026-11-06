#!/usr/bin/env node
// Real Chromium, shipped Remote runtime and actual Settings launch/relay paths.
// No Bridge connection, audio capture or RF action is performed by this audit.
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv.find(arg => arg.startsWith('--output='))?.slice(9) || mkdtempSync(join(tmpdir(), 'saturn-settings-'));
mkdirSync(output, { recursive: true });
const source = readFileSync(resolve(root, '../templates/saturn-remote-next.html'), 'utf8');
const bundle = readFileSync(resolve(root, 'dist/saturn-remote-next.js'));
const html = source.replace('<head>', `<head><script>
localStorage.clear();localStorage.setItem('saturn.remote.freqLock','0');
window.__opens=0;window.__sockets=0;window.__media=0;
const openWindow=window.open.bind(window);
window.open=(...args)=>{window.__opens++;return window.__settingsPopup=openWindow(...args)};window.__popupOpen=window.open;
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
const child = expression => evaluate(`(()=>{const d=window.__settingsPopup.document;${expression}})()`);
const search = async value => child(`const input=d.querySelector('.sw-search');input.value=${JSON.stringify(value)};input.dispatchEvent(new Event('input',{bubbles:true}));`);
try {
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  ({sessionId:session}=await command('Target.attachToTarget',{targetId,flatten:true}));
  await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false},session);
  await command('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/`},session);
  let ready=false;
  for(let i=0;i<100;i++){await pause(50);if(await evaluate(`document.readyState==='complete' && !!document.getElementById('rx-eq-band-10')`).catch(()=>false)){ready=true;break;}}
  check('Actual Remote runtime mounted EQ and CFC controls',ready);
  await evaluate(`document.getElementById('header-setup-btn').focus();document.getElementById('header-setup-btn').click()`);
  await pause(300);
  check('Header opens a separate Settings window',await evaluate(`!!window.__settingsPopup && !window.__settingsPopup.closed && !!window.__settingsPopup.document.querySelector('.sw-root')`));
  const childTarget = (await command('Target.getTargets')).targetInfos.find(target=>target.title==='Saturn Remote — Settings');
  assert.ok(childTarget);
  const {sessionId:childSession}=await command('Target.attachToTarget',{targetId:childTarget.targetId,flatten:true});
  await command('Emulation.setDeviceMetricsOverride',{width:1240,height:850,deviceScaleFactor:1,mobile:false},childSession);
  check('Ten categories, search and a live band context',await child(`return d.querySelectorAll('[data-page]').length===10 && !!d.querySelector('.sw-search') && d.querySelector('.sw-context').textContent.includes('MHz')`));
  check('Opening settings creates no socket or audio capture',await evaluate(`window.__sockets===0 && window.__media===0`));
  check('Popup has no opener authority',await child(`return d.defaultView.opener===null`));
  await child(`d.querySelector('[data-page="transmit"]').click()`);
  check('All ten TX EQ and ten CFC bands are editable here',await child(`return d.querySelectorAll('[data-setting^="transmit.eqBand"] input[type="range"]').length===10 && d.querySelectorAll('[data-setting^="transmit.cfcBand"] input[type="range"]').length===10`));
  await evaluate(`(()=>{window.__relay=[]; const input=document.getElementById('tx-mic-gain');input.disabled=false;for(const type of ['input','change'])input.addEventListener(type,()=>window.__relay.push(type));})()`);
  await child(`const input=d.querySelector('[data-setting="transmit.micGain"] .sw-exact');input.value='3';input.dispatchEvent(new Event('change',{bubbles:true}));`);
  check('Exact value reaches original input/change and existing persistence',await evaluate(`document.getElementById('tx-mic-gain').value==='3' && JSON.stringify(window.__relay)===JSON.stringify(['input','change']) && JSON.parse(localStorage.getItem('saturn.remote.radioPrefs')).txMicGainDb===3`));
  check('The band memory stores the changed microphone gain',await evaluate(`Object.values(JSON.parse(localStorage.getItem('saturn.remote.bandMemory'))).some(entry=>entry.radioPrefs.txMicGainDb===3)`));
  await evaluate(`(()=>{const input=document.getElementById('tx-mic-gain');input.value='4';input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await pause(350);
  check('Changes on the main radio mirror back into the popup',await child(`return d.querySelector('[data-setting="transmit.micGain"] .sw-input').value==='4'`));
  await child(`const input=d.querySelector('[data-setting="transmit.micGain"] .sw-exact');input.value='9999';input.dispatchEvent(new Event('change',{bubbles:true}));`);
  check('Out-of-range exact edit never reaches the original control',await evaluate(`document.getElementById('tx-mic-gain').value==='4'`));
  await evaluate(`document.getElementById('tx-mic-gain').disabled=true;window.__relay=[];`);
  await pause(350);
  check('Host-disabled state is visible and cannot be bypassed by stale events',await child(`const input=d.querySelector('[data-setting="transmit.micGain"] .sw-input');const disabled=input.disabled;input.value='5';input.dispatchEvent(new Event('input',{bubbles:true}));return disabled`) && await evaluate(`window.__relay.length===0`));
  await evaluate(`document.getElementById('tx-mic-gain').disabled=false;`);await pause(350);
  await child(`d.querySelector('[data-setting="transmit.micGain"] .sw-exact').focus();window.__retainedControl=d.querySelector('[data-setting="transmit.micGain"] .sw-exact');window.__retainedControl.value='7';`);
  check('Uncommitted exact edit really has popup focus',await child(`return d.activeElement===window.__retainedControl`));
  await evaluate(`document.querySelector('#band-grid [data-band="7150000"]').click()`);await pause(350);
  check('Band change refreshes even a focused uncommitted edit',await child(`return d.querySelector('.sw-context').textContent.includes('40m') && !window.__retainedControl.isConnected`));
  await evaluate(`window.__relay=[];`);
  await evaluate(`window.__retainedControl.dispatchEvent(new Event('change',{bubbles:true}))`);
  check('A detached old-band edit cannot modify the new band',await evaluate(`window.__relay.length===0`));

  await child(`d.querySelector('[data-page="receive"]').click()`);
  check('Receiver includes all ten EQ bands and advanced NR2',await child(`return d.querySelectorAll('[data-setting^="receive.rxEqBand"] input[type="range"]').length===10 && !!d.querySelector('[data-setting="receive.nr2GainMethod"] select')`));
  await child(`const input=d.querySelector('[data-setting="receive.nr2PostFilter"] input');input.checked=false;input.dispatchEvent(new Event('input',{bubbles:true}));input.dispatchEvent(new Event('change',{bubbles:true}));`);
  check('Checkbox edits use the actual RX preference handler',await evaluate(`document.getElementById('setup-dsp-nr2-post-filter').checked===false && JSON.parse(localStorage.getItem('saturn.remote.radioPrefs')).rxNr2PostFilterEnabled===false`));
  await search('SATP');
  check('Search finds existing names across categories',await child(`return d.querySelector('.sw-page-heading').textContent.includes('Results') && d.querySelectorAll('[data-setting]').length>=3`));
  await child(`d.querySelector('[data-page="ps"]').click()`);
  check('PureSignal controls and honest host reports share a page',await child(`return !!d.querySelector('[data-setting="transmit.psAdaptation"] select') && !!d.querySelector('[data-setting="transmit.psFeedback"] output') && !!d.querySelector('[data-setting="transmit.psDuration"] input')`));
  for(const page of ['radio','receive','transmit','ps','display','audio','meter','network','interface','about']){
    await child(`d.querySelector('[data-page="${page}"]').click()`);
    check(`${page} page fits horizontally`,await child(`return d.documentElement.scrollWidth<=d.defaultView.innerWidth && Array.from(d.querySelectorAll('.sw-card')).every(card=>card.scrollWidth<=card.clientWidth+1)`));
  }
  await child(`d.querySelector('[data-page="transmit"]').click()`);
  const screenshot=await command('Page.captureScreenshot',{format:'png'},childSession);writeFileSync(join(output,'transmitter-dark.png'),Buffer.from(screenshot.data,'base64'));
  await evaluate(`document.documentElement.dataset.theme='light'`);await pause(350);
  check('Theme follows the main radio',await child(`return d.querySelector('.sw-root').dataset.theme==='light'`));
  await child(`d.querySelector('[data-page="receive"]').click()`);
  const light=await command('Page.captureScreenshot',{format:'png'},childSession);writeFileSync(join(output,'receiver-light.png'),Buffer.from(light.data,'base64'));
  await evaluate(`document.getElementById('header-setup-btn').focus();document.getElementById('header-setup-btn').click()`);
  check('Repeated launch reuses the same window',await evaluate(`window.__opens===1`));
  await evaluate(`window.__settingsPopup.close()`);await pause(300);
  await evaluate(`document.getElementById('header-setup-btn').focus();document.getElementById('header-setup-btn').click()`);await pause(200);
  check('Closing and reopening restores the selected category',await child(`return d.querySelector('[data-page="receive"]').getAttribute('aria-current')==='page'`));
  await evaluate(`window.__settingsPopup.close();window.open=()=>null;`);await pause(100);
  await evaluate(`document.getElementById('header-setup-btn').focus();document.getElementById('header-setup-btn').click()`);
  check('Blocked popup falls back to an interactive dialog',await evaluate(`document.querySelector('.sw-overlay [role="dialog"]')?.getAttribute('aria-modal')==='true' && !!document.querySelector('.sw-overlay input')`));
  await evaluate(`window.__radioKeyDown=0;window.__radioKeyUp=0;document.addEventListener('keydown',()=>window.__radioKeyDown++);document.addEventListener('keyup',()=>window.__radioKeyUp++);const root=document.querySelector('.sw-root');root.dispatchEvent(new KeyboardEvent('keydown',{key:' ',bubbles:true}));root.dispatchEvent(new KeyboardEvent('keyup',{key:' ',bubbles:true}));`);
  check('Embedded Settings blocks radio shortcuts but preserves PTT key-up',await evaluate(`window.__radioKeyDown===0 && window.__radioKeyUp===1`));
  await evaluate(`document.querySelector('.sw-root').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))`);
  check('Escape closes the fallback and restores launch focus',await evaluate(`!document.querySelector('.sw-overlay') && document.activeElement.id==='header-setup-btn'`));
  await command('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true},session);
  await evaluate(`document.getElementById('header-setup-btn').focus();document.getElementById('header-setup-btn').click()`);
  check('Phone gets a full-screen Settings dialog without horizontal overflow',await evaluate(`!!document.querySelector('.sw-overlay') && document.querySelector('.sw-root').getBoundingClientRect().width<=390 && document.querySelector('.sw-root').scrollWidth<=390`));
  const mobile=await command('Page.captureScreenshot',{format:'png'},session);writeFileSync(join(output,'settings-phone.png'),Buffer.from(mobile.data,'base64'));
  await evaluate(`document.querySelector('.sw-footer-tools button').click()`);
  check('Phone layout link closes the fallback and reveals the real layout editor',await evaluate(`!document.querySelector('.sw-overlay') && !document.getElementById('setup-menu').hidden && !!document.querySelector('.settings-layout-editor')`));
  await evaluate(`document.getElementById('setup-close-btn').click();document.getElementById('header-setup-btn').focus();document.getElementById('header-setup-btn').click()`);
  await evaluate(`document.querySelector('.sw-close').click()`);
  check('Settings closes without starting audio or connecting the radio',await evaluate(`!document.querySelector('.sw-root') && window.__sockets===0 && window.__media===0`));
  await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false},session);
  await evaluate(`window.open=window.__popupOpen;document.getElementById('header-setup-btn').focus();document.getElementById('header-setup-btn').click()`);
  check('Desktop can pop out again after phone/fallback use',await evaluate(`!!window.__settingsPopup && !window.__settingsPopup.closed`));
  await command('Page.navigate',{url:'about:blank'},session);await pause(200);
  check('Leaving the radio page closes its Settings window',!(await command('Target.getTargets')).targetInfos.some(target=>target.title==='Saturn Remote — Settings'));
  writeFileSync(join(output,'report.json'),JSON.stringify({passed:true,checks,source:'actual shipped Remote runtime; no radio connected'},null,2)+'\n');
  console.log(`Settings workbench: ${checks.length} checks passed. ${output}`);
} catch(error){
  writeFileSync(join(output,'report.json'),JSON.stringify({passed:false,checks,error:String(error)},null,2)+'\n');throw error;
} finally { clearTimeout(watchdog);chrome.kill();server.close(); }
