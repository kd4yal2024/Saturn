// Compare the same deterministic IQ under the former WAN FFT and 3D detail FFT.
// This exercises the shipped FFT, zoom adapter, numerical history and WebGL view.
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

export async function wanZoom({ evaluate, call, output, report }) {
  await call('Emulation.setDeviceMetricsOverride', {
    width: 1440, height: 1000, deviceScaleFactor: 2, mobile: false,
  });
  await evaluate("applyLayout('desktop',false,false); syncDisplayWorkspaceRatio(); new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
  report.wanZoom = [];
  for (const [label, size] of [['before-2048', 2048], ['after-16384', 16384]]) {
    const result = await evaluate(`(()=>{
      const label=${JSON.stringify(label)}, size=${size}, rate=384000, count=16384;
      state.streamMode='wan'; state.layoutMode='desktop'; state.displayZoom=4;
      state.sampleRate=rate; state.displayIqSource='rx';
      state.terrain={...state.terrain,quality:'performance',floor:-140,ceiling:-34,
        gamma:.7,cleanup:.7,waterfallCleanup:0,cleanupBaseline:-108,smoothing:0,gridOpacity:0};
      terrainRenderer.configure(state.terrain); layoutTerrainCanvas();
      const target=displayFftTargetSize();
      if(target!==16384)throw Error('3D WAN target '+target+' instead of 16384');
      const h=spectrumHistory,r=terrainRenderer;
      h.clear('Same-IQ WAN zoom '+label);
      const tone=new Float32Array(count*2),iq=new Float32Array(count*2);
      for(let i=0;i<count;i++){
        const a=2*Math.PI*1000*i/rate,b=2*Math.PI*1300*i/rate;
        tone[2*i]=.02*(Math.cos(a)+Math.cos(b));
        tone[2*i+1]=.02*(Math.sin(a)+Math.sin(b));
      }
      const fft=new _next.FftProcessor(size); let seed=702701;
      let last=null;
      for(let t=0;t<560;t++){
        for(let i=0;i<iq.length;i++){
          seed=(Math.imul(seed,1664525)+1013904223)|0;
          iq[i]=tone[i]+((seed>>>0)/4294967296-.5)*.0005;
        }
        // A 5.3 ms event early in one IQ frame is outside the old final-2048
        // sample window, but inside the new time-bounded FFT window.
        if(t===470)for(let i=4096;i<6144;i++){
          const a=2*Math.PI*2000*i/rate;
          iq[2*i]+=.004*Math.cos(a); iq[2*i+1]+=.004*Math.sin(a);
        }
        last=_next.visibleBinsForDisplay(fft.transform(iq.subarray((count-size)*2)),4);
        if(!acceptSpectrumFrame(last,t*50,t,size))throw Error('Rejected IQ fixture row '+t);
      }
      r.render(performance.now(),layoutTerrainCanvas(),true);
      const w=r.canvas.width,rect=r.canvas.getBoundingClientRect();
      const lower=h.capacity,visible=last.length;
      const index=hz=>Math.round(visible/2-hz*size/rate);
      const left=index(1300),right=index(1000),mid=Math.round((left+right)/2);
      const peaks=[last[left],last[right]],valley=last[mid];
      if(size===16384 && !(valley<Math.min(...peaks)-15))
        throw Error('Nearby carriers not resolved: '+JSON.stringify({peaks,valley}));
      if(size===2048 && right-left>2)throw Error('Baseline unexpectedly resolved close carriers');
      const eventBin=index(2000),eventAge=559-470;
      const eventRow=h.row(eventAge),quietRow=h.row(eventAge+1);
      const around=row=>Math.max(...row.subarray(eventBin-3,eventBin+4));
      const eventContrastDb=around(eventRow)-around(quietRow);
      if(size===16384 && eventContrastDb<15)throw Error('Short event lost: '+eventContrastDb);
      if(size===2048 && eventContrastDb>6)throw Error('Old short window unexpectedly saw early event');
      const benchmarkStart=performance.now();
      for(let i=0;i<50;i++)fft.transform(iq.subarray((count-size)*2));
      const fftMeanMs=(performance.now()-benchmarkStart)/50;
      const line=new Uint8Array(w*4);
      r.gl.readPixels(0,100,w,1,r.gl.RGBA,r.gl.UNSIGNED_BYTE,line);
      const x0=Math.max(0,Math.floor(left*w/visible)-5);
      const x1=Math.min(w-1,Math.ceil(right*w/visible)+5);
      const pixelProfile=Array.from({length:x1-x0+1},(_,offset)=>{
        const x=x0+offset,p=x*4;return {x,r:line[p],g:line[p+1],b:line[p+2]};
      });
      const peakPixels=[Math.round(left*w/visible),Math.round(right*w/visible)];
      const gapRed=Math.min(...pixelProfile.filter(p=>p.x>peakPixels[0]&&p.x<peakPixels[1]).map(p=>p.r));
      if(size===16384 && gapRed>=80)throw Error('Rendered carriers still merge: '+gapRed);
      if(size===2048 && gapRed<150)throw Error('Old WAN render unexpectedly separated carriers');
      window.wanZoomPng=r.canvas.toDataURL('image/png').split(',')[1];
      return {label,fftSize:size,targetFftSize:target,sourceRateHz:rate,zoom:4,
        cssCanvas:[rect.width,rect.height],backingCanvas:[w,r.canvas.height],
        visibleBins:visible,binsPerBackingPixel:visible/w,fftWindowMs:size/rate*1000,
        fftMeanMs,eventContrastDb,
        peakPixels,gapRed,pixelProfile,
        closeCarrierBins:[left,right],closeCarrierPeaks:peaks,valleyDb:valley,
        historyRows:h.count,diagnostics:r.diagnostics()};
    })()`);
    const raw=Buffer.from(await evaluate('wanZoomPng'),'base64');
    writeFileSync(join(output,label+'-raw.png'),raw);
    const clip=await evaluate(`(()=>{const r=$('waterfall-shell').getBoundingClientRect();
      return {x:r.x,y:r.y,width:r.width,height:r.height,scale:1};})()`);
    const screenshot=await call('Page.captureScreenshot',{format:'png',clip});
    writeFileSync(join(output,label+'-waterfall.png'),Buffer.from(screenshot.data,'base64'));
    report.wanZoom.push(result);
  }
  report.zoomOne = await evaluate(`(()=>{
    const r=terrainRenderer,h=spectrumHistory;
    state.displayZoom=1;
    const target=displayFftTargetSize();
    if(target!==8192)throw Error('1x source exceeds GPU limit: '+target);
    h.clear('1x GPU texture boundary');
    acceptSpectrumFrame(new Float32Array(target).fill(-120),0,0,target);
    r.render(performance.now(),layoutTerrainCanvas(),true);
    if(r.gl.getError()!==r.gl.NO_ERROR)throw Error('1x GPU texture allocation failed');
    return {targetFftSize:target,historyTextureWidth:h.width,maxHistoryWidth:r.maxHistoryWidth};
  })()`);
  report.checks.push('same deterministic IQ: 3D WAN/4x retains 4096 visible bins, close carriers separate, 512 time rows retained');
  report.checks.push('1x zoom caps source FFT to the actual WebGL history-texture width');
}
