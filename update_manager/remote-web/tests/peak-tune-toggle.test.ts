import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {describe,it,expect} from 'vitest';

const template=readFileSync(new URL('../../templates/saturn-remote-next.html',import.meta.url),'utf8');
function bodyAfter(marker:string,end:string) {
  const start=template.indexOf(marker)+marker.length;
  if(start<marker.length)throw Error('Missing actual peak control');
  return template.slice(start,template.indexOf(end,start));
}
describe('actual Peak Tune toolbar toggle',()=>{
  it.each([true,false])('turns back off and persists the choice, usable peak=%s',usable=>{
    const state={peakTuneAssistEnabled:false,peakTuneFrequencyHz:7_200_700,peakTuneLevelDb:-70,mode:'USB'};
    const nodes:Record<string,{checked?:boolean;hidden?:boolean}>= {
      'display-peak-tune-assist':{checked:false},'peak-tune-marker':{hidden:false},'peak-tune-readout':{hidden:false}
    };
    const saved:boolean[]=[],tuned:number[]=[];
    let scheduled=0,paints=0;
    const context={state,$:(id:string)=>nodes[id],peakTuneTracker:{},emptyPeakAssistTrackingState:()=>({}),
      updatePeakTuneAssist:()=>usable?{frequencyHz:7_200_700,levelDb:-70}:null,
      currentRemoteSettings:()=>({displayPrefs:{peakTuneAssistEnabled:state.peakTuneAssistEnabled}}),
      persistLocalSettingsFallback:(s:{displayPrefs:{peakTuneAssistEnabled:boolean}})=>saved.push(s.displayPrefs.peakTuneAssistEnabled),
      scheduleRemoteSettingsSave:()=>scheduled++,updateUi:()=>paints++,
      snapTuneFrequencyHz:(n:number)=>n,clickTuneCarrierOffsetHz:()=>700,
      setFrequency:(n:number)=>tuned.push(n),formatFrequencyPlain:String,safeFixed:String,logEvent:()=>{}
    };
    const clear=bodyAfter('    function clearPeakTuneAssist(resetTracker = true) {','\n    function updatePeakTuneAssist');
    runInNewContext(`function clearPeakTuneAssist(resetTracker=true){${clear}\nthis.clearPeakTuneAssist=clearPeakTuneAssist`,context);
    const click=runInNewContext(`()=>{${bodyAfter('$("spectrum-tune-peak-btn").addEventListener("click", () => {','\n      });')}\n}`,context);
    click();
    expect(state.peakTuneAssistEnabled).toBe(true);
    expect(nodes['display-peak-tune-assist']!.checked).toBe(true);
    expect(tuned).toEqual(usable?[7_200_000]:[]);
    const tuningBefore=tuned.length;
    click();
    expect(state.peakTuneAssistEnabled).toBe(false);
    expect(nodes['display-peak-tune-assist']!.checked).toBe(false);
    expect(nodes['peak-tune-marker']!.hidden).toBe(true);
    expect(nodes['peak-tune-readout']!.hidden).toBe(true);
    expect(state.peakTuneFrequencyHz).toBeNull();
    expect(tuned.length).toBe(tuningBefore);
    expect(saved).toEqual([true,false]);expect(scheduled).toBe(2);expect(paints).toBeGreaterThan(0);
  });
});
