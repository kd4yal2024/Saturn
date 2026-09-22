// Actual renderer, deterministic adapter input. GPU readback is test-only.
import {writeFileSync} from 'node:fs';
import {join} from 'node:path';
export async function sharpness({evaluate,call,output,report,phase}) {
  report.sharpness=[];
  await evaluate(`(()=>{
    state.terrain={...state.terrain,floor:-140,ceiling:-40,gamma:1,cleanup:.6,waterfallCleanup:.6,cleanupBaseline:-129,smoothing:0};
    const style=document.createElement('style');style.textContent='[data-terrain="true"] .display-stack .spectrum-shell > :not(canvas),[data-terrain="true"] .display-stack .waterfall-shell > :not(canvas){visibility:hidden!important}';document.head.appendChild(style);
  })()`);
  for(const [width,height,dpr,quality] of [[1440,1000,1,'balanced'],[1137,901,1.25,'performance'],[1440,1000,2,'balanced'],[390,844,2,'performance']]) {
    await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:dpr,mobile:width<500});
    await evaluate(`applyLayout('${width<500?'phone':'desktop'}',false,false); syncDisplayWorkspaceRatio(); new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))`);
    for(const kind of ['constant','noise','carriers','short','pulse','gap']) {
      const result=await evaluate(`(()=>{
        const h=spectrumHistory,r=terrainRenderer,bins=new Float32Array(4096),kind=${JSON.stringify(kind)};
        state.terrain.quality=${JSON.stringify(quality)};r.configure(state.terrain);h.clear('Sharpness '+kind);
        fixtureTimestamp=0;fixtureIndex=0;let seed=1234567;
        for(let t=0;t<1800;t++) {
          for(let i=0;i<4096;i++){seed=(Math.imul(seed,1664525)+1013904223)|0;bins[i]=kind==='constant'?-129:-129+((seed>>>0)/4294967296-.5)*2.4;}
          if(kind==='carriers'){bins[738]=-126.5;bins[2170]=-58;bins[2190]=-65;}
          if(kind==='short'&&t===1679)bins[738]=-126.5;
          if(kind==='pulse'&&t===1679)bins.fill(-60);
          if(kind!=='gap'||t!==1680)acceptSpectrumFrame(bins,fixtureTimestamp,fixtureIndex,4096);fixtureIndex++;fixtureTimestamp+=33;
          if(t%256===0)r.render(performance.now(),layoutTerrainCanvas(),true);
        }
        r.render(performance.now(),layoutTerrainCanvas(),true);
        const w=r.canvas.width,upper=Math.round($('spectrum-shell').getBoundingClientRect().height/r.canvas.getBoundingClientRect().height*r.canvas.height),lower=r.canvas.height-upper;
        const pixels=new Uint8Array(w*lower*4);r.gl.readPixels(0,0,w,lower,r.gl.RGBA,r.gl.UNSIGNED_BYTE,pixels);
        const color=db=>terrainColor(${phase==='before'?'_next.cleanupNormalized(db,-140,-40,-129,.6)':'_next.waterfallCleanupNormalized(db,-140,-40,-129,.6)'},'reference');
        let spread=0,checked=0;const eventRows=[];
        for(let y=0;y<lower;y++) {
          const top=lower-1-y;
          const a=${phase==='before'?'Math.floor((1-(y+1)/lower)*512)':'Math.floor(top*512/lower)'};
          const end=${phase==='before'?'Math.ceil((1-y/lower)*512)':'Math.max(a+1,Math.floor((top+1)*512/lower))'};
          for(const x of [0,...[738,2170,2190].flatMap(bin=>[-1,0,1].map(dx=>Math.ceil((bin+1)*w/4096)-1+dx)),w-1]) {
            let db=-1000,missing=false;
            for(let age=a;age<end;age++) {
              const row=h.row(age);if(!row){missing=true;continue;}
              for(let i=Math.floor(x*4096/w);i<Math.max(Math.floor(x*4096/w)+1,Math.floor((x+1)*4096/w));i++)db=Math.max(db,row[i]);
            }
            const expected=missing?[20,22,26]:color(db),p=(y*w+x)*4;
            if(expected.some((v,c)=>Math.abs(v-pixels[p+c])>3))throw Error('Sharpness GPU mapping '+kind+' '+x+','+y);
            checked++;
          }
          const p=(y*w+(Math.ceil(739*w/4096)-1))*4;
          if((kind==='pulse'&&pixels[p]>200)||(kind==='short'&&color(-126.5).every((v,c)=>Math.abs(v-pixels[p+c])<=1)))eventRows.push(top);
        }
        if(kind==='constant')for(let p=0;p<pixels.length;p++)if(p%4!==3)spread=Math.max(spread,Math.abs(pixels[p]-pixels[p%4]));
        if(['short','pulse'].includes(kind)&&eventRows.length===0)throw Error('Brief event lost');
        if(spread>0)throw Error('Uniform input contains stripes');
        if(${phase!=='before'}&&lower<512&&['short','pulse'].includes(kind)&&eventRows.length!==1)throw Error('Compressed event duplicated or lost');
        window.sharpPng=r.canvas.toDataURL('image/png').split(',')[1];
        return {kind,viewport:[${width},${height}],dpr:${dpr},quality:${JSON.stringify(quality)},waterfallPixels:[w,lower],checked,constantSpread:spread,eventRows,diagnostics:r.diagnostics()};
      })()`);
      const name=`${width}-${dpr}-${quality}-${kind}`;
      writeFileSync(join(output,name+'-raw.png'),Buffer.from(await evaluate('sharpPng'),'base64'));
      if(width<500)await evaluate(`document.querySelector('.display-card').scrollIntoView({block:'start'})`);
      const shot=await call('Page.captureScreenshot',{format:'png'});
      writeFileSync(join(output,name+'.png'),Buffer.from(shot.data,'base64'));
      report.sharpness.push(result);
    }
  }
}
