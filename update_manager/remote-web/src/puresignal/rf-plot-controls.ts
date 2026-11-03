/** Read-only per-view presentation controls; never sends radio commands. */
import type {RfSpectrum,RfMonitorView} from './rf-monitor';
import {RfPlot,isTwoTone,referenceOffset,productLabel,RF_COLORS,productColor,type PlotOptions} from './rf-plot';
export interface ViewDisplay {options:PlotOptions;plot:RfPlot;metrics:HTMLElement;details:HTMLElement;reference:HTMLSelectElement}
export function createDisplay(view:RfMonitorView,defaults:PlotOptions):ViewDisplay {
  const doc=view.element.ownerDocument,options={...defaults};view.canvas.dataset.rfCanvas='';
  const toolbar=doc.createElement('div');toolbar.dataset.rfDisplayControls='';
  toolbar.style.cssText='display:flex;flex-wrap:wrap;gap:10px 18px;align-items:center;margin:12px 0;color:#b9cde0;font:12px system-ui';
  const select=(label:string,key:keyof PlotOptions,values:[string,string][]):HTMLSelectElement=>{
    const wrapper=doc.createElement('label');wrapper.textContent=label+' ';
    const field=doc.createElement('select');field.dataset.rfPlotControl=key;field.setAttribute('aria-label',label);
    field.style.cssText='background:#11263b;color:#e0eefb;border:1px solid #39536d;border-radius:5px;padding:5px';
    for(const [value,text] of values){const item=doc.createElement('option');item.value=value;item.textContent=text;field.append(item);}
    field.value=String(options[key]);field.addEventListener('change',()=>{
      if(key==='reference')options.reference=field.value==='pep'?'pep':'tone';
      else if(key==='spanHz')options.spanHz=Number(field.value);else options.rangeDb=Number(field.value);
    });wrapper.append(field);toolbar.append(wrapper);return field;
  };
  const reference=select('Reference','reference',[['tone','Weaker tone (dBc)'],['pep','Two-tone PEP (calculated)']]);
  select('Span','spanHz',[['16000','16 kHz'],['10000','10 kHz'],['6000','6 kHz'],['3000','3 kHz']]);
  select('Display range','rangeDb',[['80','80 dB'],['100','100 dB'],['120','120 dB'],['140','140 dB'],['160','160 dB']]);
  view.canvas.before(toolbar);
  const record=doc.createElement('details');record.dataset.rfDetailedRecord='';record.style.cssText='margin:12px 0;font-size:12px';
  const recordTitle=doc.createElement('summary');recordTitle.textContent='Full measurement record';recordTitle.style.cursor='pointer';
  view.summary.before(record);record.append(recordTitle,view.summary);
  const cursor=doc.createElement('div');cursor.dataset.rfCursor='';cursor.style.cssText='color:#a5bdd3;font:11px ui-monospace,monospace;margin:7px 0;min-height:28px';cursor.textContent='Move over the trace for frequency and FFT-bin level.';
  const metrics=doc.createElement('div');metrics.dataset.rfMetrics='';metrics.style.cssText='font:12px system-ui;margin:10px 0;color:#c9dced';
  const details=doc.createElement('div');details.dataset.rfResolution='';details.style.cssText='font:11px system-ui;color:#9fb5ca;margin:8px 0;line-height:1.6';
  view.canvas.after(cursor,metrics,details);
  return {options,plot:new RfPlot(view.canvas,()=>options,cursor),metrics,details,reference};
}
export function updateMetrics(ui:ViewDisplay,r:RfSpectrum|null):void {
  const doc=ui.metrics.ownerDocument;ui.reference.disabled=!r||!isTwoTone(r);
  if(!r){ui.metrics.textContent='No RF capture';ui.details.textContent='';return;}
  const offset=referenceOffset(r,ui.options.reference);
  const head=doc.createElement('div');head.style.cssText='display:flex;flex-wrap:wrap;gap:8px 20px;margin-bottom:8px';
  for(const text of [`RF observation: uncalibrated`,`Overload: ${r.overload}`,`Sample clips: ${r.clippedComponents}`,`Continuity: ${r.continuity}`]){
    const badge=doc.createElement('span');badge.textContent=text;badge.style.cssText='border:1px solid #3b4b60;border-radius:4px;padding:3px 6px;color:#e3bf81';head.append(badge);
  }
  const legend=doc.createElement('div');legend.style.cssText='display:flex;flex-wrap:wrap;gap:8px 18px;margin:10px 0;font-size:12px';
  for(const [label,color] of [['RF trace',RF_COLORS.trace],...(isTwoTone(r)?[['Fundamentals',RF_COLORS.fundamental],['IMD3',RF_COLORS.imd3],['IMD5',RF_COLORS.imd5],['IMD7',RF_COLORS.imd7]]:[]),['Held baseline (dashed)',RF_COLORS.baseline]]){
    const item=doc.createElement('span');item.style.color=color!;item.textContent='━ '+label;legend.append(item);
  }
  const table=doc.createElement('table');table.dataset.rfProductTable='';table.style.cssText='width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums';
  if(isTwoTone(r)){
    const header=table.createTHead().insertRow();for(const title of ['Integrated IMD','Low IQ offset','High IQ offset']){const cell=doc.createElement('th');cell.textContent=title;cell.scope='col';cell.style.cssText='text-align:left;color:#9fb5ca;padding:5px;border-bottom:1px solid #31475d';header.append(cell);}
    for(let i=0;i<6;i+=2){const row=table.insertRow();for(const [j,text] of [`IMD${r.products[i]!.order}`,productLabel(r.products[i]!,offset,ui.options.reference),productLabel(r.products[i+1]!,offset,ui.options.reference)].entries()){
      const cell=row.insertCell();cell.textContent=text.split(' · ')[0]!;cell.style.cssText='padding:6px 5px;border-bottom:1px solid #1c3044;font:13px ui-monospace,monospace';if(j)cell.title=r.products[i+j-1]!.reason;else cell.style.color=productColor(r.products[i]!.order);
    }}
  }else {const row=table.insertRow();row.insertCell().textContent='Two-tone IMD unavailable for this stimulus.';}
  ui.metrics.replaceChildren(legend,head,table);
  ui.details.textContent=`${r.window} · FFT ${r.fftSize.toLocaleString()} · bin spacing ${r.binHz.toFixed(2)} Hz · ENBW ${r.rbwHz.toFixed(2)} Hz · ${(1000*r.fftSize/r.rateHz).toFixed(1)} ms nominal sample window. `
    +'Blue/grey is current/held RF; green/orange/pink/violet identify fundamentals/IMD3/5/7, including held captures. Amber is the PS-off-requested baseline. Trace shows FFT-bin levels; the table integrates product bands. A < value is an estimated floor bound, not a resolved product. F1/F2 are fundamentals; L/H denote IQ offset, not verified RF sideband orientation. Narrow spans may hide markers. '
    +(ui.options.reference==='pep'&&isTwoTone(r)?`PEP reference is calculated from fundamentals, ${offset.toFixed(2)} dB above the weaker tone; no change in measured RF. `:'')
    +'Clocks bound Bridge-ingress age; RF acquisition age and observation-path calibration remain unverified.';
}
