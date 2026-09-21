import { describe,it,expect } from 'vitest';
import {cleanupNormalized,estimateNoiseFloor} from '../src/dsp/display-cleanup';
import {SpectrumHistory} from '../src/dsp/spectrum-history';
import {normalizeTerrain} from '../src/settings/terrain';

describe('display-only noise-floor shaping',()=>{
 it('is strictly monotone without extra clipping, and leaves high levels and Off unchanged',()=>{
  for(const baseline of [-160,-129,-45,0]) {
   let last=-1;
   for(let i=0;i<=10000;i++) {
    const db=-140+i*.01,n=cleanupNormalized(db,-140,-40,baseline,.85);
    expect(n).toBeGreaterThan(last);last=n;
    expect(n).toBeLessThanOrEqual((db+140)/100+1e-12);
    expect(cleanupNormalized(db,-140,-40,baseline,0)).toBeCloseTo((db+140)/100,12);
    if(db>=baseline+8)expect(n).toBeCloseTo((db+140)/100,12);
   }
   expect(last).toBe(1);
  }
 });
 it('compresses local background variation without erasing a just-above-floor peak',()=>{
  const f=(db:number)=>cleanupNormalized(db,-140,-40,-129,.6);
  expect(f(-128)-f(-130)).toBeCloseTo(.008,12);
  expect(f(-126.5)).toBeGreaterThan(f(-128));
  expect(f(-121)).toBeCloseTo(.19,12);
  const eps=1e-6;
  for(const knee of [-125,-121])expect(Math.abs(f(knee+eps)-f(knee-eps))).toBeLessThan(1e-6);
 });
 it('estimates once per mapping epoch, preserves broadband rises, and supports explicit re-estimate',()=>{
  const h=new SpectrumHistory();
  const frame=(db:number,t:number)=>({bins:new Float32Array(1024).fill(db),timestamp:t,sequence:t,receiver:'rx:0',centerHz:14200000,spanHz:48000,sampleRate:48000,sourceBins:1024,units:'relative dB' as const});
  h.accept(frame(-129,0),33);expect(h.noiseFloor).toBe(-129);
  h.accept(frame(-119,33),33);expect(h.noiseFloor).toBe(-129);expect(h.row(0)![0]).toBe(-119);
  const revision=h.revision;h.reestimateNoiseFloor();expect(h.noiseFloor).toBe(-119);expect(h.revision).toBe(revision);
  h.clear('retune');expect(h.noiseFloor).toBeNull();h.accept(frame(-135,66),33);expect(h.noiseFloor).toBe(-135);
 });
 it('validates optional controls and excludes missing samples without modifying data',()=>{
  const samples=new Float32Array([-129,-128,-130,NaN,-1000,Infinity]);const copy=samples.slice();
  expect(estimateNoiseFloor(samples)).toBe(-129);expect(samples).toEqual(copy);
  expect(estimateNoiseFloor(new Float32Array([-1000,NaN]))).toBeNull();
  expect(normalizeTerrain({cleanup:5,cleanupBaseline:Infinity})).toMatchObject({cleanup:.85,cleanupBaseline:null});
  expect(normalizeTerrain({cleanup:0,cleanupBaseline:-129})).toMatchObject({cleanup:0,cleanupBaseline:-129});
 });
});
