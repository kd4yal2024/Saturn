import {writeFileSync} from 'node:fs';
import {join} from 'node:path';
export async function cleanupFixtures({evaluate,call,output,report,phase}) {
  report.cleanup=[];
  await evaluate(`(()=>{
    state.terrain={...state.terrain,floor:-140,ceiling:-40,gamma:1,height:.65,smoothing:0,quality:'balanced',depth:128,cleanup:0,waterfallCleanup:0,cleanupBaseline:-129};
    const style=document.createElement('style');style.textContent='[data-terrain="true"] .display-stack .spectrum-shell > :not(canvas),[data-terrain="true"] .display-stack .waterfall-shell > :not(canvas){visibility:hidden!important}';document.head.appendChild(style);
  })()`);
  for(const kind of ['noise','noise-wide','weak','carriers','brief','strong','rise']) {
    const result=await evaluate(`(()=>{
      const kind=${JSON.stringify(kind)},r=terrainRenderer,h=spectrumHistory,bins=new Float32Array(4096);
      h.clear('Cleanup '+kind);let seed=1234567;
      for(let t=0;t<1800;t++) {
        for(let i=0;i<4096;i++) {seed=(Math.imul(seed,1664525)+1013904223)|0;bins[i]=-129+((seed>>>0)/4294967296-.5)*(kind==='noise-wide'?8:2.4);}
        if(kind==='weak'||kind==='carriers'||(kind==='brief'&&t===1680))bins[738]=-126.5;
        if(kind==='carriers'){bins[1500]=-124;bins[2170]=-58;}
        if(kind==='strong')bins[2170]=-58;
        if(kind==='rise'&&t>=1675&&t<=1680)for(let i=0;i<4096;i++)bins[i]+=10;
        acceptSpectrumFrame(bins,fixtureTimestamp,fixtureIndex++,4096);fixtureTimestamp+=33;
      }
      const raw=h.data.slice(),revision=h.revision,baseline=h.noiseFloor;
      const outputs=[];window.cleanupPngs=[];let upperReference=null;
      for(const strength of ${phase==='before'?'[0]':'[0,.9]'}) {
        r.configure({...state.terrain,cleanup:0,waterfallCleanup:strength});r.render(performance.now(),layoutTerrainCanvas(),true);
        const w=r.canvas.width,upper=Math.round($('spectrum-shell').getBoundingClientRect().height/r.canvas.getBoundingClientRect().height*r.canvas.height),lower=r.canvas.height-upper;
        const pixels=new Uint8Array(w*lower*4);r.gl.readPixels(0,0,w,lower,r.gl.RGBA,r.gl.UNSIGNED_BYTE,pixels);
        const upperPixels=new Uint8Array(w*upper*4);r.gl.readPixels(0,lower,w,upper,r.gl.RGBA,r.gl.UNSIGNED_BYTE,upperPixels);
        if(upperReference) {
          for(let p=0;p<upperPixels.length;p++)if(upperPixels[p]!==upperReference[p])throw Error('Lower cleanup changed an upper 3D pixel');
        } else upperReference=upperPixels;
        const expectedColor=db=>terrainColor(${phase==='before'?'_next.cleanupNormalized(db,-140,-40,-129,strength)':'_next.waterfallCleanupNormalized(db,-140,-40,-129,strength)'},'reference');
        const samples=[];
        for(let y=0;y<lower;y++)for(let x=Math.floor(w*.04);x<Math.floor(w*.13);x++) {
          const p=(y*w+x)*4;samples.push(.2126*pixels[p]+.7152*pixels[p+1]+.0722*pixels[p+2]);
        }
        const mean=samples.reduce((a,b)=>a+b,0)/samples.length,sd=Math.sqrt(samples.reduce((a,b)=>a+(b-mean)**2,0)/samples.length);
        let checked=0;const carriers=[738,1500,2170];
        for(let y=0;y<lower;y++)for(const x of [Math.floor(w*.05),...carriers.map(i=>Math.floor(i*w/4096))]) {
          const a=Math.max(0,Math.floor((lower-1-y)*512/lower)),end=Math.min(512,Math.max(a+1,Math.floor((lower-y)*512/lower)));
          const lo=Math.floor(x*4096/w),hi=Math.max(lo+1,Math.floor((x+1)*4096/w));let db=-1000;
          for(let age=a;age<end;age++)for(let bin=lo;bin<hi;bin++)db=Math.max(db,h.row(age)[bin]);
          const color=expectedColor(db),offset=(y*w+x)*4;
          if(color.some((v,c)=>Math.abs(v-pixels[offset+c])>3))throw Error('GPU cleanup mapping or event chronology mismatch');checked++;
        }
        // Exact palette footprints at selected event rows: no additional columns,
        // no widened carrier and no duplicate/averaged event rows.
        const weak=expectedColor(-126.5),weakRows=[],weakColumns=new Set();
        if(['weak','carriers','brief'].includes(kind))for(let y=0;y<lower;y++) {
          let present=false;
          for(let x=Math.floor(738*w/4096)-3;x<=Math.floor(738*w/4096)+3;x++) {
            const p=(y*w+x)*4;
            if(weak.every((v,c)=>Math.abs(v-pixels[p+c])<=1)){present=true;weakColumns.add(x);}
          }
          if(present)weakRows.push(y);
        }
        if(['weak','carriers'].includes(kind)&&weakRows.length!==lower)throw Error('Weak stationary carrier lost rows');
        if(kind==='brief'&&(weakRows.length<1||weakRows.length>3||weakRows.some((y,i)=>i&&y!==weakRows[i-1]+1)))throw Error('Brief weak event lost or smeared');
        if(weakRows.length&&weakColumns.size!==1)throw Error('Weak single-bin carrier widened');
        const backgroundColor=expectedColor(-129),contrast=Math.sqrt(weak.reduce((sum,v,c)=>sum+(v-backgroundColor[c])**2,0));
        window.cleanupPngs.push(r.canvas.toDataURL('image/png').split(',')[1]);
        outputs.push({strength,meanLuminance:mean,backgroundStdDev:sd,weakRgb:weak,backgroundRgb:backgroundColor,weakColorDistance:contrast,weakRows:weakRows.length,weakColumns:weakColumns.size,checkedPixels:checked});
      }
      if(revision!==h.revision||raw.some((v,i)=>v!==h.data[i]))throw Error('Cleanup changed numerical history');
      if(outputs.length===2) {
        if(['noise','noise-wide'].includes(kind)&&(outputs[1].meanLuminance>=outputs[0].meanLuminance*.8||outputs[1].backgroundRgb[2]<10))throw Error('Noise cleanup did not darken the background while retaining low-level detail: '+JSON.stringify(outputs));
        if(kind!=='rise'&&outputs[1].weakColorDistance/outputs[1].backgroundStdDev<=outputs[0].weakColorDistance/outputs[0].backgroundStdDev)throw Error('Weak carrier separation relative to clutter did not improve');
        if(outputs[1].weakRows!==outputs[0].weakRows||outputs[1].weakColumns!==outputs[0].weakColumns)throw Error('Cleanup changed carrier/event footprint');
      }
      return {kind,frames:1800,baseline,rawPreserved:true,upperPixelsUnchanged:true,outputs};
    })()`);
    const pngs=await evaluate('cleanupPngs');
    for(let i=0;i<pngs.length;i++)writeFileSync(join(output,kind+(i?'-on.png':'-off.png')),Buffer.from(pngs[i],'base64'));
    report.cleanup.push(result);
  }
}
