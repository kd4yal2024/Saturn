import {PanelWindow} from './panel-layout';
import {DEFAULT_PLOT,referenceOffset,productLabel,type RfReference} from './rf-plot';
import {createDisplay,updateMetrics,type ViewDisplay} from './rf-plot-controls';
/** Read-only RF monitor. It cannot send a radio command or authorize RF/PS. */
export interface RfProduct { order: number; offsetHz: number; dbc: number | null; upperBoundDbc: number | null; reason: string }
export interface RfSpectrum {
  stimulus?: 'unspecified' | 'two-tone-700-1900' | 'am-silent-carrier' | 'saturn-timing-multisine-v1' | 'wdsp-postgen-two-tone-v1';
  requestedStimulus?: {version: string; seed?: number; pcmRateHz?: number; repeatMs?: number};
  format: 'saturn-rf-spectrum-v1'; boot: string; serial: string; generation: string; config: string; txEpoch: string;
  sourcePin: string; sourceBoot: string; firmwareBuild: string;
  backend: 'DirectXdma' | 'P2'; source: 'ReturnedRfIq'; observedFirstNs: string; observedLastNs: string; publishedNs: string;
  rateHz: number; fftSize: number; binHz: number; rbwHz: number; window: string; centerHz: number;
  attenuationCommandDb: number; commandedWatts: number; psRequested: boolean; overload: 'clear' | 'unknown';
  continuity: 'contiguous'; diagnosticDrops: string; reference: string; rfImdQualified: false; reason: string;
  componentPeak: number; clippedComponents: number; toneBalanceDb: number; toneHz: number[]; toneDbfs: number[];
  referenceToneDbfs: number[]; products: RfProduct[]; firstBinHz: number; binsDbfs: number[];
}
const finite = (n: unknown): n is number => typeof n === 'number' && Number.isFinite(n);
const id = (n: unknown): n is string => typeof n === 'string' && /^\d{1,39}$/.test(n);
export function parseRfSpectrum(text: string): RfSpectrum {
  if (text.length > 24_000) throw new Error('RF spectrum exceeds bounded frame size');
  const r = JSON.parse(text) as RfSpectrum;
  if (!r || r.format !== 'saturn-rf-spectrum-v1' || r.source !== 'ReturnedRfIq' || !['P2', 'DirectXdma'].includes(r.backend)
    || ![r.boot,r.sourceBoot,r.serial,r.generation,r.config,r.txEpoch,r.observedFirstNs,r.observedLastNs,r.publishedNs,r.diagnosticDrops].every(id)
    || typeof r.sourcePin!=='string' || !/^[a-zA-Z0-9._-]{1,80}$/.test(r.sourcePin) || r.firmwareBuild!=='not supplied by this observer'
    || r.rateHz !== 192000 || r.fftSize !== 16384 || r.window !== 'Blackman-Harris 4-term'
    || ![r.binHz,r.rbwHz,r.centerHz,r.attenuationCommandDb,r.commandedWatts,r.componentPeak,r.clippedComponents,r.toneBalanceDb,r.firstBinHz].every(finite)
    || Math.abs(r.binHz-r.rateHz/r.fftSize)>0.00001 || r.rbwHz<r.binHz || r.rbwHz>r.binHz*3
    || r.centerHz<=0 || r.attenuationCommandDb<0 || r.attenuationCommandDb>31 || r.commandedWatts<0 || r.commandedWatts>100
    || typeof r.psRequested !== 'boolean' || !['clear','unknown'].includes(r.overload) || r.continuity!=='contiguous'
    || r.rfImdQualified!==false || r.reference!=='weaker fundamental; not PEP'
    || typeof r.reason!=='string' || r.reason.length>200 || r.componentPeak<0 || !Number.isInteger(r.clippedComponents) || r.clippedComponents<0
    || (!Array.isArray(r.toneHz)||r.toneHz.length!==2||!r.toneHz.every(finite))
    || ![r.toneDbfs,r.referenceToneDbfs].every(a=>Array.isArray(a)&&a.length===2&&a.every(v=>finite(v)&&v>=-400&&v<=100))
    || !Array.isArray(r.binsDbfs) || r.binsDbfs.length<16 || r.binsDbfs.length>2048 || !r.binsDbfs.every(v=>finite(v)&&v>=-400&&v<=100)
    || !Array.isArray(r.products) || r.products.length!==6
    || !r.products.every((p,i)=>p && p.order===[3,3,5,5,7,7][i] && finite(p.offsetHz)
      && (p.dbc===null||finite(p.dbc)) && (p.upperBoundDbc===null||finite(p.upperBoundDbc))
      && !(p.dbc!==null && p.upperBoundDbc!==null) && typeof p.reason==='string' && p.reason.length<200)
    || BigInt(r.observedFirstNs)>BigInt(r.observedLastNs) || BigInt(r.observedLastNs)>BigInt(r.publishedNs))
    throw new Error('Invalid RF spectrum contract');
  if (r.stimulus !== undefined && !['unspecified','two-tone-700-1900','am-silent-carrier','saturn-timing-multisine-v1','wdsp-postgen-two-tone-v1'].includes(r.stimulus))
    throw new Error('Unknown RF stimulus');
  if (r.stimulus === 'saturn-timing-multisine-v1' && (r.requestedStimulus?.version !== r.stimulus
    || r.requestedStimulus.seed !== 0x53465031 || r.requestedStimulus.pcmRateHz !== 48000 || r.requestedStimulus.repeatMs !== 1000))
    throw new Error('Missing timing waveform identity');
  if(r.stimulus === 'wdsp-postgen-two-tone-v1' && r.requestedStimulus?.version !== r.stimulus)
    throw new Error('Missing native two-tone waveform identity');
  if (!twoToneSpectrum(r) && r.products.some(p=>p.dbc!==null || p.upperBoundDbc!==null))
    throw new Error('Non-two-tone stimulus may not publish IMD');
  const delta=r.toneHz[1]!-r.toneHz[0]!;
  if (delta<r.binHz*24 || r.products.some((p,i)=>Math.abs(p.offsetHz-(i%2===0?r.toneHz[0]!-(Math.floor(i/2)+1)*delta:r.toneHz[1]!+(Math.floor(i/2)+1)*delta))>0.01))
    throw new Error('Inconsistent IMD marker frequencies');
  return r;
}
function twoToneSpectrum(r: RfSpectrum): boolean {
  return r.stimulus !== 'saturn-timing-multisine-v1' && r.stimulus !== 'am-silent-carrier';
}

/** Ping exchange bounds clock offset without assuming synchronized clocks or
 * zero transit time. A delayed spectrum cannot arrive looking newly observed.
 * Clock invalidated after consumer stalls, disconnect or connection generation.
 */
export class RfMonitorState {
  frame: RfSpectrum | null = null;
  baseline: RfSpectrum | null = null;
  private generation = -1;
  private requests = new Map<string,{sent:number; barrier:boolean}>();
  private timing: {boot:string; offsetLower:number; at:number} | null = null;
  private lastTick: number | null = null;
  private receipt = 0;
  private transmitting = false;
  private minimumObservation: bigint | null = null;
  note = 'RF feedback unavailable — no live capture received';
  capability = 'unavailable';
  reset(generation: number): void {
    this.generation=generation; this.frame=null; this.timing=null; this.requests.clear(); this.lastTick=null;
    this.note='RF feedback unavailable — waiting for paired acquisition';
    this.minimumObservation=null;
    this.capability='unavailable';
  }
  request(nonce: string, generation: number, now: number): void {
    if (generation!==this.generation) this.reset(generation);
    if (!id(nonce) || !finite(now) || now<0) return;
    this.requests.set(nonce,{sent:now,barrier:this.transmitting&&this.minimumObservation===null});
    if (this.requests.size>16) this.requests.delete(this.requests.keys().next().value!);
  }
  observe(text: string, generation: number, now: number): void {
    if (generation!==this.generation) this.reset(generation);
    for (const raw of text.split(';')) {
      if (raw.startsWith('saturn_ps_spectrum_caps:')) {
        this.capability=raw.slice('saturn_ps_spectrum_caps:'.length)==='raw-pairs-v1'?'raw-pairs-v1':'unavailable';
      } else if (raw.startsWith('saturn_ps_spectrum_clock:')) {
        const [nonce,boot,host]=raw.slice('saturn_ps_spectrum_clock:'.length).split(',');
        const request=nonce===undefined?undefined:this.requests.get(nonce);
        if (!request || !id(boot) || !id(host) || now<request.sent || now-request.sent>500) continue;
        this.requests.delete(nonce!);
        const hostMs=Number(BigInt(host))/1e6;
        this.timing={boot,offsetLower:request.sent-hostMs,at:now};
        if(request.barrier&&this.transmitting)this.minimumObservation=BigInt(host);
      } else if (raw.startsWith('saturn_ps_spectrum:')) {
        try {
          const frame=parseRfSpectrum(raw.slice('saturn_ps_spectrum:'.length));
          if(!this.transmitting || this.minimumObservation===null || BigInt(frame.observedFirstNs)<this.minimumObservation)continue;
          if (!this.timing || this.timing.boot!==frame.boot || now-this.timing.at>2000) { this.note='RF timing unavailable — waiting for a bounded clock exchange'; continue; }
          const maxAge=now-Number(BigInt(frame.observedFirstNs))/1e6-this.timing.offsetLower;
          if (maxAge<0 || maxAge>750) { this.note='Stale RF capture rejected'; continue; }
          if (this.frame && this.frame.boot===frame.boot && BigInt(frame.serial)<=BigInt(this.frame.serial)) continue;
          this.frame=frame; this.receipt=now; this.note='RF-feedback relative measurements — observation path not yet qualified';
        } catch { this.note='Invalid RF spectrum rejected'; }
      }
    }
  }
  current(now: number, connected: boolean, transmitting: boolean): boolean {
    this.setTransmitting(connected&&transmitting);
    if (this.lastTick!==null && (now-this.lastTick>750 || now<this.lastTick)) {this.timing=null;this.frame=null;this.minimumObservation=null;this.requests.clear();this.note='Monitor consumer stalled — timing invalidated';}
    this.lastTick=now;
    if (!connected || !transmitting || this.capability!=='raw-pairs-v1') return false;
    if (!this.frame || !this.timing || this.minimumObservation===null || BigInt(this.frame.observedFirstNs)<this.minimumObservation || now-this.timing.at>2000 || now-this.receipt>750) return false;
    const age=now-Number(BigInt(this.frame.observedFirstNs))/1e6-this.timing.offsetLower;
    return age>=0 && age<=750;
  }
  setTransmitting(active: boolean): void {
    if(active!==this.transmitting){this.minimumObservation=null;this.requests.clear();}
    this.transmitting=active;
  }
  saveBaseline(now=this.receipt): boolean {
    if(!this.current(now,true,this.transmitting))return false;
    if (!this.frame || !twoToneSpectrum(this.frame) || this.frame.psRequested || this.frame.clippedComponents>0 || !this.frame.reason.startsWith('relative estimate')) return false;
    this.baseline=structuredClone(this.frame); return true;
  }
  baselineMatches(frame: RfSpectrum): boolean {
    const b=this.baseline;
    return !!b && b.boot===frame.boot && b.sourceBoot===frame.sourceBoot && twoToneSpectrum(b) && twoToneSpectrum(frame) && b.backend===frame.backend && b.centerHz===frame.centerHz
      && b.stimulus===frame.stimulus
      && b.attenuationCommandDb===frame.attenuationCommandDb && b.commandedWatts===frame.commandedWatts
      && b.sourcePin===frame.sourcePin && b.toneHz.every((n,i)=>n===frame.toneHz[i]) && b.window===frame.window && b.fftSize===frame.fftSize;
  }
}

function productText(p: RfProduct,offset=0,reference:RfReference='tone'): string {
  return p.dbc!==null || p.upperBoundDbc!==null ? productLabel(p,offset,reference) : `unavailable: ${p.reason}`;
}
export function rfMeasurementText(r: RfSpectrum,reference:RfReference='tone'): string {
  const offset=referenceOffset(r,reference);
  if (!twoToneSpectrum(r)) return `${r.backend} · RF feedback · ${r.centerHz.toLocaleString()} Hz · ${r.commandedWatts} commanded W (not measured)\n`
    + `${r.stimulus === 'saturn-timing-multisine-v1' ? 'Timing Measurement — saturn-timing-multisine-v1; seed 0x53465031; 48000 Hz PCM; 1000 ms repeat.' : 'AM silent carrier.'}\n`
    + `Two-tone IMD unavailable for this stimulus. Spectrum shown in relative dBFS, not calibrated dBm.\n`
    + `Overload ${r.overload}; sample clipping ${r.clippedComponents}; continuity ${r.continuity}; ADC attenuation command ${r.attenuationCommandDb} dB.\n`
    + `${r.window}; bins ${r.binHz.toFixed(2)} Hz; ENBW ${r.rbwHz.toFixed(2)} Hz.\n`
    + `Generation ${r.generation} / config ${r.config} / TX ${r.txEpoch}; diagnostic drops ${r.diagnosticDrops}. Host ${r.sourcePin.slice(0,12)}.\n`
    + 'Requested waveform is not proof of observed RF content. Review raw paired samples offline for signed delay and ambiguity; no delay is applied here. Freshness bounds Bridge ingress, not FPGA/RF acquisition age.';
  return `${r.backend} · RF feedback · ${r.centerHz.toLocaleString()} Hz · ${r.commandedWatts} commanded W (not measured)\n`
    + (r.stimulus === 'wdsp-postgen-two-tone-v1' ? 'Stimulus: balanced WDSP gen1 before IQC; tagged zero-input keepalive (not PCM tone audio).\n' : '')
    + `${r.reason}. Overload ${r.overload}; sample clipping ${r.clippedComponents}; continuity ${r.continuity}.\n`
    + `ADC attenuation command ${r.attenuationCommandDb} dB; tone imbalance ${r.toneBalanceDb.toFixed(1)} dB.\n`
    + `RF fundamentals ${r.toneDbfs.map(n=>n.toFixed(1)).join(' / ')} dBFS.\n`
    + `Reference-channel tones ${r.referenceToneDbfs.map(n=>n.toFixed(1)).join(' / ')} dBFS.\n`
    + r.products.map((p,i)=>`IMD${p.order} ${i%2===0?'lower':'upper'} (${p.offsetHz.toFixed(0)} Hz IQ offset): ${productText(p,offset,reference)}`).join('\n')
    + `\nReference: ${reference==='pep'?`calculated two-tone PEP (${offset.toFixed(2)} dB above weaker tone; not measured watts)`:r.reference}. ${r.window}; bins ${r.binHz.toFixed(2)} Hz; ENBW ${r.rbwHz.toFixed(2)} Hz.\n`
    + `Generation ${r.generation} / config ${r.config} / TX ${r.txEpoch}; diagnostic drops ${r.diagnosticDrops}. Host ${r.sourcePin.slice(0,12)}; firmware identity ${r.firmwareBuild}.\n`
    + 'Relative observation-path estimates, not calibrated RF IMD. Freshness bounds Bridge ingress, not FPGA/RF acquisition age. IQ offset sign is not yet bench-qualified as RF sideband orientation.';
}
export interface RfMonitorView { element: HTMLElement; canvas: HTMLCanvasElement; status: HTMLElement; summary: HTMLElement }
export async function copyRfMeasurements(view: Pick<RfMonitorView,'status'|'summary'>, feedback: HTMLElement, title='Saturn RF Feedback / IMD Monitor'): Promise<void> {
  // Freeze the displayed snapshot before awaiting clipboard access; subsequent live
  // updates must not change the status associated with these measurements.
  const text=`${title}\nCopied at ${new Date().toISOString()}\n${view.status.textContent ?? ''}\n\n${view.summary.textContent ?? ''}`;
  const doc=feedback.ownerDocument;
  feedback.querySelector('[data-rf-copy-fallback]')?.remove();
  feedback.textContent='Copying measurements…';
  try {
    const clipboard=doc.defaultView?.navigator.clipboard;
    if(!clipboard)throw new Error('Clipboard unavailable');
    await clipboard.writeText(text);
    feedback.textContent='Measurements copied.';
    return;
  } catch { /* Older browsers and unfocused pop-outs may require manual copy. */ }
  const previous=doc.activeElement;
  const field=doc.createElement('textarea');
  field.readOnly=true;field.value=text;field.setAttribute('data-rf-copy-fallback','');
  field.setAttribute('aria-label','RF measurements to copy');
  field.style.cssText='display:block;width:100%;height:160px;user-select:text';
  feedback.textContent='Clipboard unavailable. Press Ctrl+C (or long-press Copy) on the selected measurements.';
  feedback.append(field);field.focus();field.select();
  try {
    if(doc.execCommand('copy')) {
      field.remove();feedback.textContent='Measurements copied.';
      if(previous && 'focus' in previous)(previous as HTMLElement).focus();
    }
  } catch { /* Keep the selected, visible text available; never claim success. */ }
}

export function mountRfMonitor(root: HTMLElement, model: RfMonitorState): (connected:boolean,transmitting:boolean,now:number)=>void {
  const canvas=root.querySelector<HTMLCanvasElement>('[data-rf-canvas]')!;
  const status=root.querySelector<HTMLElement>('[data-rf-status]')!;
  const summary=root.querySelector<HTMLElement>('[data-rf-summary]')!;
  const save=root.querySelector<HTMLButtonElement>('[data-rf-save]')!;
  const exportButton=root.querySelector<HTMLButtonElement>('[data-rf-export]')!;
  const pop=root.querySelector<HTMLButtonElement>('[data-rf-popout]')!;
  let live=false, popup:Window|null=null, child:RfMonitorView|null=null;
  const main={element:root,canvas,status,summary};
  const displays=new Map<RfMonitorView,ViewDisplay>();displays.set(main,createDisplay(main,DEFAULT_PLOT));
  const copy=root.querySelector<HTMLButtonElement>('[data-rf-copy]')!;
  const copyStatus=root.querySelector<HTMLElement>('[data-rf-copy-status]')!;
  copy.addEventListener('click',()=>{void copyRfMeasurements(main,copyStatus);});
  save.addEventListener('click',()=>{if(live)model.saveBaseline(performance.now());});
  exportButton.addEventListener('click',()=>{
    if(!model.baseline)return;
    const url=URL.createObjectURL(new Blob([JSON.stringify({format:'saturn-rf-spectrum-baseline-v1',kind:'spectrum-only; raw paired samples retained separately on host when configured',frame:model.baseline},null,2)],{type:'application/json'}));
    const a=document.createElement('a');a.href=url;a.download='saturn-ps-off-spectrum-baseline.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
  });
  const panelWindow=new PanelWindow(root,'rf-imd',{width:1100,height:900});
  pop.addEventListener('click',()=>{
    popup=panelWindow.open();if(!popup){if(root.dataset.panelMode!=='expanded')status.textContent='Pop-out blocked; RF / IMD remains docked';return;}
    popup.opener=null;popup.document.title='Saturn RF Feedback / IMD Monitor';
    const body=popup.document.body;body.style.cssText='background:#07111f;color:#d8e7f7;font:14px system-ui;padding:12px';
    const style=popup.document.createElement('style');style.textContent='button{background:#173750;color:#e0eefb;border:1px solid #3b5d79;border-radius:5px;padding:6px 10px;cursor:pointer}button:hover{background:#244964}h2{font-size:20px}';popup.document.head.append(style);
    const label=popup.document.createElement('h2');label.textContent='RF Feedback / IMD Monitor — read only';
    const childCanvas=popup.document.createElement('canvas');childCanvas.style.cssText='display:block;position:relative;inset:auto;width:100%;height:clamp(280px,calc(100vh - 430px),760px)';
    const childStatus=popup.document.createElement('p'), childSummary=popup.document.createElement('pre');childSummary.style.cssText='white-space:pre-wrap;user-select:text';
    childStatus.textContent=status.textContent;childSummary.textContent=summary.textContent;
    const childCopy=popup.document.createElement('button');childCopy.type='button';childCopy.textContent='Copy measurements';childCopy.setAttribute('data-rf-copy','');
    const childCopyStatus=popup.document.createElement('div');childCopyStatus.setAttribute('role','status');childCopyStatus.setAttribute('data-rf-copy-status','');
    const childView={element:body,canvas:childCanvas,status:childStatus,summary:childSummary};
    childCopy.addEventListener('click',()=>{void copyRfMeasurements(childView,childCopyStatus);});
    body.replaceChildren(label,childStatus,childCanvas,childCopy,childCopyStatus,childSummary);
    if(child)displays.delete(child);child=childView;displays.set(childView,createDisplay(childView,displays.get(main)!.options));panelWindow.attach(popup);
  });
  return (connected,transmitting,now)=>{
    live=model.current(now,connected,transmitting);
    const r=model.frame, match=r?model.baselineMatches(r):false;
    const label=!connected?'DISCONNECTED — no live RF evidence':model.capability!=='raw-pairs-v1'?'UNAVAILABLE — host RF observer not enabled':!transmitting?'RX / held capture — not live':!live?`WAITING / STALE — ${model.note}`:'LIVE RF feedback — relative estimates, RF path unqualified';
    if(child&&popup?.closed){displays.delete(child);child=null;popup=null;}
    for(const view of [main,...(child&&popup&&!popup.closed?[child]:[])]) {
      const ui=displays.get(view)!;
      const text=(r?rfMeasurementText(r,ui.options.reference):'No real paired feedback window received. Digital TX display is not RF feedback.')
        + (model.baseline?`\nPS-off-requested baseline: ${match?'matching commanded settings; amber overlay':'settings / source-session mismatch — overlay hidden'}. Native correction state is not certified by this spectrum. LP700 power/SWR must be recorded independently.`:'');
      view.status.textContent=label+(root.dataset.panelNotice?` · ${root.dataset.panelNotice}`:'');view.summary.textContent=text;
      updateMetrics(ui,r);ui.plot.update(r,match?model.baseline:null,live);
    }
    save.disabled=!live||!r||r.psRequested||r.clippedComponents>0||!r.reason.startsWith('relative estimate');
    exportButton.disabled=model.baseline===null;
  };
}
