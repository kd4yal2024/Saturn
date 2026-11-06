import { spawn } from 'node:child_process';
import { join } from 'node:path';
export const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
export async function chromeSession(output, run, options = {}) {
  const chrome = spawn(process.env.CHROMIUM || 'google-chrome', [
    '--headless=new','--no-sandbox','--disable-dev-shm-usage','--no-first-run',
    '--disable-background-networking','--disable-extensions','--disable-sync',
    '--disable-popup-blocking','--autoplay-policy=no-user-gesture-required',
    '--mute-audio','--enable-precise-memory-info','--window-size=1600,1100',
    '--remote-debugging-pipe',`--user-data-dir=${join(output,'profile')}`,'about:blank',
  ], {stdio:['ignore','ignore','pipe','pipe','pipe']});
  let next=0, buffer='', stderr=''; const pending=new Map();
  chrome.stderr.on('data', data=>{stderr+=data});
  chrome.stdio[4].on('data', data=>{
    buffer+=data;
    for(let end;(end=buffer.indexOf('\0'))>=0;){
      const raw=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!raw)continue;
      const response=JSON.parse(raw),p=pending.get(response.id);
      if(p){pending.delete(response.id);response.error?p.reject(Error(JSON.stringify(response.error))):p.resolve(response.result)}
    }
  });
  chrome.on('error',error=>{for(const p of pending.values())p.reject(error);pending.clear()});
  const command=(method,params={},sessionId)=>new Promise((resolve,reject)=>{
    const id=++next;pending.set(id,{resolve,reject});
    chrome.stdio[3].write(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})})+'\0');
  });
  const watchdog=setTimeout(()=>{chrome.kill();for(const p of pending.values())p.reject(Error('Chrome timeout: '+stderr.slice(-1000)))},options.timeoutMs ?? 180000);
  try{
    const {targetId}=await command('Target.createTarget',{url:'about:blank'});
    const {sessionId}=await command('Target.attachToTarget',{targetId,flatten:true});
    const call=(method,params={})=>command(method,params,sessionId);
    const evaluate=async expression=>{
      const r=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});
      if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);
      return r.result.value;
    };
    await call('Page.enable');await call('Runtime.enable');
    return await run({command,call,evaluate,targetId,sessionId});
  }finally{clearTimeout(watchdog);chrome.kill()}
}
