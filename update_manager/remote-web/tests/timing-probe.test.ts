import {describe,expect,it} from 'vitest';
import {createTimingProbePcm} from '../src/puresignal/timing-probe';
describe('offline finite timing stimulus',()=>{
  it('is deterministic, finite, tapered and below the existing two-tone peak ceiling',()=>{
    const a=createTimingProbePcm(),b=createTimingProbePcm();
    expect(a.samples).toEqual(b.samples);expect(a.samples.length).toBe(48000);
    expect(Math.abs(a.samples[0]!)).toBe(0);expect(Math.abs(a.samples.at(-1)!)).toBe(0);
    expect(a.componentPeak).toBeLessThan(.4);expect(a.componentPeak).toBeGreaterThan(.39);
    expect(a.rms).toBeGreaterThan(.06);expect(a.rms).toBeLessThan(.2);
    expect(a.frequenciesHz.length).toBe(32);
    expect(a.frequenciesHz.every(f=>f>400&&f<2600)).toBe(true);
    expect(a.samples.every(Number.isFinite)).toBe(true);
  });
  it.each([0,99,45001,Infinity,NaN,100.5])('rejects invalid finite preparation %s',ms=>{
    expect(()=>createTimingProbePcm(ms)).toThrow();
  });
  it('does not repeat at the old two-tone beat period',()=>{
    const {samples}=createTimingProbePcm();
    let difference=0;
    for(let i=1000;i<10000;i++)difference+=Math.abs(samples[i]!-samples[i+40]!);
    expect(difference).toBeGreaterThan(100);
  });
});
