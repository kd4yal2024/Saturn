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
const source = readFileSync(process.argv.find(arg => arg.startsWith('--template='))?.slice(11) || resolve(root, '../templates/saturn-remote-next.html'), 'utf8');
const bundle = readFileSync(process.argv.find(arg => arg.startsWith('--bundle='))?.slice(9) || resolve(root, 'dist/saturn-remote-next.js'));
assert.equal(source.split('    void init();').length,2,'one production initialization marker');
const html = source.replace('<head>', `<head><script>
if(!sessionStorage.getItem('band-fixture-seeded')){localStorage.clear();sessionStorage.setItem('band-fixture-seeded','1');localStorage.setItem('saturn.remote.freqLock','0')}
window.__sockets=0;window.__media=0;
const NativeWebSocket=window.WebSocket;
window.WebSocket=class extends NativeWebSocket{constructor(...args){super(...args);window.__sockets++}};
if(navigator.mediaDevices?.getUserMedia){const capture=navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);navigator.mediaDevices.getUserMedia=(...args)=>{window.__media++;return capture(...args)}};
</script>`).replace('    void init();','    void init().then(() => { window.__bandPageReady = true; });');
let connectedSettings = false;
const settingsWrites = [];
const server = createServer(async (req, res) => {
  if (req.url?.startsWith('/remote-assets/remote-next.js')) { res.setHeader('Content-Type','text/javascript'); res.end(bundle); }
  else if (req.url === '/') { res.setHeader('Content-Type','text/html'); res.end(html); }
  else if (connectedSettings && req.url === '/remote_settings') {
    if (req.method === 'POST') { let raw='';for await (const chunk of req) raw+=chunk;settingsWrites.push(JSON.parse(raw)); }
    const settings=settingsWrites.at(-1);
    if(!settings){res.writeHead(404);res.end('{}');return;}
    res.setHeader('Content-Type','application/json');res.end(JSON.stringify({status:'ok',settings}));
  }
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
  if(!await selectedGroup(group))await selectGroup(group);
  await evaluate(`document.querySelector('#band-grid [data-band-group="${group}"][data-band="${name}"]').click()`);
  await pause(100);
};
const attenuation = async db => evaluate(`(()=>{const el=document.getElementById('rx-attenuation');el.value='${db}';el.dispatchEvent(new Event('change',{bubbles:true}));})()`);
const ready = async () => {
  for(let i=0;i<100;i++){await pause(50);if(await evaluate(`window.__bandPageReady===true`).catch(()=>false))return true;}
  return false;
};
try {
  const {targetId}=await command('Target.createTarget',{url:'about:blank'});
  ({sessionId:session}=await command('Target.attachToTarget',{targetId,flatten:true}));
  await command('Emulation.setDeviceMetricsOverride',{width:1440,height:1000,deviceScaleFactor:1,mobile:false},session);
  await command('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/`},session);
  check('Actual radio runtime initializes',await ready());
  check('Ham HF contains only the eleven amateur-band buttons',await selectedGroup('ham') && await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===11 && !document.querySelector('#band-grid [data-band-group="ham"][data-band="27385000"]') && !document.querySelector('#band-grid [data-band-group="ham"][data-band="99500000"]')`));
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
  check('SWL offers fifteen broadcast presets plus separate 27 MHz and FM memories',await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===17 && [...document.querySelectorAll('#band-grid .band-btn:not([hidden])')].every(b=>b.dataset.bandGroup==='swl')`));
  await clickBand('swl','6000000');
  check('49m recalls AM at 6 MHz with the live highlight',await evaluate(`state.vfoA===6000000 && state.mode==='AM' && document.querySelector('#band-grid .band-btn.active').textContent==='49m' && document.querySelector('#band-grid .band-btn.active').getAttribute('aria-checked')==='true'`));
  await evaluate(`document.querySelector('#mode-grid [data-mode="SAM"]').click()`);await attenuation(20);
  await evaluate(`setFrequency(6075000)`);
  await selectGroup('ham');await clickBand('ham','7150000');
  await evaluate(`document.querySelector('#mode-grid [data-mode="LSB"]').click()`);await attenuation(0);
  await evaluate(`setFrequency(7198300)`);
  await clickBand('ham','7150000');
  check('Clicking the current HF band retains its latest tune before autosave',await evaluate(`state.vfoA===7198300 && state.mode==='LSB' && state.rxAttenuationDb===0`));
  await selectGroup('swl');await clickBand('swl','6000000');
  check('SWL last-used frequency, mode and attenuation recall independently',await evaluate(`state.vfoA===6075000 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await evaluate(`setFrequency(6085500)`);
  await clickBand('swl','6000000');
  check('Clicking the current SWL band retains its latest tune and mode',await evaluate(`state.vfoA===6085500 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await selectGroup('ham');await clickBand('ham','7150000');
  check('HF last-used frequency, mode and attenuation still recall',await evaluate(`state.vfoA===7198300 && state.mode==='LSB' && state.rxAttenuationDb===0`));
  await clickBand('swl','27385000');
  await evaluate(`setFrequency(27435000);document.querySelector('#mode-grid [data-mode="LSB"]').click();Object.assign(state,{txDrive:40,txMicGainDb:3,displayZoom:5});saveRadioPrefs()`);
  await attenuation(10);
  await clickBand('ham','7150000');
  await clickBand('swl','27385000');
  check('27 MHz 11m returns from 40m with its last frequency and operating settings',await evaluate(`state.vfoA===27435000 && state.mode==='LSB' && state.rxAttenuationDb===10 && state.txDrive===40 && state.txMicGainDb===3 && state.displayZoom===5 && document.querySelector('#band-grid .band-btn.active').dataset.band==='27385000' && frequencyEntryBandForHz(state.vfoA)?.label==='11m'`));
  await selectGroup('swl');await clickBand('swl','25900000');
  await evaluate(`setFrequency(25950000);document.querySelector('#mode-grid [data-mode="SAM"]').click()`);await attenuation(20);
  await selectGroup('ham');await clickBand('swl','27385000');
  check('27 MHz 11m and 25.9 MHz SWL 11m save separate memories',await evaluate(`state.vfoA===27435000 && state.mode==='LSB' && state.rxAttenuationDb===10 && state.bandMemory['11m'].frequency===27435000 && state.bandMemory['SWL 11m'].frequency===25950000 && state.bandMemory['SWL 11m'].radioPrefs.mode==='SAM'`));
  await evaluate(`setFrequency(3950000);document.querySelector('#mode-grid [data-mode="LSB"]').click()`);await attenuation(10);
  await selectGroup('swl');await clickBand('swl','3950000');
  check('Overlapping 75m SWL does not inherit 80m Ham memory',await evaluate(`state.vfoA===3950000 && state.mode==='AM'`));
  await evaluate(`document.querySelector('#mode-grid [data-mode="SAM"]').click()`);await attenuation(20);
  await evaluate(`setFrequency(3975000)`);
  await selectGroup('ham');await clickBand('ham','3800000');
  check('80m retains its frequency, LSB and attenuation after the overlapping SWL visit',await evaluate(`state.vfoA===3950000 && state.mode==='LSB' && state.rxAttenuationDb===10`));
  await selectGroup('swl');await clickBand('swl','3950000');
  check('Overlapping 75m independently retains its last frequency, SAM and attenuation',await evaluate(`state.vfoA===3975000 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await evaluate(`recallOperationsBandMemory('80m',state.bandMemory['80m']);recallOperationsBandMemory('SWL 75m',state.bandMemory['SWL 75m'])`);
  check('Operations recall keeps overlapping HF and SWL frequency/settings separate',await evaluate(`state.vfoA===3975000 && state.mode==='SAM' && state.rxAttenuationDb===20 && state.bandMemoryGroup==='swl' && state.bandMemory['80m'].frequency===3950000`));
  await command('Page.reload',{},session);check('Reload restores the remembered SWL tab',await ready() && await selectedGroup('swl') && await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===17`));
  await evaluate(`document.getElementById('band-tab-swl').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true,cancelable:true}))`);
  check('Left arrow activates and focuses HF directly',await selectedGroup('ham') && await evaluate(`document.activeElement.id==='band-tab-ham'`));
  await evaluate(`document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true,cancelable:true}))`);
  check('Home activates and focuses Ham HF',await selectedGroup('ham') && await evaluate(`document.activeElement.id==='band-tab-ham'`));
  await evaluate(`document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true,cancelable:true}))`);
  check('End activates and focuses SWL',await selectedGroup('swl') && await evaluate(`document.activeElement.id==='band-tab-swl'`));
  await clickBand('swl','3950000');
  check('SWL last-used frequency/mode/settings survive a page reload',await evaluate(`state.vfoA===3975000 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await selectGroup('ham');await clickBand('ham','7150000');
  check('HF last-used frequency survives a page reload',await evaluate(`state.vfoA===7198300 && state.mode==='LSB' && state.rxAttenuationDb===0`));
  await evaluate(`openFrequencyEntry()`);
  check('Keypad keeps the 11m recall visible independently of the Ham HF panel',await evaluate(`document.querySelector('[data-freq-band="27385000"]').getClientRects().length>0`));
  await evaluate(`document.querySelector('[data-freq-band="27385000"]').click()`);
  check('27 MHz 11m last frequency and settings survive reload and keypad recall',await evaluate(`state.vfoA===27435000 && state.mode==='LSB' && state.rxAttenuationDb===10 && state.txDrive===40 && state.txMicGainDb===3 && state.displayZoom===5&&state.bandPanelGroup==='swl'`));
  await selectGroup('swl');await clickBand('swl','25900000');
  check('SWL 11m still recalls its separate broadcast frequency after reload',await evaluate(`state.vfoA===25950000 && state.mode==='SAM' && state.rxAttenuationDb===20`));
  await selectGroup('swl');await clickBand('swl','3950000');
  await evaluate(`document.querySelector('#band-grid .band-btn.active').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}))`);await pause(100);
  check('Keyboard navigation recalls the visible SWL band memory, never a hidden HF button',await evaluate(`state.vfoA===27435000 && document.activeElement.dataset.bandGroup==='swl' && document.activeElement.getAttribute('aria-checked')==='true'`));
  await evaluate(`document.getElementById('band-group-tabs').scrollIntoView({block:'center'})`);
  const desktop=await command('Page.captureScreenshot',{format:'png'},session);writeFileSync(join(output,'swl-desktop.png'),Buffer.from(desktop.data,'base64'));
  await command('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true},session);await pause(300);
  await evaluate(`document.getElementById('band-group-tabs').scrollIntoView({block:'center'})`);
  check('SWL remains usable on a phone without overflowing its panel',await evaluate(`(()=>{const s=document.getElementById('band-group-tabs'),g=document.getElementById('band-grid'),r=s.getBoundingClientRect();return r.width>0 && r.right<=innerWidth && r.left>=0 && g.scrollWidth<=g.clientWidth+1 && [...g.querySelectorAll('.band-btn')].some(b=>!b.hidden && b.getClientRects().length)})()`));
  await selectGroup('ham');check('Phone can return to the eleven-band HF bank',await evaluate(`document.querySelectorAll('#band-grid .band-btn:not([hidden])').length===11`));
  await selectGroup('swl');
  check('Only HF and SWL tabs and settings choices remain',await evaluate(`document.querySelectorAll('#band-group-tabs [role="tab"]').length===2&&!document.getElementById('band-tab-general')&&[...document.getElementById('band-group').options].map(o=>o.value).join(',')==='ham,swl'`));
  await evaluate(`if(!document.querySelector('#band-grid [data-band="27385000"]').getClientRects().length)document.getElementById('band-more-btn').click()`);await pause(100);
  check('SWL exposes the distinct 27 MHz 11m button on phones',await evaluate(`document.querySelector('#band-grid [data-band="27385000"]').getClientRects().length>0&&document.querySelector('#band-grid [data-band="25900000"]').textContent==='11m BC'`));
  await clickBand('swl','27385000');
  check('Phone 27 MHz recall keeps its last frequency and settings',await evaluate(`state.vfoA===27435000 && state.mode==='LSB' && state.txDrive===40 && state.rxAttenuationDb===10`));
  await evaluate(`localStorage.setItem(BAND_PANEL_KEY,JSON.stringify({panel:'general',memory:'ham'}))`);
  await command('Page.reload',{},session);
  check('Retired General preference restores SWL without losing either 11m memory',await ready() && await selectedGroup('swl') && await evaluate(`state.bandMemory['11m'].frequency===27435000&&state.bandMemory['SWL 11m'].frequency===25950000`));
  await selectGroup('swl');
  const mobile=await command('Page.captureScreenshot',{format:'png'},session);writeFileSync(join(output,'swl-phone.png'),Buffer.from(mobile.data,'base64'));
  check('Band navigation opened no connection, microphone or TX session',await evaluate(`window.__sockets===0 && window.__media===0 && state.txPhase==='rx' && !state.twoToneEnabled`));
  // Exercise the connected production controller, state parser, callbacks,
  // autosave and HTTP persistence. Only host replies are simulated; there is
  // no radio connection and TX/microphone authorization is never supplied.
  connectedSettings=true;
  await evaluate(`(() => {
    clearTimeout(remoteSettingsSaveTimer);remoteSettingsSaveTimer=null;
    Object.assign(state,{activeVfo:'A',vfoA:27385000,vfoB:27385000,dds:27385000,bandMemoryGroup:'ham',mode:'LSB',txDrive:10,rxAttenuationDb:0,bandMemoryPristineConnection:true});
    const prefs={...currentRadioPrefs(),txDrive:60};state.bandMemory=sanitizeBandMemory({
      '11m':{frequency:27385000,radioPrefs:prefs,displayPrefs:currentDisplayPrefs(),displayZoom:6},
      '40m':{frequency:7262000,radioPrefs:{...prefs,mode:'USB',txDrive:50,rxAttenuationDb:10},displayPrefs:currentDisplayPrefs(),displayZoom:3}
    });
    window.__bandHost={...state};window.__bandWire=[];window.__bandPending=[];window.__bandConfig=1;window.__bandSerial=1;
    const send=text=>{if(typeof text!=='string'||/trx:0,true|saturn_tx_arm:|saturn_ps_two_tone:/.test(text))throw Error('RF action forbidden in band fixture');window.__bandWire.push(text);if(_next.managedSettingsMessage(text))window.__bandPending.push(text)};
    state.ws={readyState:WebSocket.OPEN,bufferedAmount:0,send,close(){}};state.connected=true;state.bridgeReady=true;state.remoteClientRole='operator';
    managedPcm.select(true);managedPcmEpoch=managedPcm.connect(send,()=>{throw Error('TX media forbidden')});
    const hello='saturn_tx_media_caps:psg1,pcm,owner1,transactions1;tx_codec_accept:0,pcm;saturn_ps_control_caps:0,measurement-only;saturn_tx_owner:shared,1,ReadyOff,1,1,tci,true,none;saturn_tx_committed_drive:0,1,10;ready;';
    txPathNegotiator=new _next.TxPathNegotiator('auto');txPathNegotiator.observe(hello);txPathNegotiator.takeSnapshot();
    managedPcm.text(hello,managedPcmEpoch);managedPcm.filterTuningEcho(hello);managedPcm.setOperator(true);
    window.__completeBand=()=>{
      const text=window.__bandPending.shift();if(!text)return false;
      const config=++window.__bandConfig,serial=++window.__bandSerial;
      handleTciText('tx_config:0,'+config+',queued,'+config+';');
      window.__bandHost=_next.applyTciText(text,window.__bandHost).state;const h=window.__bandHost;
      const source=/saturn_satp_control:(tci|satp);/.exec(text)?.[1]||h.satp?.source||'tci';h.satp={...h.satp,source};
      handleTciText('saturn_tx_owner:shared,'+serial+',ReadyOff,'+serial+','+config+','+source+',true,none;saturn_tx_committed_drive:0,'+config+','+h.txDrive+';saturn_satp_state:'+source+',true,true,true,healthy,-12.0,512,0,1,false;'
        +'vfo:0,0,'+h.vfoA+';vfo:0,1,'+h.vfoB+';dds:0,'+h.dds+';modulation:0,'+h.mode+';tx_drive:0,'+h.txDrive+';rx_attenuation:0,'+h.rxAttenuationDb+';tx_config:0,'+config+',applied,'+config+';');
      return true;
    };
    setBandPanelGroup('ham');updateUi();
  })()`);
  await clickBand('ham','7150000');
  check('First band departure preserves saved 11m power instead of initial host defaults',await evaluate(`state.bandMemory['11m'].radioPrefs.txDrive===60`));
  const atomicBandRecall=await evaluate(`__bandPending[0].includes('vfo:0,0,7262000;')&&__bandPending[0].includes('tx_drive:0,50;')&&__bandPending[0].includes('modulation:0,USB;')`);
  await evaluate(`handleTciText('vfo:0,0,27385000;dds:0,27385000;modulation:0,LSB;tx_drive:0,10;rx_attenuation:0,0;')`);
  await pause(850);
  check('Pending host readback cannot overwrite destination band memory',await evaluate(`state.bandMemory['40m'].radioPrefs.mode==='USB'&&state.bandMemory['40m'].radioPrefs.txDrive===50`));
  check('Connected band recall sends tuning and saved settings in one transaction',atomicBandRecall);
  check('HTTP autosave waits for the connected settings receipts',settingsWrites.length===0);
  await clickBand('swl','27385000');
  check('Rapid return preserves the previous destination memory despite old echoes',await evaluate(`state.bandMemory['40m'].radioPrefs.mode==='USB'&&state.bandMemory['40m'].radioPrefs.txDrive===50`));
  await evaluate(`setFrequency(27435000)`);
  await clickBand('ham','7150000');await clickBand('swl','27385000');
  check('A fresh VFO tune survives switching away while host settings are pending',await evaluate(`state.vfoA===27435000&&state.bandMemory['11m'].frequency===27435000&&state.bandMemory['11m'].radioPrefs.txDrive===60`));
  for(let n=0;n<20;n++){await evaluate(`__completeBand()`);await pause(120);if(await evaluate(`managedPcm.status().pendingSettings===0`))break;}
  check('Connected band queue completes without fault or keying',await evaluate(`managedPcm.status().phase==='ready'&&managedPcm.status().pendingSettings===0&&state.txPhase==='rx'&&state.txDrive===60&&state.mode==='LSB'`));
  for(let n=0;n<40 && !settingsWrites.length;n++)await pause(100);
  check('Settled HTTP save retains distinct 11m and 40m settings',settingsWrites.at(-1)?.bandMemory?.['11m']?.radioPrefs?.txDrive===60&&settingsWrites.at(-1)?.bandMemory?.['40m']?.radioPrefs?.txDrive===50&&settingsWrites.at(-1)?.bandMemory?.['40m']?.radioPrefs?.mode==='USB');
  // A rejected receive-mode recall must not strand every subsequent memory.
  await evaluate(`state.rxWbfmSupported=true;state.bandMemory.FM={frequency:99500000,radioPrefs:{...currentRadioPrefs(),mode:'WFM'},displayPrefs:currentDisplayPrefs(),displayZoom:2};updateUi()`);
  await clickBand('swl','99500000');
  await evaluate(`(() => {
    if(!__bandPending.shift())throw Error('Missing rejected FM request');
    const id=++__bandConfig;
    handleTciText('tx_config:0,'+id+',rejected,InvalidConfig,0;tx_fault:0,shared_config,InvalidConfig;');
  })()`);
  check('Rejected mode remains an explicit rejection and cannot key',await evaluate(`managedPcm.status().phase==='fault'&&managedPcm.status().lastTransaction.includes('rejected (InvalidConfig)')&&!managedPcm.arm('tci')`));
  await evaluate(`handleTciText('modulation:0,LSB;tx_drive:0,60;');scheduleRemoteSettingsSave()`);
  await pause(850);
  check('Rejected recall does not save old host settings over its requested memory',await evaluate(`state.bandMemory.FM.radioPrefs.mode==='WFM'&&state.bandMemory.FM.frequency===99500000`));
  await evaluate(`updateOperationsMemoryPanel();[...document.querySelectorAll('.operations-memory-button')].find(b=>b.querySelector('strong').textContent==='40M').click()`);
  await pause(100);
  check('Explicit valid band recall reaches the host after a rejected mode',await evaluate(`__bandPending.some(s=>s.includes('vfo:0,0,7262000;')&&s.includes('modulation:0,USB;'))`));
  for(let n=0;n<20;n++){await evaluate(`__completeBand()`);await pause(120);if(await evaluate(`managedPcm.status().pendingSettings===0`))break;}
  check('40m restores its own settings after rejected FM recall',await evaluate(`managedPcm.status().phase==='ready'&&state.vfoA===7262000&&state.mode==='USB'&&state.txDrive===50&&state.rxAttenuationDb===10&&state.bandPanelGroup==='ham'`));
  await clickBand('swl','27385000');
  for(let n=0;n<20;n++){await evaluate(`__completeBand()`);await pause(120);if(await evaluate(`managedPcm.status().pendingSettings===0`))break;}
  check('11m restores its distinct settings after rejected FM recall',await evaluate(`managedPcm.status().phase==='ready'&&state.vfoA===27435000&&state.mode==='LSB'&&state.txDrive===60&&state.rxAttenuationDb===0`));
  await evaluate(`state.rxFilterShiftHz=12000;state.filterLow=0;state.filterHigh=6000;saveRadioPrefs();sendRxFilterBand();updateUi()`);
  for(let n=0;n<20;n++){await evaluate(`__completeBand()`);await pause(120);if(await evaluate(`managedPcm.status().pendingSettings===0`))break;}
  check('Saved off-frequency RX filter shift is visible to the operator',await evaluate(`document.getElementById('rx-filter-shift-readout').textContent==='Shift +12000 Hz'&&!document.getElementById('rx-filter-center-btn').disabled`));
  await evaluate(`document.getElementById('rx-filter-center-btn').click()`);
  check('Center RX filter submits an on-frequency LSB passband without changing the voice profile',await evaluate(`state.rxFilterShiftHz===0&&state.filterLow===0&&state.filterHigh===6000&&state.mode==='LSB'&&state.txDrive===60&&__bandWire.at(-1)==='rx_filter_band:0,-6000,0;'`));
  for(let n=0;n<20;n++){await evaluate(`__completeBand()`);await pause(120);if(await evaluate(`managedPcm.status().pendingSettings===0`))break;}
  await clickBand('ham','7150000');
  for(let n=0;n<20;n++){await evaluate(`__completeBand()`);await pause(120);if(await evaluate(`managedPcm.status().pendingSettings===0`))break;}
  await clickBand('swl','27385000');
  for(let n=0;n<20;n++){await evaluate(`__completeBand()`);await pause(120);if(await evaluate(`managedPcm.status().pendingSettings===0`))break;}
  check('Corrected 27 MHz filter shift survives a band round-trip',await evaluate(`state.vfoA===27435000&&state.rxFilterShiftHz===0&&state.filterHigh===6000&&state.bandMemory['11m'].radioPrefs.rxFilterShiftHz===0`));
  // Actual source selector, managed transaction, rejection and host echo.
  await evaluate(`handleTciText('saturn_satp_state:tci,true,true,true,healthy,-12.0,512,0,1,false;');updateUi();
    window.__selectSource=value=>{const select=document.getElementById('tx-audio-source');select.value=value;select.dispatchEvent(new Event('change',{bubbles:true}))};__selectSource('satp')`);
  await evaluate(`(()=>{const text=__bandPending.shift();if(text!=='saturn_satp_control:satp;')throw Error('Native source selector did not dispatch');const id=++__bandConfig;handleTciText('tx_config:0,'+id+',rejected,CleanupPending,0;tx_fault:0,shared_config,CleanupPending;')})()`);
  check('Source rejection clears the pending selector and remains explicit',await evaluate(`satpSourceRequestAt===0&&managedPcm.status().lastTransaction.includes('rejected (CleanupPending)')`));
  await evaluate(`handleTciText('saturn_satp_state:tci,true,true,true,healthy,-12.0,512,0,1,false;');updateUi();__selectSource('tci')`);
  await evaluate(`__completeBand()`);await pause(120);
  check('Browser mic selection recovers after an explicitly rejected Native request',await evaluate(`managedPcm.status().phase==='ready'&&managedPcm.status().source==='tci'&&state.satp.source==='tci'`));
  await evaluate(`handleTciText('saturn_satp_state:tci,true,true,true,healthy,-12.0,512,0,1,false;');updateUi();__selectSource('satp');__completeBand()`);await pause(120);
  check('Native source selection returns after real queued/applied receipts',await evaluate(`managedPcm.status().phase==='ready'&&managedPcm.status().source==='satp'&&state.satp.source==='satp'`));
  await evaluate(`handleTciText('saturn_satp_state:satp,true,true,true,healthy,-12.0,512,0,1,false;');updateUi();__selectSource('tci');__completeBand()`);await pause(120);
  check('Browser source returns on 11m without changing its voice settings or keying',await evaluate(`state.vfoA===27435000&&state.mode==='LSB'&&state.txDrive===60&&managedPcm.status().source==='tci'&&state.txPhase==='rx'`));
  check('Connected fixture requested no RF, microphone or TX media',await evaluate(`__sockets===0&&__media===0&&state.txPhase==='rx'&&__bandWire.every(s=>!s.includes('trx:0,true'))`));
  await evaluate(`state.connected=false;state.ws=null;managedPcm.disconnect()`);
  await command('Page.reload',{},session);check('Connected band memories reload from HTTP storage',await ready());
  await clickBand('ham','7150000');check('Reloaded 40m memory keeps its own committed settings',await evaluate(`state.vfoA===7262000&&state.mode==='USB'&&state.txDrive===50&&state.rxAttenuationDb===10`));
  await clickBand('swl','27385000');check('Reloaded 11m memory keeps its latest frequency and committed settings',await evaluate(`state.vfoA===27435000&&state.mode==='LSB'&&state.txDrive===60&&state.rxAttenuationDb===0`));
  writeFileSync(join(output,'report.json'),JSON.stringify({passed:true,checks,source:'actual shipped Remote runtime; offline browser, no radio connected'},null,2)+'\n');
  console.log(`Ham/SWL band panel: ${checks.length} checks passed. ${output}`);
} catch(error){
  const bandState = await evaluate(`({frequency:state.vfoA,mode:state.mode,attenuation:state.rxAttenuationDb,watts:state.txDrive,mic:state.txMicGainDb,zoom:state.displayZoom,memoryGroup:state.bandMemoryGroup,memory:state.bandMemory,activeBand:document.querySelector('#band-grid .band-btn.active')?.dataset.band,keypadBand:frequencyEntryBandForHz(state.vfoA)?.label})`).catch(()=>null);
  writeFileSync(join(output,'report.json'),JSON.stringify({passed:false,checks,error:String(error),bandState,settingsWrites},null,2)+'\n');throw error;
} finally { clearTimeout(watchdog);chrome.kill();server.close(); }
