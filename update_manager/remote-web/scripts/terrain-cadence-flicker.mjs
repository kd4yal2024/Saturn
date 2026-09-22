// Actual WebGL2 waterfall, identical synthetic 30 Hz IQ / 50 ms WAN schedule.
// Compare the former arrival-time buckets with presentation-time buckets.
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

export async function cadenceFlicker({ evaluate, call, output, report }) {
  await call('Emulation.setDeviceMetricsOverride', {
    width: 1440, height: 1000, deviceScaleFactor: 2, mobile: false,
  });
  await evaluate("applyLayout('desktop',false,false); syncDisplayWorkspaceRatio(); new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
  report.cadenceFlicker = [];
  for (const timestampMode of ['arrival', 'presentation']) {
    const result = await evaluate(`(()=>{
      const mode=${JSON.stringify(timestampMode)}, h=spectrumHistory, r=terrainRenderer;
      state.streamMode='wan'; state.layoutMode='desktop'; state.displayZoom=2;
      state.waterfallSpeed=1; state.sampleRate=384000; state.displayIqSource='rx';
      state.terrain={...state.terrain,quality:'balanced',floor:-140,ceiling:-40,
        gamma:1,cleanup:0,smoothing:0};
      r.configure(state.terrain); layoutTerrainCanvas();
      h.clear('Controlled WAN cadence '+mode);
      const missingBefore=h.missing, aggregatedBefore=h.aggregated;
      const bins=new Float32Array(8192).fill(-120); bins[1024]=-80; bins[4096]=-55;
      let latestArrival=-Infinity, version=0, lastDrawnVersion=0, lastDraw=-Infinity, accepted=0;
      for(let tick=0;tick<=120;tick++) {
        const now=tick*1000/60;
        if(tick%2===0) { latestArrival=now; version++; }
        if(version!==lastDrawnVersion && now-lastDraw>=49) {
          if(!acceptSpectrumFrame(bins,mode==='arrival'?latestArrival:now,version,bins.length))
            throw Error('WAN cadence frame rejected');
          lastDrawnVersion=version; lastDraw=now; accepted++;
        }
      }
      r.render(performance.now(),layoutTerrainCanvas(),true);
      const rect=r.canvas.getBoundingClientRect();
      const upper=Math.round($('spectrum-shell').getBoundingClientRect().height/rect.height*r.canvas.height);
      const lower=r.canvas.height-upper,rowPixels=r.diagnostics().waterfallRowPixels,pixel=new Uint8Array(4),grayAges=[];
      for(let age=0;age<h.count;age++) {
        const top=age*rowPixels;
        if(top>=lower)break;
        r.gl.readPixels(100,lower-1-top,1,1,r.gl.RGBA,r.gl.UNSIGNED_BYTE,pixel);
        const gray=Math.abs(pixel[0]-20)<=1 && Math.abs(pixel[1]-22)<=1 && Math.abs(pixel[2]-26)<=1;
        if(gray) grayAges.push(age);
        if(gray!==(h.row(age)===null)) throw Error('GPU missing-row mismatch '+mode+' age '+age+' '+Array.from(pixel));
      }
      const missingBuckets=h.missing-missingBefore, aggregatedBuckets=h.aggregated-aggregatedBefore;
      if(mode==='arrival' && (missingBuckets<15 || grayAges.length<15))
        throw Error('Old arrival timestamps did not reproduce horizontal gaps');
      if(mode==='presentation' && (missingBuckets!==0 || grayAges.length!==0))
        throw Error('Presentation timestamps still rendered artificial gaps');
      window.cadencePng=r.canvas.toDataURL('image/png').split(',')[1];
      return {timestampMode:mode,sourceFps:30,rafFps:60,displayCadenceMs:50,
        accepted,historyRows:h.count,missingBuckets,aggregatedBuckets,
        gpuGrayRows:grayAges.length,lowerPixels:[r.canvas.width,lower]};
    })()`);
    writeFileSync(join(output,`cadence-${timestampMode}.png`),
      Buffer.from(await evaluate('cadencePng'),'base64'));
    report.cadenceFlicker.push(result);
  }
  report.checks.push('30 Hz source / 50 ms WAN gate: presentation timestamps remove artificial GPU gray bars while retaining real-gap semantics');
}
