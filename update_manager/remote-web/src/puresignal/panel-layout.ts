/** Window layout only. No transport, settings, credentials or TX authority. */
export interface PanelGeometry { width:number; height:number; left:number; top:number }
export interface PanelScreen { width:number; height:number; left?:number; top?:number }
const panels=new Set<PanelWindow>();
const prefix='saturn.ps.panel-layout.v1:';
const finite=(v:unknown):v is number=>typeof v==='number'&&Number.isFinite(v);
export function panelGeometry(value:unknown,screen:PanelScreen,defaults={width:960,height:700}):PanelGeometry {
  const v=value&&typeof value==='object'?value as Record<string,unknown>:{};
  const sw=finite(screen.width)&&screen.width>0?screen.width:1280;
  const sh=finite(screen.height)&&screen.height>0?screen.height:800;
  const left=finite(screen.left)?screen.left:0,top=finite(screen.top)?screen.top:0;
  const clamp=(n:number,lo:number,hi:number)=>Math.round(Math.max(lo,Math.min(hi,n)));
  const width=clamp(finite(v.width)?v.width:defaults.width,Math.min(320,sw),sw);
  const height=clamp(finite(v.height)?v.height:defaults.height,Math.min(240,sh),sh);
  return {width,height,left:clamp(finite(v.left)?v.left:left,left,left+sw-width),
    top:clamp(finite(v.top)?v.top:top,top,top+sh-height)};
}
export class PanelWindow {
  private popup:Window|null=null;
  private timer:number|null=null;
  private lastFocus:HTMLElement|null=null;
  private lastSaved='';
  private resizeObserver:MutationObserver|null=null;
  constructor(private root:HTMLElement,private key:string,private defaults={width:960,height:700}) {
    if(!/^[a-z-]{1,40}$/.test(key))throw new Error('Invalid panel layout key');
    panels.add(this);
    this.root.style.resize='vertical';this.root.style.overflow='auto';this.root.style.minHeight='160px';
    try{
      const height=Number(localStorage.getItem(prefix+this.key+':docked'));
      if(Number.isFinite(height)&&height>=160)this.root.style.height=`${Math.min(height,Math.max(160,window.innerHeight-100))}px`;
    }catch{}
    if(typeof MutationObserver!=='undefined'){
      this.resizeObserver=new MutationObserver(()=>{
        const height=parseFloat(this.root.style.height);
        if(Number.isFinite(height)&&height>=160)try{localStorage.setItem(prefix+this.key+':docked',String(height));}catch{}
      });
      this.resizeObserver.observe(this.root,{attributes:true,attributeFilter:['style']});
    }
    window.addEventListener('pagehide',()=>this.dock(false));
  }
  open():Window|null {
    if(window.matchMedia('(max-width: 767px)').matches){
      this.root.hidden=false;this.root.style.display='';this.root.dataset.panelMode='expanded';this.root.scrollIntoView({block:'start'});return null;
    }
    if(this.popup&&!this.popup.closed){this.popup.focus();return this.popup;}
    let saved:unknown;
    try{saved=JSON.parse(localStorage.getItem(prefix+this.key)??'null');}catch{saved=null;}
    const extra=window.screen as Screen&{availLeft?:number;availTop?:number};
    const g=panelGeometry(saved,{width:extra.availWidth,height:extra.availHeight,left:extra.availLeft??0,top:extra.availTop??0},this.defaults);
    const popup=window.open('','_blank',`popup,width=${g.width},height=${g.height},left=${g.left},top=${g.top}`);
    if(!popup){this.root.hidden=false;this.root.style.display='';this.root.dataset.panelMode='docked';this.root.dataset.panelNotice='Pop-out blocked; panel remains docked. Allow pop-ups to detach it.';return null;}
    delete this.root.dataset.panelNotice;
    this.lastFocus=document.activeElement instanceof HTMLElement?document.activeElement:null;
    popup.opener=null;this.popup=popup;
    return popup;
  }
  attach(popup:Window):void {
    if(this.popup!==popup||popup.closed)return;
    const dock=popup.document.createElement('button');dock.type='button';dock.textContent='Dock';dock.dataset.psDock=this.key;
    dock.addEventListener('click',()=>this.dock());popup.document.body.prepend(dock);
    this.root.hidden=true;this.root.style.display='none';this.root.dataset.panelMode='detached';
    const save=()=>{
      if(popup.closed)return;
      const value=JSON.stringify({width:popup.innerWidth,height:popup.innerHeight,left:popup.screenX,top:popup.screenY,mode:'detached'});
      if(value===this.lastSaved)return;this.lastSaved=value;
      try{localStorage.setItem(prefix+this.key,value);}catch{/* Layout storage is optional. */}
    };
    save();popup.addEventListener('resize',save);popup.addEventListener('pagehide',()=>this.dock());
    if(this.timer!==null)window.clearInterval(this.timer);
    this.timer=window.setInterval(()=>{if(popup.closed)this.dock();else save();},500);
  }
  dock(focus=true):void {
    const popup=this.popup;this.popup=null;
    if(this.timer!==null){window.clearInterval(this.timer);this.timer=null;}
    this.root.hidden=false;this.root.style.display='';this.root.dataset.panelMode='docked';
    if(popup&&!popup.closed)popup.close();
    if(focus&&this.lastFocus?.isConnected)this.lastFocus.focus();
  }
  reset():void {this.dock(false);this.lastSaved='';this.root.style.height='';delete this.root.dataset.panelNotice;
    try{localStorage.removeItem(prefix+this.key);localStorage.removeItem(prefix+this.key+':docked');}catch{}}
}
export function resetPanelLayouts():void {for(const panel of panels)panel.reset();}
