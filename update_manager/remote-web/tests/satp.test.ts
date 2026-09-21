import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { parseSatpState, satpTxBlockReason, txUsesBrowserMic } from '../src/audio/satp';
import { applyTciText } from '../src/tci/apply';
import { createAppState } from '../src/state/app-state';
import type { TciRadioState } from '../src/tci/state';
import { runInNewContext } from 'node:vm';

const args = ['satp', 'true', 'true', 'true', 'healthy', '-12.0', '512', '0', '1', 'false'];

function pttHarness(source: 'tci' | 'satp', ready = true) {
  const html=readFileSync(new URL('../../templates/saturn-remote-next.html',import.meta.url),'utf8');
  const start=html.indexOf('    async function setPtt(');
  const code=html.slice(start,html.indexOf('    async function setTwoToneEnabled',start));
  const state: any={satp:{...parseSatpState(args,100)!,source,ready},connected:true,
    ws:{readyState:1},moxRequested:false,txEnabled:false,pttRequestId:0,audioStreaming:true};
  const commands: string[]=[]; let micCalls=0;
  const context: any={state,performance:{now:()=>100},satpSourceRequestAt:0,WebSocket:{OPEN:1},TX_LOCAL_REQUEST_PERMIT_MS:1000,
    _next:{txUsesBrowserMic,satpTxBlockReason:(s:any)=>satpTxBlockReason(s,100)},
    rfDisabledBlocksTx:()=>false,clientRoleBlocksTx:()=>false,normalizeTxReadyState:()=>true,
    generatedTxSourceActive:()=>false,sendTci:(s:string)=>commands.push(s),
    startMicCapture:async()=>{micCalls++;return false;}, // browser mic unavailable/denied
    sendTxShutdownIfPossible:()=>commands.push('trx:0,false;')};
  for(const name of ['updateUi','refreshBridgeControlHeartbeatForTx','applyTxSourceOverrides','setTxMediaPriority',
    'muteRxAudioForMox','switchDisplayToTxArm','logEvent','clearTxTimeouts','lockTx','recordOperatorFault',
    'stopMicCapture','recoverRxAfterMox','extendTxReadyWindow','scheduleNextBridgeRttPing']) context[name]=()=>{};
  const setPtt=runInNewContext(`${code}; setPtt`,context) as (active:boolean)=>Promise<boolean>;
  return {setPtt,state,commands,micCalls:()=>micCalls};
}
describe('native SATP TX integration', () => {
  it('executes shipped native PTT and release with browser microphone unavailable', async () => {
    const h=pttHarness('satp');
    expect(await h.setPtt(true)).toBe(true);
    expect(h.micCalls()).toBe(0);
    expect(h.commands).toContain('trx:0,true,tci;');
    expect(await h.setPtt(false)).toBe(true);
    expect(h.commands).toContain('trx:0,false;');
    expect(h.state.moxRequested).toBe(false);
  });
  it('blocks native PTT before sending key-on when audio is not ready', async () => {
    const h=pttHarness('satp',false);
    expect(await h.setPtt(true)).toBe(false); expect(h.commands).toEqual([]);
  });
  it('keeps browser-mode microphone failure cancellation', async () => {
    const h=pttHarness('tci');
    expect(await h.setPtt(true)).toBe(false); expect(h.micCalls()).toBe(1);
    expect(h.commands).toContain('trx:0,false;');
  });
  it('uses acknowledged native audio without requiring a browser microphone', () => {
    const s = parseSatpState(args, 100)!;
    expect(txUsesBrowserMic(s)).toBe(false);
    expect(txUsesBrowserMic(undefined)).toBe(true);
    expect(txUsesBrowserMic({...s, source:'tci'})).toBe(true);
    expect(txUsesBrowserMic(undefined, false)).toBe(false);
    expect(satpTxBlockReason(s, 200)).toBeNull();
  });
  it('blocks stale, pending, unpaired, disabled and lost native audio', () => {
    const s = parseSatpState(args, 100)!;
    for (const changed of [{pending:true}, {paired:false}, {enabled:false}, {ready:false}]) {
      expect(satpTxBlockReason({...s, ...changed}, 200)).not.toBeNull();
    }
    expect(satpTxBlockReason(s, 2000)).not.toBeNull();
    expect(satpTxBlockReason({...s, source:'tci', paired:false, ready:false}, 200)).toBeNull();
  });
  it('rejects malformed status and accepts authoritative TCI status without changing MON', () => {
    expect(parseSatpState([])).toBeNull();
    expect(parseSatpState(['bad', ...args.slice(1)])).toBeNull();
    expect(parseSatpState([...args.slice(0,5), 'NaN', ...args.slice(6)])).toBeNull();
    const state=createAppState(); state.txMonitorSupported=true; state.txMonitorEnabled=true;
    const result=applyTciText(`saturn_satp_state:${args.join(',')};`, state as unknown as TciRadioState);
    expect(result.state.satp?.source).toBe('satp');
    expect(result.state.txMonitorEnabled).toBe(true);
  });
  it('wires SATP into real PTT, resets pairing on disconnect, and keeps MON at G2', () => {
    const html=readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url),'utf8');
    expect(html).toContain('_next.txUsesBrowserMic(state.satp, voiceRequested)');
    expect(html).toContain('_next.satpTxBlockReason(state.satp)');
    expect(html).toContain('saturn_satp_control:renew;');
    expect(html.match(/resetSatpState\(\);/g)?.length).toBeGreaterThanOrEqual(2);
    expect(html).toContain('MON: G2 headphones');
    const handlers=html.slice(html.indexOf('$("tx-mon-btn").addEventListener'),html.indexOf('$("tx-drive").addEventListener'));
    expect(handlers).not.toContain('source ===');
    expect(handlers).toContain('tx_monitor:0,');
  });
});
