// Deterministic comparisons through the real template adapter; no radio sockets.
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
export async function controlled({evaluate, call, output, report, phase}) {
  report.controlled=[];
  await evaluate(`(()=>{
    state.terrain={...state.terrain,floor:-140,ceiling:-40,gamma:1,height:.65,smoothing:0,quality:'balanced',depth:128};
    window.controlledStyle=document.createElement('style');
    controlledStyle.textContent='[data-terrain="true"] .display-stack .spectrum-shell > :not(canvas),[data-terrain="true"] .display-stack .waterfall-shell > :not(canvas){visibility:hidden!important}';
    document.head.appendChild(controlledStyle);
  })()`);
  for(const kind of ['constant','noise','carrier-flat','carriers','broadband','pulse']) {
    const result=await evaluate(`(()=>{
      const kind=${JSON.stringify(kind)},r=terrainRenderer,h=spectrumHistory;
      h.clear('Controlled '+kind);r.configure(state.terrain);
      const bins=new Float32Array(4096);window.controlledRecorded=[];
      for(let t=0;t<1800;t++) {
        for(let i=0;i<bins.length;i++) {
          let db=kind==='constant'?-110:kind==='carrier-flat'?-134:-134+1.5*Math.sin(i*1.99+t*.31)+.8*Math.sin(i*.17+t*.47);
          if(kind==='carriers') {if(i===738)db=-112;if(i===2170)db=-58;}
          if(kind==='carrier-flat' && i===2170)db=-58;
          if(kind==='broadband' && t>=1675 && t<=1680)db=-104;
          if(kind==='pulse' && t===1680)db=-60;
          bins[i]=db;
        }
        acceptSpectrumFrame(bins,fixtureTimestamp,fixtureIndex++,4096);fixtureTimestamp+=33;
        if(t>=1288)controlledRecorded.push({bins:bins.slice(),timestamp:fixtureTimestamp-33,sequence:fixtureIndex-1});
        if(t%128===0)r.render(performance.now(),layoutTerrainCanvas(),true);
      }
      r.render(performance.now(),layoutTerrainCanvas(),true);
      const w=r.canvas.width,height=r.canvas.height;
      const upper=Math.round($('spectrum-shell').getBoundingClientRect().height/r.canvas.getBoundingClientRect().height*height),lower=height-upper;
      const pixels=new Uint8Array(w*lower*4);r.gl.readPixels(0,0,w,lower,r.gl.RGBA,r.gl.UNSIGNED_BYTE,pixels);
      let spread=0;for(let i=0;i<pixels.length;i++)if(i%4!==3)spread=Math.max(spread,Math.abs(pixels[i]-pixels[i%4]));
      if(kind==='constant' && spread>1)throw Error('Constant waterfall contains stripes: '+spread);
      for(let age=0;age<512;age++) {
        const row=h.row(age),record=controlledRecorded[511-age];
        if(!row || row.some((v,i)=>v!==record.bins[i]) || h.rows[h.physical(age)].timestamp!==record.timestamp)throw Error('Ring chronology differs from recorded frames at '+age);
      }
      if(kind==='broadband')for(let age=0;age<512;age++) {
        const elevated=h.row(age)[100]>-110;
        if(elevated!==(age>=119&&age<=124))throw Error('Broadband increase has incorrect measured duration');
      }
      if(kind==='carrier-flat')for(let age=0;age<512;age++) {
        if(h.row(age)[2170]!==-58 || h.row(age)[100]!==-134)throw Error('Stationary carrier acquired time variation');
      }
      const front=r.project(0,Math.floor(r.columns/2));const fence=new Uint8Array(4);
      r.gl.readPixels(Math.floor(w/2),lower+Math.max(1,Math.floor((1-front[1])*upper*.5)),1,1,r.gl.RGBA,r.gl.UNSIGNED_BYTE,fence);
      if(${JSON.stringify(phase)}==='after' && kind==='constant' && fence[2]>15)throw Error('Opaque front fence remains');
      let checkedPixels=0;
      for(let y=0;y<lower;y++)for(const x of [0,Math.floor(w*.18),Math.floor(w*.53),w-1]) {
        const ageLo=Math.max(0,Math.floor((1-(y+1)/lower)*512)),ageHi=Math.min(512,Math.max(ageLo+1,Math.ceil((1-y/lower)*512)));
        const lo=Math.floor(x*4096/w),hi=Math.max(lo+1,Math.floor((x+1)*4096/w));let db=-1000;
        for(let age=ageLo;age<ageHi;age++)for(let bin=lo;bin<hi;bin++)db=Math.max(db,h.row(age)[bin]);
        const expected=terrainColor((db+140)/100,'reference'),offset=(y*w+x)*4;
        if(expected.some((v,c)=>Math.abs(v-pixels[offset+c])>3))throw Error('Waterfall ring/pixel mismatch '+kind+' at '+x+','+y);
        checkedPixels++;
      }
      if(kind==='constant') {
        const expected=terrainColor(.3,'reference'),pixel=new Uint8Array(4);
        for(const age of [4,20,60,110])for(const fraction of [.1,.5,.9]) {
          const projected=r.project(age,Math.floor(fraction*(r.columns-1)));
          r.gl.readPixels(Math.round(projected[0]*(w-1)),lower+Math.floor((1-projected[1])*upper),1,1,r.gl.RGBA,r.gl.UNSIGNED_BYTE,pixel);
          if(expected.some((v,c)=>Math.abs(v-pixel[c])>3))throw Error('Constant surface has color terraces/depth tint');
        }
      }
      const bright=[];if(kind==='pulse')for(let y=0;y<lower;y++)if(pixels[(y*w+Math.floor(w/2))*4]>200)bright.push(y);
      if(kind==='pulse' && (bright.length<1||bright.length>3||bright.some((y,i)=>i&&y!==bright[i-1]+1)))throw Error('Pulse is lost/duplicated: '+bright);
      const raw=Array.from(h.latestRaw).sort((a,b)=>a-b),median=raw[2048];
      window.controlledPng=r.canvas.toDataURL('image/png').split(',')[1];
      return {kind,frames:1800,wraps:Math.floor(1800/512),head:h.head,waterfallSpread:spread,checkedPixels,frontFencePixel:Array.from(fence),pulsePixelRows:bright,
        levels:{min:raw[0],median,p95:raw[Math.floor(raw.length*.95)],max:raw[raw.length-1],floor:-140,ceiling:-40,normalizedMedian:(median+140)/100},diagnostics:r.diagnostics()};
    })()`);
    writeFileSync(join(output,kind+'-raw.png'),Buffer.from(await evaluate('controlledPng'),'base64'));
    const shot=await call('Page.captureScreenshot',{format:'png'});writeFileSync(join(output,kind+'-overlays-off.png'),Buffer.from(shot.data,'base64'));
    if(phase==='after') {
      const comparison=await evaluate(`(()=>{
        const r=terrainRenderer,h=spectrumHistory,revision=h.revision;
        const before=JSON.stringify([r.project(0,Math.floor(r.columns/2)),r.project(100,Math.floor(r.columns/2))]);
        r.configure({...state.terrain,palette:'reference-dark'});r.render(performance.now(),layoutTerrainCanvas(),true);
        window.controlledDarkPng=r.canvas.toDataURL('image/png').split(',')[1];
        if(before!==JSON.stringify([r.project(0,Math.floor(r.columns/2)),r.project(100,Math.floor(r.columns/2))])||revision!==h.revision)throw Error('Palette changed geometry or history');
        r.configure(state.terrain);r.render(performance.now(),layoutTerrainCanvas(),true);
        return {geometryUnchanged:true,historyUnchanged:true};
      })()`);
      writeFileSync(join(output,kind+'-dark-raw.png'),Buffer.from(await evaluate('controlledDarkPng'),'base64'));
      result.paletteIsolation=comparison;
    }
    report.controlled.push(result);
  }
  // Identical CPU frames fed to the Traditional waterfall with identical palette/range.
  report.recordedComparison=await evaluate(`(()=>{
    const paused=state.displayPaused,revision=spectrumHistory.revision;
    setTerrainMode('traditional',false);waterfallRenderer.clear();
    for(const record of controlledRecorded)waterfallRenderer.pushLine(record.bins,-140,-40,'reference',100,0);
    waterfallRenderer.render();
    const r=waterfallRenderer,gl=r.gl,pixel=new Uint8Array(4);let checked=0;
    for(const age of [0,1,119,120,121,255,511]) {
      const value=controlledRecorded[511-age].bins[2048],expected=r.colorForDb(value,-140,-40,'reference',100);
      const physical=(r.textureHead+age)%r.textureHeight;
      for(const bin of [0,738,2048,2170,4095]) {
        const expected=r.colorForDb(controlledRecorded[511-age].bins[bin],-140,-40,'reference',100);
        const actual=r.textureData.subarray((physical*r.textureWidth+bin)*4,(physical*r.textureWidth+bin)*4+3);
        if(expected.some((v,c)=>v!==actual[c]))throw Error('Traditional ring color/age mismatch');
        const raw=spectrumHistory.row(age)[bin];
        if(raw!==controlledRecorded[511-age].bins[bin])throw Error('3D raw level mismatch');
        checked++;
      }
    }
    window.controlledTraditionalPng=r.canvas.toDataURL('image/png').split(',')[1];
    setTerrainMode('3d',false);
    if(spectrumHistory.revision!==revision||state.displayPaused!==paused)throw Error('Recorded comparison changed history');
    controlledStyle.remove();terrainRenderer.render(performance.now(),layoutTerrainCanvas(),true);
    return {frames:controlledRecorded.length,checkedRows:checked,historyPreserved:true};
  })()`);
  writeFileSync(join(output,'traditional-recorded.png'),Buffer.from(await evaluate('controlledTraditionalPng'),'base64'));
  const shot=await call('Page.captureScreenshot',{format:'png'});writeFileSync(join(output,'pulse-overlays-on.png'),Buffer.from(shot.data,'base64'));
  report.alignment=[];
  for(const [width,height,dpr] of [[1440,1000,1],[1137,901,1.25],[1280,900,1.5],[390,844,2]]) {
    await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:dpr,mobile:width<500});
    await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
    for(const zoom of [1,4,16]) report.alignment.push(await evaluate(`(()=>{
      state.displayZoom=${zoom};state.terrain.quality='high';spectrumHistory.clear('Alignment fixture');
      const bins=new Float32Array(4096/${zoom}).fill(-110);
      for(let i=0;i<512;i++){acceptSpectrumFrame(bins,fixtureTimestamp,fixtureIndex++,4096);fixtureTimestamp+=33;}
      const r=terrainRenderer;r.configure(state.terrain);r.render(performance.now(),layoutTerrainCanvas(),true);updateAxes();
      const upper=$('spectrum-shell'),lower=$('waterfall-shell'),u=upper.getBoundingClientRect(),l=lower.getBoundingClientRect();let maxErrorHz=0;
      for(const age of [0,10,30])for(const col of [0,Math.floor((r.columns-1)/2),r.columns-1]) {
        const fraction=col/(r.columns-1),p=r.project(age,col),expected=state.dds+(fraction-.5)*displaySpanHz();
        const surface=frequencyFromDisplayPoint(u.left+p[0]*u.width,upper,u.top+p[1]*u.height);
        const waterfall=frequencyFromDisplayPoint(l.left+fraction*l.width,lower,l.top+20);
        const ruler=frequencyFromDisplayPoint(u.left+fraction*u.width,upper,u.top+5);
        maxErrorHz=Math.max(maxErrorHz,Math.abs(surface-expected),Math.abs(waterfall-expected),Math.abs(ruler-expected));
      }
      if(maxErrorHz>displaySpanHz()/r.columns)throw Error('Frequency surfaces disagree after resize/zoom '+maxErrorHz);
      return {width:${width},dpr:${dpr},zoom:${zoom},maxErrorHz,backing:[r.canvas.width,r.canvas.height],css:[r.canvas.getBoundingClientRect().width,r.canvas.getBoundingClientRect().height]};
    })()`));
  }

}
