import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, it, expect } from 'vitest';
import { TxPowerDraft } from '../src/settings/tx-power';
import { clampTxDriveWatts } from '../src/settings/normalize';

const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
function handler(event: string, context: object) {
  const marker = `$("tx-drive").addEventListener("${event}", () => {`;
  const start = template.indexOf(marker) + marker.length;
  if (start < marker.length) throw Error('Missing power handler');
  const end = template.indexOf('\n      });', start);
  return runInNewContext(`() => {${template.slice(start, end)}\n}`, context);
}
describe('actual TX power slider commit', () => {
  it.each([50,100])('preserves %i W across repeated old host echoes before change', watts => {
    const txPowerDraft = new TxPowerDraft();
    const state = {txDrive:40};
    const slider = {value:String(watts), max:'100'};
    const readout = {textContent:''};
    const sent: string[] = [], saved: number[] = [];
    const context = {state,txPowerDraft,clampTxDriveWatts,
      $:(id:string)=>id==='tx-drive'?slider:readout,
      saveRadioPrefs:()=>saved.push(state.txDrive), sendTci:(s:string)=>sent.push(s)};
    handler('input',context)();
    const echo = template.split('\n').find(line=>line.includes('!txPowerDraft.editing && status.committedWatts'))!;
    expect(echo).toBeDefined();
    for(let n=0;n<20;n++) {
      runInNewContext(echo,{...context,status:{committedWatts:40,pendingSettings:0,phase:'ready'}});
      slider.value=String(txPowerDraft.displayed(state.txDrive));
    }
    expect(slider.value).toBe(String(watts));
    handler('change',context)();
    expect(sent).toEqual([`tx_drive:0,${watts};`]);
    expect(saved).toEqual([watts]);
    expect(txPowerDraft.editing).toBe(false);
    runInNewContext(echo,{...context,status:{committedWatts:watts,pendingSettings:0,phase:'ready'}});
    expect(state.txDrive).toBe(watts);
  });
  it('respects a changed host ceiling and discards a disconnected draft',()=>{
    const d=new TxPowerDraft();d.preview(100);expect(d.commit(100,60)).toBe(60);
    d.preview(50);d.cancel();expect(d.displayed(10)).toBe(10);
  });
});
