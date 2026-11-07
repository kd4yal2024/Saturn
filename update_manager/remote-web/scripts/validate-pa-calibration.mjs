#!/usr/bin/env node
// Shipped PA panel and bundle in Chrome, without a radio or a TX action.
import {createServer} from 'node:http';
import {readFileSync,writeFileSync,mkdtempSync,mkdirSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
import {chromeSession,pause} from './lib/chrome-session.mjs';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const output=process.argv.find(x=>x.startsWith('--output='))?.slice(9)||mkdtempSync(join(tmpdir(),'saturn-pa-calibration-'));
mkdirSync(output,{recursive:true});
const panel=readFileSync(join(root,'../templates/saturn-remote-next.html'),'utf8').match(/<details class="tx-advanced-card" id="tx-pa-calibration">[\s\S]*?<\/details>/)?.[0];assert.ok(panel);
const bundle=readFileSync(join(root,'dist/saturn-remote-next.js'));
const html=`<!doctype html><html><head><meta charset="utf-8"><style>body{background:#081425;color:#dbe8fc;font:16px system-ui;padding:30px}details{max-width:850px}button,input,select{margin:8px;padding:10px}p{line-height:1.6}</style></head><body>${panel}<script>
window.__sockets=0;window.WebSocket=class{constructor(){window.__sockets++;throw Error('No radio in fixture')}};
</script><script src="/bundle.js"></script><script>
const api=window.SaturnRemoteNext||window.SaturnRemoteNextBundle;
window.requests=[];window.view={requested:null,host:undefined,drive:undefined,canApply:false};
window.panel=new api.PaCalibrationPanel(document.getElementById('tx-pa-calibration'),p=>{requests.push(api.paCalibrationCommand(p));view.requested=p;});
window.paint=()=>panel.update(view);window.field=n=>document.querySelector('[data-pa="'+n+'"]');
document.querySelector('details').open=true;paint();window.__ready=true;</script></body></html>`;
const server=createServer((req,res)=>{if(req.url==='/bundle.js'){res.setHeader('Content-Type','text/javascript');res.end(bundle)}else{res.setHeader('Content-Type','text/html');res.end(html)}});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const checks=[];const check=(name,value)=>{assert.ok(value,name);checks.push(name)};
try {
 await chromeSession(output,async({call,evaluate})=>{
  await call('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/`});
  for(let i=0;i<100&&!await evaluate('Boolean(window.__ready)');i++)await pause(100);
  check('Starts on legacy without host permission',await evaluate(`field('apply').disabled&&field('gain').disabled&&field('status').textContent.includes('host not confirmed')`));
  await evaluate(`field('defaults').click();field('band').selectedIndex=9;field('band').dispatchEvent(new Event('change'))`);
  check('G2 values are a draft and do not send a command',await evaluate(`requests.length===0&&field('gain').value==='46.5'&&field('status').textContent.includes('draft')`));
  await evaluate(`view.canApply=true;view.host=null;paint();field('gain').value='48.5';field('gain').dispatchEvent(new Event('change'));field('apply').click();paint()`);
  check('Apply sends one complete profile with the independent 11m gain',await evaluate(`requests.length===1&&view.requested[9]===48.5&&view.requested[10]===46.5&&requests[0].startsWith('tx_pa_calibration:0,')`));
  check('Requested profile is not reported as host applied',await evaluate(`field('status').textContent.includes('requested profile differs')&&field('status').textContent.includes('host: legacy curve')`));
  await evaluate(`view.host=[...view.requested];view.drive='11m · host drive command 54/255 at 40 W target; RF unmeasured';paint()`);
  check('Host confirmation and command are distinguished from RF power',await evaluate(`!field('status').textContent.includes('requested profile differs')&&field('status').textContent.includes('RF unmeasured')`));
  await evaluate(`field('gain').value='81';field('gain').dispatchEvent(new Event('change'))`);
  check('Invalid edit restores the previous valid displayed gain',await evaluate(`field('gain').value==='48.5'`));
  await evaluate(`view.canApply=false;paint();field('legacy').click();field('apply').click()`);
  check('A keyed/unavailable host cannot apply a profile',await evaluate(`requests.length===1&&field('apply').disabled`));
  await evaluate(`view.canApply=true;paint();field('apply').click();paint()`);
  check('Legacy fallback is an explicit profile action',await evaluate(`requests.length===2&&requests[1]==='tx_pa_calibration:0,legacy;'`));
  check('No radio socket or RF action created',await evaluate('__sockets===0'));
  const shot=await call('Page.captureScreenshot',{format:'png'});writeFileSync(join(output,'offline-pa-calibration.png'),Buffer.from(shot.data,'base64'));
 },{timeoutMs:60000});
 const result={checks,passed:checks.length,offline:true,rfTestPerformed:false};writeFileSync(join(output,'result.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({output,...result},null,2));
} finally { await new Promise(done=>server.close(done)); }
