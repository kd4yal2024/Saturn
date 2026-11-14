// Runs only an explicitly supplied Rust TEST executable. No installed Bridge,
// radio address or backend can be selected. Each workload owns a fresh process.
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pause } from './chrome-session.mjs';

export async function startDisplayBridge(binary, output) {
  mkdirSync(output, {recursive:true});
  if(existsSync(join(output,'ready.json'))||existsSync(join(output,'stop')))throw Error('Use a fresh fixture output directory');
  // Reserve an ephemeral loopback port. Binding races fail the fixture startup;
  // we never retry against another service or connect without its ready record.
  const reserve = createServer();
  await new Promise((done, reject) => {reserve.once('error', reject);reserve.listen(0, '127.0.0.1', done)});
  const port = reserve.address().port;
  await new Promise(done => reserve.close(done));
  const proc = spawn(resolve(binary), ['--exact', 'tci::display_replay::loopback_display_fixture', '--ignored', '--nocapture'], {
    env:{...process.env, SATURN_DISPLAY_REPLAY_DIR:resolve(output), SATURN_DISPLAY_REPLAY_PORT:String(port)},
    stdio:['ignore','pipe','pipe'],
  });
  let log=''; let ended=false;
  proc.stdout.on('data', data=>{log+=data});proc.stderr.on('data', data=>{log+=data});
  const exit = new Promise((done,reject)=>{proc.once('error', reject);proc.once('exit',(code,signal)=>{ended=true;done({code,signal})})});
  let stopped=false;
  async function stop() {
    if(stopped)return;stopped=true;
    writeFileSync(join(output,'stop'),'');
    const deadline=setTimeout(()=>proc.kill('SIGKILL'),4000);
    try {
      const status=await exit;
      writeFileSync(join(output,'bridge.log'),log);
      if(status.code!==0)throw Error('Display fixture failed: '+JSON.stringify(status)+' '+log.slice(-3000));
      return JSON.parse(readFileSync(join(output,'result.json'),'utf8'));
    } finally {clearTimeout(deadline)}
  }
  try {
    let ready;
    for(let i=0;i<100&&!ended;i++) {
      if(existsSync(join(output,'ready.json'))) {ready=JSON.parse(readFileSync(join(output,'ready.json'),'utf8'));break}
      await pause(50);
    }
    if(!ready||ready.port!==port||ready.hardwareAccess!==false||!ready.synthetic)throw Error('Display fixture did not become ready: '+log.slice(-2000));
    return {url:`ws://127.0.0.1:${port}/tci`,pid:proc.pid,stop};
  } catch(error) {try{await stop()}catch{}throw error}
}

export function readProcessCpu(pid) {
  const s=readFileSync(`/proc/${pid}/stat`,'utf8');
  const fields=s.slice(s.lastIndexOf(')')+2).trim().split(/\s+/);
  return {ticks:Number(fields[11])+Number(fields[12]),startTicks:fields[19]};
}
