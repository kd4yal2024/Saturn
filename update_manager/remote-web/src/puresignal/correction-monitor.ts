import {PanelWindow} from './panel-layout';
/** Read-only WDSP AmpView. Curve existence, snapshot freshness and correction
 * application are independent. A recent readback does NOT date the fit. */
import {copyRfMeasurements} from './rf-monitor';
import {canvasScale,RF_COLORS} from './rf-plot';
export interface CorrectionFrame {
  format:'saturn-ps-correction-v1';boot:string;serial:string;snapshotNs:string;publishedNs:string;
  sourcePin:string;backend:'P2'|'DirectXdma';centerHz:number;commandedWatts:number;attenuationCommandDb:number;
  psRequested:boolean;keyed:boolean;nativeApplied:boolean;calibrations:number;nativeState:number;
  nativeSamples:number;phaseReferenceDeg:number;diagnosticDrops:string;curveAge:'unknown';rfImdQualified:false;
  measured:number[][];correction:number[][];
  nativeInfo?:number[];
}
const finite=(n:unknown):n is number=>typeof n==='number'&&Number.isFinite(n);
const id=(n:unknown):n is string=>typeof n==='string'&&/^\d{1,39}$/.test(n);
export function parseCorrection(text:string):CorrectionFrame {
  if(text.length>160000)throw new Error('Correction display frame too large');
  const r=JSON.parse(text) as CorrectionFrame;
  const points=(a:unknown,max:number)=>Array.isArray(a)&&a.length<=max&&a.every(p=>Array.isArray(p)&&p.length===4&&p.every(n=>finite(n)&&Math.abs(n)<=1e6));
  if(!r||r.format!=='saturn-ps-correction-v1'||![r.boot,r.serial,r.snapshotNs,r.publishedNs,r.diagnosticDrops].every(id)
    || !['DirectXdma','P2'].includes(r.backend)||typeof r.sourcePin!=='string'||!/^[a-zA-Z0-9._-]{1,80}$/.test(r.sourcePin)
    || ![r.centerHz,r.commandedWatts,r.attenuationCommandDb,r.calibrations,r.nativeState,r.nativeSamples,r.phaseReferenceDeg].every(finite)
    ||r.centerHz<=0||r.commandedWatts<0||r.commandedWatts>100||r.attenuationCommandDb<0||r.attenuationCommandDb>31
    ||![r.nativeSamples,r.calibrations,r.nativeState].every(Number.isInteger)||r.nativeSamples<0||r.nativeSamples>4096
    ||![r.psRequested,r.keyed,r.nativeApplied].every(n=>typeof n==='boolean')||r.curveAge!=='unknown'||r.rfImdQualified!==false
    ||(r.nativeInfo!==undefined&&(!Array.isArray(r.nativeInfo)||r.nativeInfo.length!==16||!r.nativeInfo.every(n=>Number.isInteger(n)&&n>=-2147483648&&n<=2147483647)
      ||r.nativeInfo[5]!==r.calibrations||r.nativeInfo[15]!==r.nativeState))
    ||!points(r.measured,1024)||!points(r.correction,512)||r.measured.length!==Math.min(r.nativeSamples,1024)
    ||r.correction.length!==(r.nativeSamples>0?512:0)||BigInt(r.snapshotNs)>BigInt(r.publishedNs))throw new Error('Invalid correction display contract');
  // Correction x-coordinates must be ordered and normalized. Measured scatter
  // can include overrange points; never silently re-order or fit those values.
  for(let i=0;i<r.correction.length;i++)for(const col of [0,2]){
    const x=r.correction[i]![col]!;
    if(x<0||x>1||(i>0&&x<r.correction[i-1]![col]!))throw new Error('Invalid correction curve coordinates');
  }
  return r;
}
export const phaseDegrees=(cos:number,sin:number,reference:number):number=>wrapPhase(Math.atan2(sin,cos)*180/Math.PI-reference);
export const wrapPhase=(degrees:number):number=>((degrees+180)%360+360)%360-180;
export class CorrectionMonitorState {
  frame:CorrectionFrame|null=null;capability=false;note='No native correction display received';
  private generation=-1;private requests=new Map<string,{at:number;barrier:boolean}>();
  private timing:{boot:string;lower:number;at:number}|null=null;private lastTick:number|null=null;
  private transmitting=false;private barrier:bigint|null=null;
  reset(generation:number):void {this.generation=generation;this.frame=null;this.capability=false;this.timing=null;this.requests.clear();this.lastTick=null;this.barrier=null;this.note='Waiting for native correction display';}
  request(nonce:string,generation:number,now:number):void {
    if(generation!==this.generation)this.reset(generation);
    if(!id(nonce)||!finite(now)||now<0)return;
    this.requests.set(nonce,{at:now,barrier:this.transmitting&&this.barrier===null});
    if(this.requests.size>16)this.requests.delete(this.requests.keys().next().value!);
  }
  setTransmitting(active:boolean):void {if(active!==this.transmitting){this.transmitting=active;this.barrier=null;this.requests.clear();}}
  observe(text:string,generation:number,now:number):void {
    if(generation!==this.generation)this.reset(generation);
    for(const part of text.split(';')){
      if(part.startsWith('saturn_ps_correction_caps:'))this.capability=part==='saturn_ps_correction_caps:wdsp-ampview-v1';
      else if(part.startsWith('saturn_ps_spectrum_clock:')){
        const [nonce,boot,host]=part.slice('saturn_ps_spectrum_clock:'.length).split(',');const req=nonce?this.requests.get(nonce):undefined;
        if(!req||!id(boot)||!id(host)||now<req.at||now-req.at>500)continue;
        this.requests.delete(nonce!);this.timing={boot,lower:req.at-Number(BigInt(host))/1e6,at:now};
        if(req.barrier&&this.transmitting)this.barrier=BigInt(host);
      } else if(part.startsWith('saturn_ps_correction:')){
        try {
          const r=parseCorrection(part.slice('saturn_ps_correction:'.length));
          if(!this.validAge(r,now)) {this.note='Delayed native snapshot rejected';continue;}
          if(this.frame&&this.frame.boot===r.boot&&BigInt(r.serial)<=BigInt(this.frame.serial))continue;
          this.frame=r;this.note='Native readback only; fit age and curve configuration unknown';
        }catch{this.note='Invalid native correction display rejected';}
      }
    }
  }
  private validAge(r:CorrectionFrame,now:number):boolean {
    if(!this.timing||this.timing.boot!==r.boot||now<this.timing.at||now-this.timing.at>2000)return false;
    const age=now-Number(BigInt(r.snapshotNs))/1e6-this.timing.lower;
    return age>=0&&age<=1250;
  }
  current(now:number,connected:boolean,transmitting:boolean):boolean {
    this.setTransmitting(connected&&transmitting);
    if(this.lastTick!==null&&(now<this.lastTick||now-this.lastTick>1250)){
      this.timing=null;this.requests.clear();this.barrier=null;this.note='Consumer stalled; timing invalidated';
    }
    this.lastTick=now;
    return connected&&this.capability&&!!this.frame&&this.validAge(this.frame,now)
      &&(!transmitting||(this.barrier!==null&&BigInt(this.frame.snapshotNs)>=this.barrier&&this.frame.keyed));
  }
  status(generation:number,now:number,connected:boolean,transmitting:boolean):CorrectionFrame|null {
    return generation===this.generation&&this.current(now,connected,transmitting)?this.frame:null;
  }
}
export function correctionSummary(r:CorrectionFrame):string {
  return `${r.backend} · WDSP native AmpView · ${r.centerHz.toLocaleString()} Hz commanded now (curve frequency unknown)\n`
    +`${r.commandedWatts} commanded W (not measured); attenuation command ${r.attenuationCommandDb} dB.\n`
    +`Native IQC run flag: ${r.nativeApplied?'enabled':'disabled'}; TX owner keyed report: ${r.keyed}; PS requested: ${r.psRequested}.\n`
    +`Calibration counter ${r.calibrations}; native state code ${r.nativeState}. ${r.measured.length}/${r.nativeSamples} measured scatter points; ${r.correction.length}/512 correction points.\n`
    +`Phase = atan2(ys, yc) in degrees minus WDSP reference ${r.phaseReferenceDeg.toFixed(3)}°; displayed modulo 360°. Correction phase is WDSP ya_cor (degrees).\n`
    +'Magnitude uses WDSP ym / ym_cor, in the native inverse-gain convention; dB view is 20 log10(ratio). Not a forward PA-gain calibration.\n'
    +`Snapshot ${r.snapshotNs} ns; published ${r.publishedNs} ns; serial ${r.serial}; boot ${r.boot}. Host ${r.sourcePin}; diagnostic drops ${r.diagnosticDrops}.\n`
    +'Snapshot freshness is not calibration age. Curve age/configuration and overload coverage unknown. Native correction is not proof of RF improvement or calibrated RF IMD.';
}
type View={status:HTMLElement;summary:HTMLElement;gain:HTMLCanvasElement;phase:HTMLCanvasElement};
function plot(canvas:HTMLCanvasElement,series:{points:number[][];color:string;line:boolean}[],ylabel:string,range:[number,number],live:boolean):void {
  const rect=canvas.getBoundingClientRect();if(rect.width<2||rect.height<2)return;
  const c=canvas.getContext('2d');if(!c)return;
  const w=Math.round(rect.width),h=Math.round(rect.height),scale=canvasScale(w,h,canvas.ownerDocument.defaultView?.devicePixelRatio??1);
  if(canvas.width!==Math.round(w*scale))canvas.width=Math.round(w*scale);if(canvas.height!==Math.round(h*scale))canvas.height=Math.round(h*scale);
  c.setTransform(scale,0,0,scale,0,0);const bg=c.createLinearGradient(0,0,0,h);bg.addColorStop(0,'#0c1c30');bg.addColorStop(1,'#050b14');c.fillStyle=bg;c.fillRect(0,0,w,h);
  const left=62,right=w-15,top=25,bottom=h-34;
  const x=(n:number)=>left+n*(right-left),y=(n:number)=>bottom-(n-range[0])/(range[1]-range[0])*(bottom-top);
  c.font='11px ui-monospace,monospace';c.lineWidth=.6;
  for(let i=0;i<=10;i++){
    const n=range[0]+i*(range[1]-range[0])/10;c.strokeStyle=i%2?'#14293c':'#29445e';c.beginPath();c.moveTo(left,y(n));c.lineTo(right,y(n));c.stroke();
    c.beginPath();c.moveTo(x(i/10),top);c.lineTo(x(i/10),bottom);c.stroke();
    if(i%2===0){c.fillStyle='#9fb4c9';c.fillText(n.toFixed(1),4,y(n)+4);c.fillText((i/10).toFixed(1),x(i/10)-8,bottom+15);}
  }
  c.strokeStyle='#49627b';c.strokeRect(left,top,right-left,bottom-top);
  c.fillStyle='#cfe2f4';c.fillText(ylabel,left,14);c.fillText('Normalized magnitude (WDSP x)',left,h-6);
  c.textAlign='right';c.fillStyle=live?'#69dcba':'#a4b2c1';c.fillText(live?'LIVE':'HELD / NOT LIVE',right,h-6);c.textAlign='left';
  c.save();c.beginPath();c.rect(left,top,right-left,bottom-top);c.clip();
  c.globalAlpha=live?1:.7;
  for(const s of series){c.strokeStyle=s.color;c.fillStyle=s.color;c.lineWidth=2;c.beginPath();let previous:number[]|null=null;
    for(const p of s.points){if(!p.every(finite)){previous=null;continue;}
      if(s.line){if(previous&&Math.abs(p[1]!-previous[1]!)<(range[1]-range[0])*.8)c.lineTo(x(p[0]!),y(p[1]!));else c.moveTo(x(p[0]!),y(p[1]!));previous=p;}
      else c.fillRect(x(p[0]!)-.7,y(p[1]!)-.7,1.4,1.4);
    }if(s.line)c.stroke();
  }c.restore();
}
export function mountCorrectionMonitor(root:HTMLElement,model:CorrectionMonitorState):(connected:boolean,transmitting:boolean,now:number)=>void {
  const main:View={status:root.querySelector('[data-correction-status]')!,summary:root.querySelector('[data-correction-summary]')!,gain:root.querySelector('[data-correction-gain]')!,phase:root.querySelector('[data-correction-phase]')!};
  let popup:Window|null=null,child:View|null=null,db=true;
  const decorate=(view:View,legend:HTMLElement)=>{
    const doc=view.summary.ownerDocument;legend.replaceChildren();legend.style.cssText='display:flex;flex-wrap:wrap;gap:8px 18px;margin:10px 0;font:12px system-ui';
    for(const [label,color] of [['Magnitude measured',RF_COLORS.trace],['Magnitude correction',RF_COLORS.imd3],['Phase measured',RF_COLORS.imd5],['Phase correction',RF_COLORS.fundamental]]){
      const item=doc.createElement('span');item.textContent='━ '+label;item.style.color=color!;legend.append(item);
    }
    const record=doc.createElement('details'),title=doc.createElement('summary');title.textContent='Curve details';record.style.cssText='margin:12px 0;font-size:12px';
    view.summary.before(record);record.append(title,view.summary);
  };
  decorate(main,root.querySelector('[data-correction-legend]')!);
  const bindCopy=(doc:Document,view:View,button:HTMLButtonElement,feedback:HTMLElement)=>{
    button.addEventListener('click',()=>{void copyRfMeasurements(view,feedback,'Saturn WDSP Correction / Phase Monitor');});
    const exportButton=doc.createElement('button');exportButton.type='button';exportButton.textContent='Export curve JSON';exportButton.setAttribute('data-correction-export','');
    button.after(exportButton);
    exportButton.addEventListener('click',()=>{
      if(!model.frame)return;const win=doc.defaultView;if(!win)return;
      const text=JSON.stringify({format:'saturn-ps-correction-export-v1',copiedAtIso:new Date().toISOString(),displayStatus:view.status.textContent,qualification:view.summary.textContent,frame:model.frame},null,2);
      const url=URL.createObjectURL(new Blob([text],{type:'application/json'}));const a=doc.createElement('a');a.href=url;a.download='saturn-wdsp-correction.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
    });
  };
  bindCopy(root.ownerDocument,main,root.querySelector('[data-correction-copy]')!,root.querySelector('[data-correction-copy-status]')!);
  root.querySelector('[data-correction-scale]')!.addEventListener('change',e=>{db=(e.target as HTMLSelectElement).value==='db';});
  const panelWindow=new PanelWindow(root,'ampview',{width:1100,height:900});
  root.querySelector('[data-correction-popout]')!.addEventListener('click',()=>{
    popup=panelWindow.open();if(!popup){if(root.dataset.panelMode!=='expanded')main.status.textContent='Pop-out blocked; Correction / Phase remains docked';return;}
    popup.opener=null;const doc=popup.document;doc.title='Saturn WDSP Correction / Phase Monitor';
    const body=doc.body;body.style.cssText='background:#07111f;color:#d8e7f7;font:14px system-ui;padding:12px';
    const style=doc.createElement('style');style.textContent='button,select{background:#173750;color:#e0eefb;border:1px solid #3b5d79;border-radius:5px;padding:6px 10px}button{cursor:pointer}button:hover{background:#244964}h2{font-size:20px}';doc.head.append(style);
    const title=doc.createElement('h2');title.textContent='WDSP Correction / Phase Monitor — read only';
    const legend=doc.createElement('div');
    const scaleLabel=doc.createElement('label');scaleLabel.textContent='Magnitude scale ';const scale=doc.createElement('select');
    for(const [value,text] of [['db','dB ratio'],['linear','Linear ratio']]){const option=doc.createElement('option');option.value=value!;option.textContent=text!;scale.append(option);}
    scale.value=db?'db':'linear';scale.addEventListener('change',()=>{db=scale.value==='db';(root.querySelector('[data-correction-scale]') as HTMLSelectElement).value=scale.value;});scaleLabel.append(scale);
    const status=doc.createElement('p'),summary=doc.createElement('pre'),gain=doc.createElement('canvas'),phase=doc.createElement('canvas');
    summary.style.cssText='white-space:pre-wrap;user-select:text';for(const c of [gain,phase])c.style.cssText='display:block;position:relative;inset:auto;width:100%;height:clamp(240px,calc((100vh - 280px)/2),420px);margin:10px 0';
    gain.setAttribute('aria-label','WDSP magnitude correction graph');phase.setAttribute('aria-label','WDSP phase correction graph in degrees');
    const copy=doc.createElement('button');copy.type='button';copy.textContent='Copy measurements';copy.setAttribute('data-correction-copy','');
    const feedback=doc.createElement('div');feedback.setAttribute('role','status');
    body.replaceChildren(title,status,legend,scaleLabel,gain,phase,copy,feedback,summary);child={status,summary,gain,phase};decorate(child,legend);panelWindow.attach(popup);
    bindCopy(doc,child,copy,feedback);
    // Populate immediately; opening a window never initiates fitting/keying.
    status.textContent=main.status.textContent;summary.textContent=main.summary.textContent;
  });
  return (connected,transmitting,now)=>{
    const fresh=model.current(now,connected,transmitting),r=model.frame;
    const label=!connected?'DISCONNECTED — no live correction evidence':!model.capability?'UNAVAILABLE — host native correction display not enabled':!r?`WAITING — ${model.note}`:!fresh?`STALE / held native snapshot — ${model.note}`:r.nativeSamples===0?'NO CALIBRATION CURVE — fresh native readback; no fitted curve available':!transmitting?'RX / held native curve — not live':`LIVE native readback — correction ${r.nativeApplied?'applied (WDSP flag)':'not applied'}; fit age unknown`;
    const summary=r?correctionSummary(r):'Waiting for real WDSP GetPSDisp-equivalent data. No synthetic curves. Opening this panel cannot enable PureSignal or key the radio.';
    const magnitude=(n:number)=>db?(n>0?20*Math.log10(n):NaN):n;
    const measured=r?.measured.map(p=>[p[0]!,magnitude(p[1]!)])??[],curve=r?.correction.map(p=>[p[0]!,magnitude(p[1]!)])??[];
    const measuredPhase=r?.measured.filter(p=>Math.hypot(p[2]!,p[3]!)>1e-12).map(p=>[p[0]!,phaseDegrees(p[2]!,p[3]!,r.phaseReferenceDeg)])??[];
    const phaseCurve=r?.correction.map(p=>[p[2]!,wrapPhase(p[3]!)])??[];
    const bounds=(points:number[][],low:number,high:number):[number,number]=>{
      const values=points.map(p=>p[1]!).filter(finite);let min=Math.min(low,...values),max=Math.max(high,...values);const margin=(max-min)*.08;return[min-margin,max+margin];
    };
    for(const view of [main,...(child&&popup&&!popup.closed?[child]:[])]){
      view.status.textContent=label+(root.dataset.panelNotice?` · ${root.dataset.panelNotice}`:'');view.summary.textContent=summary;
      plot(view.gain,[{points:measured,color:RF_COLORS.trace,line:false},{points:curve,color:RF_COLORS.imd3,line:true}],db?'Magnitude / inverse gain (dB)':'Magnitude / inverse gain (linear)',bounds([...measured,...curve],db?-1:0,db?1:1.2),fresh&&transmitting);
      plot(view.phase,[{points:measuredPhase,color:RF_COLORS.imd5,line:false},{points:phaseCurve,color:RF_COLORS.fundamental,line:true}],'Relative phase (degrees)',bounds([...measuredPhase,...phaseCurve],-5,5),fresh&&transmitting);
    }
  };
}
