/** Read-only presentation of the same generation/freshness-checked PS tab data. */
import {PanelWindow} from './panel-layout';
import type {pureSignalTestView} from './test-view';
export function psHealthText(view:ReturnType<typeof pureSignalTestView>,connected:boolean):string {
  return `${connected?'Connected — host reports':'DISCONNECTED — no live PS status'}\n`
    +`${view.owner}\nSource: ${view.source}\n${view.committedMode}\n`
    +`Native state: ${view.state}\nNative feedback level: ${view.feedback}\nCalibrations: ${view.calibrations}\nCorrection: ${view.correcting}\n`
    +`Feedback packets: ${view.packets}; gaps: ${view.gaps}\n${view.timing}\n`
    +'ADC/DDC overload coverage: unknown unless separately supplied for the captured interval.\n'
    +'Native feedback level is dimensionless, not percent, dBm or watts. Native flags are not RF improvement measurements.\n'
    +view.qualification;
}
export function mountPsHealthMonitor(root:HTMLElement):(connected:boolean,view:ReturnType<typeof pureSignalTestView>)=>void {
  const output=root.querySelector<HTMLElement>('[data-ps-health-summary]')!;
  const overview=root.querySelector<HTMLElement>('[data-ps-health-overview]');
  const button=root.querySelector<HTMLButtonElement>('[data-ps-health-popout]')!;
  const layout=new PanelWindow(root,'ps-monitor',{width:750,height:650});
  let popup:Window|null=null,child:HTMLElement|null=null;
  button.addEventListener('click',()=>{
    popup=layout.open();if(!popup)return;
    popup.document.title='Saturn PS Monitor';
    popup.document.body.style.cssText='background:#07111f;color:#d8e7f7;font:14px system-ui;padding:12px';
    const heading=popup.document.createElement('h2');heading.textContent='PureSignal Monitor — read only';
    child=popup.document.createElement('pre');child.style.cssText='white-space:pre-wrap;user-select:text';child.textContent=output.textContent;
    popup.document.body.replaceChildren(heading,child);layout.attach(popup);
  });
  return (connected,view)=>{
    output.textContent=psHealthText(view,connected)+(root.dataset.panelNotice?`\n${root.dataset.panelNotice}`:'');
    if(overview)overview.textContent=connected?`${view.correcting} · Fits: ${view.calibrations}\n${view.owner}`:'Disconnected';
    if(popup&&!popup.closed&&child)child.textContent=output.textContent;
  };
}
