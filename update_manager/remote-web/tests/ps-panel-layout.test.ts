import {describe,it,expect} from 'vitest';
import {panelGeometry} from '../src/puresignal/panel-layout';
import {readFileSync} from 'node:fs';
describe('PS panel layout boundaries',()=>{
  it('recovers an off-screen saved window after a monitor is removed',()=>{
    expect(panelGeometry({width:1000,height:900,left:3500,top:-900},{width:1280,height:720})).toEqual({width:1000,height:720,left:280,top:0});
  });
  it('keeps saved negative coordinates on an available monitor to the left',()=>{
    expect(panelGeometry({width:800,height:600,left:-1900,top:60},{width:1920,height:1080,left:-1920})).toEqual({width:800,height:600,left:-1900,top:60});
  });
  it('bounds corrupted or oversized dimensions and uses safe defaults',()=>{
    expect(panelGeometry({width:NaN,height:Infinity,left:'bad',top:null},{width:1440,height:900})).toEqual({width:960,height:700,left:0,top:0});
    expect(panelGeometry({width:-100,height:1,left:-10,top:10000},{width:1440,height:900})).toEqual({width:320,height:240,left:0,top:660});
    expect(panelGeometry(null,{width:200,height:150})).toEqual({width:200,height:150,left:0,top:0});
  });
  it('keeps window management independent of radio authority and transport',()=>{
    const source=readFileSync(new URL('../src/puresignal/panel-layout.ts',import.meta.url),'utf8');
    expect(source).not.toContain('new WebSocket');expect(source).not.toContain('sendTci');
    expect(source).toContain('popup.opener=null');expect(source).toContain("popup.document.createElement('button')");
    const html=readFileSync(new URL('../../templates/saturn-remote-next.html',import.meta.url),'utf8');
    expect(html).toContain('ps-reset-panel-layout');expect(html).toContain('ps-health-monitor');
  });
});
