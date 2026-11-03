/** Presentation only: no sample processing, transport, settings or TX authority. */
import type {RfSpectrum,RfProduct} from './rf-monitor';
export type RfReference='tone'|'pep';
export interface PlotOptions {reference:RfReference;spanHz:number;rangeDb:number}
export const DEFAULT_PLOT:PlotOptions={reference:'tone',spanHz:16000,rangeDb:140};
export const RF_COLORS={trace:'#55c7ff',fundamental:'#73efb2',imd3:'#ffad69',imd5:'#ed91db',imd7:'#a69bff',baseline:'#f5be55'};
export function productColor(order:number):string {
  return order===3?RF_COLORS.imd3:order===5?RF_COLORS.imd5:RF_COLORS.imd7;
}
export function isTwoTone(r:RfSpectrum):boolean {
  return r.stimulus!=='saturn-timing-multisine-v1'&&r.stimulus!=='am-silent-carrier';
}
/** Two-tone fundamental-derived PEP, not measured watts or an RF calibration. */
export function referenceOffset(r:RfSpectrum,reference:RfReference):number {
  if(reference==='tone'||!isTwoTone(r))return 0;
  const weak=Math.min(...r.toneDbfs),strong=Math.max(...r.toneDbfs);
  return strong-weak+20*Math.log10(1+10**((weak-strong)/20));
}
export function productLabel(p:RfProduct,offset=0,reference:RfReference='tone'):string {
  const units=reference==='pep'?'dB re PEP':'dBc';
  return p.dbc!==null?`${(p.dbc-offset).toFixed(1)} ${units}`
    :p.upperBoundDbc!==null?`< ${(p.upperBoundDbc-offset).toFixed(1)} ${units} · floor estimate`:'Unavailable';
}
export function plotLimits(r:RfSpectrum|null,options:PlotOptions):{lowHz:number;highHz:number;topDb:number;bottomDb:number} {
  const tones=!!r&&isTwoTone(r),center=tones&&options.spanHz<16000?(r.toneHz[0]!+r.toneHz[1]!)/2:0;
  const topDb=tones?Math.max(5,Math.ceil((r.toneBalanceDb-referenceOffset(r,options.reference)-5)/10)*10+5):5;
  return {lowHz:center-options.spanHz/2,highHz:center+options.spanHz/2,topDb,bottomDb:topDb-options.rangeDb};
}
export function canvasScale(width:number,height:number,dpr:number):number {
  return Math.min(Math.max(.5,Number.isFinite(dpr)?dpr:1),3,Math.sqrt(12_000_000/(width*height)));
}
function ticks(span:number,pixels:number):number {
  const wanted=span/Math.max(2,pixels/70),power=10**Math.floor(Math.log10(wanted));
  return [1,2,5,10].find(n=>n*power>=wanted)!*power;
}
interface Cursor {hz:number}
export class RfPlot {
  private frame:RfSpectrum|null=null;
  private baseline:RfSpectrum|null=null;
  private live=false;
  private cursor:Cursor|null=null;
  private last='';
  constructor(readonly canvas:HTMLCanvasElement,private options:()=>PlotOptions,private readout:HTMLElement) {
    canvas.style.display='block';canvas.style.position='relative';canvas.style.inset='auto';
    canvas.addEventListener('pointermove',e=>{
      const rect=canvas.getBoundingClientRect(),width=rect.width,left=54,right=width-16;
      const {lowHz,highHz}=plotLimits(this.frame,this.options());
      const x=e.clientX-rect.left;
      this.cursor=x>=left&&x<=right?{hz:lowHz+(x-left)/(right-left)*(highHz-lowHz)}:null;
      if(!this.cursor)this.readout.textContent='Move over the trace for frequency and FFT-bin level.';
      this.last='';this.paint();
    });
    canvas.addEventListener('pointerleave',()=>{this.cursor=null;this.readout.textContent='Move over the trace for frequency and FFT-bin level.';this.last='';this.paint();});
  }
  update(frame:RfSpectrum|null,baseline:RfSpectrum|null,live:boolean):void {
    this.frame=frame;this.baseline=baseline;this.live=live;if(!frame)this.readout.textContent='No RF capture available.';this.paint();
  }
  private paint():void {
    const rect=this.canvas.getBoundingClientRect();if(rect.width<2||rect.height<2)return;
    const width=Math.round(rect.width),height=Math.round(rect.height),doc=this.canvas.ownerDocument;
    const scale=canvasScale(width,height,doc.defaultView?.devicePixelRatio??1),o=this.options(),r=this.frame;
    const key=[r?.boot,r?.serial,this.baseline?.boot,this.baseline?.serial,this.live,width,height,scale,o.reference,o.spanHz,o.rangeDb,this.cursor?.hz].join(':');
    if(key===this.last)return;this.last=key;
    this.canvas.width=Math.round(width*scale);this.canvas.height=Math.round(height*scale);
    const c=this.canvas.getContext('2d');if(!c)return;c.setTransform(scale,0,0,scale,0,0);
    const bg=c.createLinearGradient(0,0,0,height);bg.addColorStop(0,'#0c1c30');bg.addColorStop(1,'#050b14');c.fillStyle=bg;c.fillRect(0,0,width,height);
    const left=54,right=width-16,top=40,bottom=height-42,{lowHz,highHz,topDb,bottomDb}=plotLimits(r,o);
    const x=(hz:number)=>left+(hz-lowHz)/(highHz-lowHz)*(right-left),y=(db:number)=>top+(topDb-db)/(topDb-bottomDb)*(bottom-top);
    c.font='11px ui-monospace,monospace';c.lineWidth=.6;
    for(let db=Math.ceil(bottomDb/5)*5;db<=topDb;db+=5){
      c.strokeStyle=db%10===0?'#29445e':'#14293c';c.beginPath();c.moveTo(left,y(db));c.lineTo(right,y(db));c.stroke();
      if(db%10===0){c.fillStyle='#9fb4c9';c.textAlign='right';c.fillText(String(db),left-9,y(db)+4);}
    }
    const step=ticks(highHz-lowHz,right-left);
    for(let hz=Math.ceil(lowHz/(step/5))*(step/5);hz<=highHz;hz+=step/5){
      const major=Math.abs(hz/step-Math.round(hz/step))<1e-6;c.strokeStyle=major?'#29445e':'#14293c';c.beginPath();c.moveTo(x(hz),top);c.lineTo(x(hz),bottom);c.stroke();
      if(major){c.fillStyle='#9fb4c9';c.textAlign='center';c.fillText(`${+(hz/1000).toFixed(2)}`,x(hz),bottom+18);}
    }
    c.strokeStyle='#49627b';c.strokeRect(left,top,right-left,bottom-top);
    c.textAlign='left';c.fillStyle='#cfe2f4';c.font='12px system-ui';
    c.fillText(r?`${r.backend} · ${r.centerHz.toLocaleString()} Hz`:'Returned RF spectrum',left,17);
    c.textAlign='right';c.fillStyle=this.live?'#69dcba':'#a4b2c1';c.fillText(this.live?'LIVE':r?'HELD / NOT LIVE':'NO CAPTURE',right,17);
    c.textAlign='left';c.font='11px ui-monospace,monospace';c.fillStyle='#9fb4c9';
    const units=r&&!isTwoTone(r)?'relative dBFS':o.reference==='pep'?'dB re calculated two-tone PEP':'dBc · weaker fundamental';
    c.fillText(units,left,height-9);c.textAlign='right';c.fillText('IQ offset (kHz)',right,height-9);
    c.save();c.beginPath();c.rect(left,top,right-left,bottom-top);c.clip();
    const trace=(s:RfSpectrum,color:string,dashed=false,fill=false)=>{
      const ref=isTwoTone(s)?Math.min(...s.toneDbfs)+referenceOffset(s,o.reference):0;
      c.strokeStyle=color;c.lineWidth=dashed?1.2:1.6;c.setLineDash(dashed?[5,4]:[]);c.beginPath();
      // Every received bin is drawn; no numerical smoothing changes measurements.
      s.binsDbfs.forEach((db,i)=>{const px=x(s.firstBinHz+i*s.binHz),py=y(db-ref);if(i)c.lineTo(px,py);else c.moveTo(px,py);});
      if(fill){
        c.save();const wash=c.createLinearGradient(0,top,0,bottom);wash.addColorStop(0,'#55c7ff38');wash.addColorStop(1,'#55c7ff06');
        c.lineTo(x(s.firstBinHz+(s.binsDbfs.length-1)*s.binHz),bottom);c.lineTo(x(s.firstBinHz),bottom);c.closePath();c.fillStyle=wash;c.fill();c.restore();
        c.beginPath();s.binsDbfs.forEach((db,i)=>{const px=x(s.firstBinHz+i*s.binHz),py=y(db-ref);if(i)c.lineTo(px,py);else c.moveTo(px,py);});
      }
      c.stroke();c.setLineDash([]);
    };
    if(r)trace(r,this.live?RF_COLORS.trace:'#95a8bd',false,true);
    if(this.baseline)trace(this.baseline,RF_COLORS.baseline,true);
    if(r&&isTwoTone(r)){
      const markers=[...r.toneHz.map((hz,i)=>({hz,label:`F${i+1}`,color:RF_COLORS.fundamental,resolved:r.reason.startsWith('relative estimate')})),...r.products.map((p,i)=>({hz:p.offsetHz,label:`${p.order}${i%2?'H':'L'}`,color:productColor(p.order),resolved:p.dbc!==null}))];
      // Color only the actual retained bins in a resolved line's integration
      // band. Expected guides never manufacture a peak or resolve a floor bound.
      const ref=Math.min(...r.toneDbfs)+referenceOffset(r,o.reference);
      for(const m of markers){if(!m.resolved||m.hz<lowHz||m.hz>highHz)continue;
        const center=Math.round((m.hz-r.firstBinHz)/r.binHz),first=Math.max(0,center-5),last=Math.min(r.binsDbfs.length-1,center+5);
        c.strokeStyle=m.color;c.lineWidth=2;c.beginPath();
        for(let i=first;i<=last;i++){const px=x(r.firstBinHz+i*r.binHz),py=y(r.binsDbfs[i]!-ref);if(i===first)c.moveTo(px,py);else c.lineTo(px,py);}c.stroke();
      }
      markers.forEach((m,i)=>{if(m.hz<lowHz||m.hz>highHz)return;
        c.strokeStyle=m.color;c.globalAlpha=.4;c.lineWidth=.8;c.setLineDash([3,5]);c.beginPath();c.moveTo(x(m.hz),top);c.lineTo(x(m.hz),bottom);c.stroke();c.setLineDash([]);c.globalAlpha=1;
        c.textAlign='center';c.font='11px ui-monospace,monospace';const ty=top+13+(i%3)*15;c.fillStyle='#081522';c.fillRect(x(m.hz)-14,ty-10,28,13);c.fillStyle=m.color;c.fillText(m.label,x(m.hz),ty);
      });
    }
    if(r&&this.cursor){
      const index=Math.round((this.cursor.hz-r.firstBinHz)/r.binHz),db=r.binsDbfs[index];
      if(db!==undefined){
        const hz=r.firstBinHz+index*r.binHz,ref=isTwoTone(r)?Math.min(...r.toneDbfs)+referenceOffset(r,o.reference):0;
        c.strokeStyle='#ecf4fa';c.globalAlpha=.7;c.setLineDash([2,3]);c.beginPath();c.moveTo(x(hz),top);c.lineTo(x(hz),bottom);c.stroke();c.setLineDash([]);c.globalAlpha=1;
        c.fillStyle='#edf7ff';c.beginPath();c.arc(x(hz),y(db-ref),3,0,Math.PI*2);c.fill();
        this.readout.textContent=`IQ ${hz>=0?'+':''}${hz.toFixed(1)} Hz · ${(db-ref).toFixed(1)} ${isTwoTone(r)?o.reference==='pep'?'dB re PEP':'dBc':'dBFS'} per FFT bin · IMD table uses integrated bands`;
      }else this.readout.textContent='No FFT bin at this offset.';
    }
    c.restore();
  }
}
