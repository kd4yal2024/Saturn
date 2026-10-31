#!/usr/bin/env node
// Actual template + shipped IIFE, isolated Chromium profile, no network/RF.
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawn } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
// Test harness only: the actual run_inner fixture launches this against its
// ephemeral loopback port and ordinary files. Never point this at a G2 tunnel.
const replayUrl = process.argv.find(s => s.startsWith('--offline-fixture='))?.split('=').slice(1).join('=');
if (replayUrl && !/^ws:\/\/127\.0\.0\.1:\d+\/tci$/.test(replayUrl)) throw new Error('ephemeral loopback fixture URL required');
const measurementDuration = Number(process.argv.find(s => s.startsWith('--measurement-duration-ms='))?.split('=')[1] ?? 500);
const timingOnly = process.argv.includes('--timing-only');
assert(!timingOnly || replayUrl, 'timing action requires the isolated offline fixture');
assert([500, 45000].includes(measurementDuration), 'explicit short or long offline replay duration only');
assert(measurementDuration === 500 || replayUrl, 'long action requires the offline fixture');
const output = mkdtempSync(join(tmpdir(), 'saturn-ps-tab-browser-'));
const source = readFileSync(resolve(root, '../templates/saturn-remote-next.html'), 'utf8');
const bundle = readFileSync(join(root, 'dist/saturn-remote-next.js'), 'utf8');
const tag = /<script\s+src="\/remote-assets\/remote-next\.js[^"]*"[\s\S]*?<\/script>/;
assert(tag.test(source));
const page = join(output, 'remote.html');
writeFileSync(page, source.replace(tag, `<script>${bundle.replaceAll('</script', '<\\/script')}</script>`));
const chrome = spawn(process.env.CHROMIUM || 'google-chrome', ['--headless=new', '--no-sandbox',
  '--disable-dev-shm-usage', '--no-first-run', '--disable-background-networking', '--disable-extensions',
  '--disable-sync', '--mute-audio', '--allow-file-access-from-files', '--remote-debugging-pipe',
  `--user-data-dir=${join(output, 'profile')}`, 'about:blank'],
{ stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'] });
let sequence = 0, buffer = '', stderr = '';
const pending = new Map();
chrome.stderr.on('data', data => { stderr = (stderr + data).slice(-4000); });
chrome.on('error', error => { for (const p of pending.values()) p.reject(error); pending.clear(); });
chrome.stdio[4].on('data', data => {
  buffer += data.toString();
  let end;
  while ((end = buffer.indexOf('\0')) >= 0) {
    const raw = buffer.slice(0, end); buffer = buffer.slice(end + 1);
    if (!raw) continue;
    const message = JSON.parse(raw), p = pending.get(message.id);
    if (!p) continue;
    pending.delete(message.id);
    message.error ? p.reject(new Error(JSON.stringify(message.error))) : p.resolve(message.result);
  }
});
function command(method, params = {}, sessionId) {
  return new Promise((resolveCall, rejectCall) => {
    const id = ++sequence; pending.set(id, { resolve: resolveCall, reject: rejectCall });
    chrome.stdio[3].write(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }) + '\0');
  });
}
const pause = ms => new Promise(done => setTimeout(done, ms));
const watchdog = setTimeout(() => { console.error('Browser deadline', stderr); chrome.kill(); process.exit(2); }, 60000 + 2 * measurementDuration);
const reports = [];
try {
  const { targetId } = await command('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await command('Target.attachToTarget', { targetId, flatten: true });
  const call = (method, params = {}) => command(method, params, sessionId);
  const evaluate = async expression => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  };
  await call('Page.enable'); await call('Runtime.enable');
  await call('Page.addScriptToEvaluateOnNewDocument', { source: `
    window.__psOffline = { commands: [], socketAttempts: 0, errors: [], mediaFrames:0, mediaNonzero:0 };
    window.fetch = async () => new Response('{}', { status: 404 });
    ${replayUrl ? `localStorage.setItem('saturn.remote.splitTransport', 'off');
    const Socket = window.WebSocket;
    window.WebSocket = class extends Socket {
      constructor() { super(${JSON.stringify(replayUrl)}); }
      send(value) { if (typeof value === 'string') window.__psOffline.commands.push(value);
        else if(value instanceof ArrayBuffer){window.__psOffline.mediaFrames++;
          if(new Uint8Array(value).subarray(64).some(v=>v!==0))window.__psOffline.mediaNonzero++;}
        super.send(value); }
    };` : `window.WebSocket = class extends EventTarget {
      static CONNECTING=0; static OPEN=1; static CLOSING=2; static CLOSED=3;
      constructor() { super(); this.readyState=3; window.__psOffline.socketAttempts++; }
      send(text) { window.__psOffline.commands.push(String(text)); }
      close() {}
    };`}
    window.addEventListener('error', e => window.__psOffline.errors.push(e.message));
    window.addEventListener('unhandledrejection', e => window.__psOffline.errors.push(String(e.reason)));
  ` });
  for (const width of replayUrl ? [1440] : [1440, 390]) {
    await call('Emulation.setDeviceMetricsOverride', { width, height: 960, deviceScaleFactor: 1, mobile: width < 600 });
    await call('Page.navigate', { url: pathToFileURL(page).href + (replayUrl ? '?force_legacy_ws=1' : '') });
    let ready = false;
    for (let n = 0; n < 100; n++) {
      await pause(50);
      ready = await evaluate('document.readyState === "complete" && !!document.getElementById("control-tab-ps")').catch(() => false);
      if (ready) break;
    }
    assert(ready, `Full runtime startup failed (${width}): ${JSON.stringify(await evaluate('window.__psOffline'))}`);
    if (replayUrl) {
      const until = async (expression, label, timeoutMs = 10000) => {
        const deadline = Date.now() + timeoutMs;
        while (Date.now() < deadline) { if (await evaluate(expression)) return; await pause(50); }
        throw new Error(label + ': ' + JSON.stringify(await evaluate(`({status: document.getElementById('managed-pcm-status').textContent,
          result: document.getElementById('setup-ps-measurement-result').textContent,
          controls: Object.fromEntries(['setup-ps-owner','setup-ps-source','setup-ps-control-contract',
            'setup-ps-committed-mode','setup-ps-lock-reason','setup-ps-measurement-limits',
            'tx-arm-btn','mode-select'].map(id => {const el=document.getElementById(id);
              return [id,el ? {text:el.textContent,value:el.value,disabled:el.disabled}:null];})),
          log: document.getElementById('event-log')?.textContent?.slice(-3000), trace: window.__psOffline})`)));
      };
      assert.equal(await evaluate(`document.getElementById('managed-pcm-select').value`), 'auto');
      await evaluate(`window.saturnConnectBridgeDiagnostic()`);
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')`, 'Managed negotiation');
      const autoCommands = await evaluate(`window.__psOffline.commands.slice()`);
      assert(autoCommands.includes('tx_codec_caps:0,pcm;'), JSON.stringify(autoCommands));
      assert(!autoCommands.some(c=>/tx_puresignal_auto_attenuate:|tx_two_tone:|tx_codec_caps:0,pcm,opus/.test(c)), JSON.stringify(autoCommands));
      await evaluate(`(() => { const drive = document.getElementById('tx-drive'); drive.value='5';
        drive.dispatchEvent(new Event('input')); drive.dispatchEvent(new Event('change')); })()`);
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')`, 'Settings commit');
      await until(`!document.getElementById('setup-ps-off').disabled`, 'RX restore and committed configuration');
      assert.equal(await evaluate(`document.getElementById('tx-drive').max`), '5');
      for (const watts of [3,5]) {
        await evaluate(`(() => {const p=document.getElementById('setup-ps-commanded-watts');p.value='${watts}';p.dispatchEvent(new Event('change'));})()`);
        await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')
          && document.getElementById('setup-ps-power-result').textContent.includes('Host committed command: ${watts} W')`, `PS-page power commit ${watts}`);
      }
      // Separate actual DOM changes without advancing the host between them.
      const burstBefore = await evaluate(`window.__psOffline.commands.filter(c=>c.startsWith('tx_mic_gain:')).length`);
      await evaluate(`(() => {for(let i=0;i<18;i++){const gain=document.getElementById('tx-mic-gain');gain.value=String(-i);gain.dispatchEvent(new Event('input'));gain.dispatchEvent(new Event('change'));}})()`);
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')
        && window.__psOffline.commands.filter(c=>c.startsWith('tx_mic_gain:')).length===${burstBefore+18}`, '18 separate settings committed without owner overflow', 15000);
      // Two distinct messages, each through the actual UI/host commit path.
      // Lowering drive must not ratchet away startup's reviewed ceiling.
      for (const watts of [3, 5]) {
        await evaluate(`(() => { const drive=document.getElementById('tx-drive');drive.value=${JSON.stringify(String(watts))};
          drive.dispatchEvent(new Event('input'));drive.dispatchEvent(new Event('change')); })()`);
        await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')`, `Restore drive ${watts}`);
      }
      // Reproduce ordinary RX interaction through the actual UI, not a fake
      // controller: band recall followed by a burst of absolute VFO updates.
      // The fixture is loopback/files only; no key command precedes this check.
      await evaluate(`document.querySelector('#band-grid [data-band="7150000"]').click()`);
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')`, '40m recall commit');
      // Real input arrives on separate event-loop turns. Do not manufacture a
      // multi-second renderer stall by executing 500 full UI renders in one
      // JavaScript task: the existing settings watchdog must remain effective.
      for (let chunk=0; chunk<50; chunk++) {
        await evaluate(`(() => { for (let i=0; i<10; i++) document.body.dispatchEvent(new KeyboardEvent('keydown', {key:'ArrowUp', bubbles:true, cancelable:true})); })()`);
        await pause(10);
      }
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')`, 'VFO burst commit');
      const tuning = await evaluate(`({status: document.getElementById('managed-pcm-status').textContent,
        commands: window.__psOffline.commands.slice(), errors: window.__psOffline.errors.slice()})`);
      assert(!tuning.commands.some(c => /saturn_ps_measurement:0,start|trx:0,true/.test(c)), JSON.stringify(tuning));
      assert(tuning.commands.some(c => /vfo:0,0,7200000;/.test(c)), JSON.stringify(tuning));
      assert.deepEqual(tuning.errors, []);
      writeFileSync(join(output, 'rx-tuning-result.json'), JSON.stringify(tuning, null, 2));
      // Interleave knob input with committed echoes as well as a synchronous
      // burst. Old echoes must not change the next increment's starting point.
      for (let chunk=0; chunk<20; chunk++) {
        await evaluate(`(() => { for (let i=0; i<10; i++) document.body.dispatchEvent(new KeyboardEvent('keydown', {key:${JSON.stringify(chunk % 2 ? 'ArrowDown' : 'ArrowUp')}, bubbles:true, cancelable:true})); })()`);
        await pause(20);
      }
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')`, 'Interleaved VFO echo commit');
      // Real slider -> transaction -> committed host echo, including zero.
      // Leave normal gain at -12 to prove MeasurementOnly does not inherit it.
      for (const db of [-12, 0, -12]) {
        const before = await evaluate(`document.getElementById('managed-pcm-transaction').textContent`);
        await evaluate(`(() => { const gain=document.getElementById('tx-mic-gain');gain.value='${db}';
          gain.dispatchEvent(new Event('input'));gain.dispatchEvent(new Event('change')); })()`);
        await until(`document.getElementById('managed-pcm-transaction').textContent !== ${JSON.stringify(before)}
          && document.getElementById('managed-pcm-transaction').textContent.includes(': applied')
          && document.getElementById('managed-pcm-status').textContent.startsWith('ready:')
          && document.getElementById('tx-mic-gain-readout').textContent === '${db.toFixed(1)} dB'`, `Committed mic gain ${db}`);
      }
      await until(`document.getElementById('setup-ps-stimulus').textContent.includes('fixed 0 dB')`, 'Host unity measurement policy');
      if (timingOnly) {
        await evaluate(`(() => {const select=document.getElementById('setup-ps-measurement-kind');select.value='timing';select.dispatchEvent(new Event('change'));})()`);
        await until(`document.getElementById('setup-ps-stimulus').textContent.includes('Timing Measurement')
          && document.getElementById('setup-ps-stimulus').textContent.includes('two-tone IMD unavailable')
          && !document.getElementById('setup-ps-stimulus').textContent.includes('capability not advertised')`, 'Host timing capability');
      }
      await evaluate(`document.getElementById('control-tab-ps').click(); document.getElementById('tx-arm-btn').click();
        document.getElementById('setup-ps-duration-ms').value='${measurementDuration}';
        document.getElementById('setup-ps-attended').checked=true;
        document.getElementById('setup-ps-attended').dispatchEvent(new Event('change'));`);
      await until(`!document.getElementById('setup-ps-measure').disabled`, 'Measurement action availability');
      writeFileSync(join(output, 'before-measurement.json'), JSON.stringify(await evaluate(`({
        frequency: document.getElementById('tx-zone-frequency').textContent,
        limits: document.getElementById('setup-ps-measurement-limits').textContent,
        attended: document.getElementById('setup-ps-attended').checked,
        duration: document.getElementById('setup-ps-duration-ms').value,
        status: document.getElementById('managed-pcm-status').textContent})`), null, 2));
      await evaluate(`document.getElementById('setup-ps-measure').click()`);
      const toneStarted = Date.now();
      await until(`document.getElementById('setup-ps-measurement-result').textContent.includes('Saved on Bridge:')`, 'Measurement cleanup/export', measurementDuration + 15000);
      assert(Date.now() - toneStarted >= measurementDuration - 100, 'short pulse cannot pass long replay');
      const result = await evaluate(`({result: document.getElementById('setup-ps-measurement-result').textContent,
        status: document.getElementById('managed-pcm-status').textContent, trace: window.__psOffline})`);
      assert(!result.status.startsWith('fault:'), JSON.stringify(result));
      assert.deepEqual(result.trace.errors, [], JSON.stringify(result));
      assert.equal(await evaluate(`document.getElementById('tx-mic-gain-readout').textContent`), '-12.0 dB');
      assert(result.trace.commands.some(c => c === `saturn_ps_measurement:0,start,${measurementDuration}${timingOnly ? ',saturn-timing-multisine-v1' : ',wdsp-postgen-two-tone-v1'};`));
      if(!timingOnly){assert(result.trace.mediaFrames>0);assert.equal(result.trace.mediaNonzero,0,'native tone must use genuine zero-input keepalive, never mic/PCM tones');}
      assert(!result.trace.commands.some(c => /tx_puresignal:0,true|trx:0,true/.test(c)));
      if (timingOnly) {
        assert.equal(result.trace.commands.filter(c=>c.startsWith('saturn_ps_measurement:0,start,')).length,1);
        assert.equal(await evaluate(`document.getElementById('setup-ps-calibrate').disabled`),true);
        writeFileSync(join(output,'timing-result.json'),JSON.stringify(result,null,2));
        console.log(JSON.stringify({event:'exported',actualUi:true,fixtureOnly:true,timingOnly:true,physicalRfQualified:false,output,result}));
        break;
      }
      // Second finite action through the SAME real browser/native/runtime.
      // A silent AM source must key and clean up without microphone speech.
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')`, 'First stimulus fully clean');
      await evaluate(`document.querySelector('#mode-grid [data-mode="AM"]').click()`);
      await until(`document.getElementById('managed-pcm-status').textContent.startsWith('ready:')
        && document.getElementById('setup-ps-stimulus').textContent.includes('AM silent carrier')`, 'AM settings committed');
      await evaluate(`document.getElementById('setup-ps-duration-ms').value='${measurementDuration}';
        document.getElementById('setup-ps-attended').checked=true;
        document.getElementById('setup-ps-attended').dispatchEvent(new Event('change'));`);
      await until(`!document.getElementById('setup-ps-measure').disabled`, 'Silent AM action availability');
      await evaluate(`document.getElementById('setup-ps-measure').click()`);
      const amStarted = Date.now();
      await until(`document.getElementById('setup-ps-measurement-result').textContent.includes('Saved on Bridge:')`, 'Silent AM cleanup/export', measurementDuration + 15000);
      assert(Date.now() - amStarted >= measurementDuration - 100, 'short AM pulse cannot pass long replay');
      const am = await evaluate(`({result:document.getElementById('setup-ps-measurement-result').textContent,
        status:document.getElementById('managed-pcm-status').textContent, stimulus:document.getElementById('setup-ps-stimulus').textContent,
        trace:window.__psOffline})`);
      assert(!am.status.startsWith('fault:'),JSON.stringify(am));assert.deepEqual(am.trace.errors,[]);
      assert.equal(am.trace.commands.filter(c=>c===`saturn_ps_measurement:0,start,${measurementDuration};`).length,1);
      assert(!am.trace.commands.some(c=>/tx_puresignal:0,true|trx:0,true/.test(c)));
      writeFileSync(join(output,'am-carrier-result.json'),JSON.stringify(am,null,2));
      const releaseBefore = await evaluate(`window.__psOffline.commands.filter(c=>c==='trx:0,false;').length`);
      // Deliberately bypass the UI ceiling in this isolated fixture to retain
      // the real Bridge's pre-queue rejection regression.
      await evaluate(`(() => { const drive = document.getElementById('tx-drive'); drive.max='100';drive.value='6';
        drive.dispatchEvent(new Event('input')); drive.dispatchEvent(new Event('change')); })()`);
      await until(`document.getElementById('managed-pcm-status').textContent.includes('InvalidConfig')`, 'Real Bridge rejection reason');
      await pause(700);
      const rejected = await evaluate(`({status: document.getElementById('managed-pcm-status').textContent,
        log: document.getElementById('event-log')?.textContent,
        releases: window.__psOffline.commands.filter(c => c === 'trx:0,false;').length})`);
      assert.equal(rejected.releases - releaseBefore, 1, JSON.stringify({releaseBefore,...rejected}));
      writeFileSync(join(output, 'rejected-settings-result.json'), JSON.stringify(rejected, null, 2));
      writeFileSync(join(output, 'fixture-result.json'), JSON.stringify(result, null, 2));
      console.log(JSON.stringify({event:'exported', actualUi:true, fixtureOnly:true, physicalRfQualified:false, output, result}));
      break;
    }
    const before = await evaluate(`(() => {
      const $ = id => document.getElementById(id);
      return { tabs: [...document.querySelectorAll('.context-tab')].map(b => b.textContent),
        parent: $('setup-tx-puresignal-section').parentElement.id,
        controls: document.querySelectorAll('#setup-tx-puresignal-enabled').length,
        feedback: $('setup-tx-puresignal-feedback').textContent,
        checked: $('setup-tx-puresignal-enabled').checked,
        disabled: $('setup-tx-puresignal-enabled').disabled };
    })()`);
    assert.deepEqual(before.tabs, ['RX', 'DSP', 'TX', 'PS']);
    assert.equal(before.parent, 'ps-control-body'); assert.equal(before.controls, 1);
    assert.equal(before.feedback, 'Unavailable'); assert.equal(before.checked, false); assert.equal(before.disabled, true);
    await evaluate(`document.getElementById('control-tab-ps').click()`);
    const selected = await evaluate(`(() => {
      const $ = id => document.getElementById(id);
      $('radio-context-rail').scrollTop = 0; window.scrollTo(0, 0);
      const rect = $('ps-control-panel').getBoundingClientRect();
      return { selected: $('control-tab-ps').getAttribute('aria-selected'), hidden: $('ps-control-panel').hidden,
        width: rect.width, height: rect.height, pttCount: document.querySelectorAll('#ptt-btn').length,
        commands: window.__psOffline.commands };
    })()`);
    assert.equal(selected.selected, 'true'); assert.equal(selected.hidden, false);
    assert(selected.width > 200 && selected.height > 100); assert.equal(selected.pttCount, 1);
    assert.deepEqual(selected.commands, []);
    // Programmatic events cannot bypass the read-only/disconnected guard.
    const denied = await evaluate(`(() => {
      const c = document.getElementById('setup-tx-puresignal-enabled'); c.checked = true;
      c.dispatchEvent(new Event('change')); return { checked: c.checked, commands: window.__psOffline.commands };
    })()`);
    assert.equal(denied.checked, false); assert.deepEqual(denied.commands, []);
    // Settings search is only a navigation route to this same main PS surface.
    await evaluate(`document.getElementById('header-setup-btn').click();
      document.querySelector('[data-settings-section="transmit"]').click();
      document.querySelector('[data-settings-entry="transmit.puresignal"]').click();`);
    assert(await evaluate(`document.getElementById('setup-menu').hidden && !document.getElementById('ps-control-panel').hidden`));
    await call('Page.reload'); await pause(350);
    assert(await evaluate(`document.getElementById('radio-context-rail').dataset.activeContext === 'ps'
      && document.getElementById('setup-tx-puresignal-feedback').textContent === 'Unavailable'`));
    const after = await evaluate('window.__psOffline');
    assert.deepEqual(after.commands, []); assert.deepEqual(after.errors, []);
    await evaluate(`document.getElementById('radio-context-rail').scrollTop = 0; window.scrollTo(0, 0)`);
    const shot = await call('Page.captureScreenshot', { format: 'png' });
    writeFileSync(join(output, `ps-${width}.png`), Buffer.from(shot.data, 'base64'));
    reports.push({ width, before, selected, after, noRfOrControlCommands: true });
  }
  writeFileSync(join(output, 'result.json'), JSON.stringify({ passed: true, reports, g2Access: false, rfRequested: false }, null, 2));
  console.log(JSON.stringify({ passed: true, widths: reports.map(r => r.width), output,
    fixtureMeasurementRequested: Boolean(replayUrl), physicalRfExecuted: false, g2Access: false }));
} finally {
  clearTimeout(watchdog);
  if (chrome.exitCode === null) await command('Browser.close').catch(() => {});
  chrome.kill();
}
