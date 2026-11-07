import { describe, expect, it } from 'vitest';
import { TxPathNegotiator } from '../src/transport/tx-path';
import { ManagedPcmSession } from '../src/transport/managed-pcm';
import { readFileSync } from 'node:fs';

describe('host-driven default TX path', () => {
  it('actual page sends receive discovery after managed disconnect without old authority or an exception', () => {
    const html=readFileSync(new URL('../../templates/saturn-remote-next.html',import.meta.url),'utf8');
    const body=html.slice(html.indexOf('    function sendTci(text) {'),html.indexOf('    function sendBridgeRttPing()'));
    const session=new ManagedPcmSession({now:()=>0,changed:()=>{},capture:async()=>{throw new Error('no capture');}});
    session.select(true);session.disconnect();
    const sent: string[]=[];
    const socket={readyState:1,send:(s:string)=>sent.push(s)};
    const path=new TxPathNegotiator();
    const send=new Function('state','WebSocket','txPathNegotiator','managedPcm','managedPcmSelected',body+';return sendTci;')({connected:true,ws:socket},{OPEN:1},path,session,()=>true);
    expect(()=>send('rx_smeter:0,0;iq_start:0;trx:0,true;tx_drive:0,60;')).not.toThrow();
    expect(sent).toEqual(['rx_smeter:0,0;iq_start:0;']);
    expect(session.status().phase).toBe('disconnected');
    expect(()=>send('trx:0,false;')).not.toThrow();
    expect(sent.at(-1)).toBe('trx:0,false;');
  });
  it('retains faulted RX tuning and attenuation capability through fragmented Auto negotiation',()=>{
    const p=new TxPathNegotiator();
    p.observe('saturn_tx_owner:shared,1,ReadyOff,3,11,tci,false,SevereOverdrive;saturn_rx_tuning_caps:0,fault-latched-v1;saturn_ps_fault_attenuation_caps:0,latched-rx-off-v1;saturn_tx_fault_ack_state:0,3,true;');
    p.observe('saturn_ps_manual_attenuation_caps:0,rx-off-ordered-v1;');
    p.observe('saturn_tx_media_caps:psg1,pcm,owner1,transactions1;');p.observe('ready;');
    const controls:string[]=[];
    const session=new ManagedPcmSession({now:()=>0,changed:()=>{},capture:async()=>{throw new Error('no RF/capture');}});
    session.select(true);const epoch=session.connect(s=>controls.push(s),()=>{});session.setOperator(true);
    session.text(p.takeSnapshot()+'tx_codec_accept:0,pcm;',epoch);
    expect(session.status().receiveTuningReady).toBe(true);
    expect(session.status().canAdjustFaultedFeedback).toBe(true);
    session.sendControl('vfo:0,0,7100000;dds:0,7100000;');
    expect(controls.at(-1)).toBe('vfo:0,0,7100000;dds:0,7100000;');
    expect(session.status().phase).toBe('fault');expect(session.arm('tci')).toBe(false);
  });
  it('retains manual ATT and separately negotiated Auto through fragmented startup without transmitting', () => {
    const p = new TxPathNegotiator();
    p.observe('saturn_ps_manual_attenuation_caps:0,rx-off-ordered-v1;');
    p.observe('saturn_ps_auto_caps:0,continuous-wdsp-v1;');
    p.observe('saturn_tx_media_caps:psg1,pcm,owner1,transactions1;ready;');
    const controls: string[] = [], media: ArrayBuffer[] = [];
    const session = new ManagedPcmSession({now:()=>0,changed:()=>{},capture:async()=>{throw new Error('no capture');}});
    session.select(p.path === 'managed');
    const epoch=session.connect(c=>controls.push(c),f=>media.push(f));
    session.text(p.takeSnapshot(),epoch);
    expect(session.status().manualAttenuationSupported).toBe(true);
    expect(session.status().autoAdaptationSupported).toBe(true);
    expect(controls).toEqual(['tx_codec_caps:0,pcm;']);expect(media).toEqual([]);
    session.disconnect();expect(session.status().manualAttenuationSupported).toBe(false);
    expect(session.status().autoAdaptationSupported).toBe(false);
  });
  it('retains explicit generated-trial limits across Auto without authorizing output', () => {
    const p=new TxPathNegotiator();
    p.observe('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;');
    p.observe('saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
    p.observe('saturn_tx_media_caps:psg1,pcm,owner1,transactions1;ready;');
    const snapshot=p.takeSnapshot();
    expect(snapshot).toContain('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;');
    expect(snapshot).toContain('saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
  });
  it('retains timing capability across the initial Auto snapshot', () => {
    const path=new TxPathNegotiator();
    path.observe('saturn_ps_stimulus_caps:0,saturn-timing-multisine-v1;');
    path.observe('saturn_tx_media_caps:psg1,pcm,owner1,transactions1;ready;');
    expect(path.path).toBe('managed');
    expect(path.takeSnapshot()).toContain('saturn_ps_stimulus_caps:0,saturn-timing-multisine-v1;');
  });
  it('waits across snapshot messages, selects shared only at ready, and retains negotiation', () => {
    const path = new TxPathNegotiator();
    expect(path.ready).toBe(false);
    expect(path.observe('tx_enable:0,true;saturn_ps_control_caps:0,measurement-only;')).toBe(false);
    expect(path.observe('saturn_tx_media_caps:psg1,pcm,owner1,transactions1;')).toBe(false);
    expect(path.observe('saturn_tx_owner:shared,1,ReadyOff,1,1,tci,true,none;ready;')).toBe(true);
    expect(path.path).toBe('managed');
    expect(path.takeSnapshot()).toContain('tx_enable:0,true;');
    expect(path.takeSnapshot()).toBe(';');
    expect(path.observe('ready;')).toBe(false);
  });
  it('keeps older Legacy hosts working, but never guesses before ready', () => {
    const path = new TxPathNegotiator();
    path.observe('protocol:1.8;tx_enable:0,true;');
    expect(path.path).toBe('pending');
    path.observe('ready;'); expect(path.path).toBe('legacy');
    expect(path.filterControl('tx_mic_gain:0,0;')).toBe('tx_mic_gain:0,0;');
  });
  it.each(['auto', 'legacy', 'managed'] as const)('drops pre-negotiation TX intent, never queues it (%s)', preference => {
    const path = new TxPathNegotiator(preference);
    const input = 'tx_codec_caps:0,pcm,opus_wb;tx_puresignal_auto_attenuate:0,true;tx_two_tone:0,false;'
      + 'tx_noise_gate:0,true;tx_drive:0,60;modulation:0,AM;dds:0,7200000;trx:0,true;'
      + 'saturn_ps_measurement:0,start,45000;rx_smeter:0,0;iq_start:0;trx:0,false;';
    expect(path.filterControl(input)).toBe('rx_smeter:0,0;iq_start:0;trx:0,false;');
    path.observe('ready;'); expect(path.takeSnapshot()).not.toContain('trx:0,true');
  });
  it.each([['managed', 'ready;'], ['legacy', 'saturn_tx_media_caps:psg1,pcm;ready;']] as const)(
    'blocks explicit mismatches rather than falling back (%s)', (preference, snapshot) => {
      const path = new TxPathNegotiator(preference); path.observe(snapshot);
      expect(path.path).toBe('blocked'); expect(path.ready).toBe(false);
      expect(path.reason).toContain('select Auto and reconnect');
      expect(path.filterControl('trx:0,true;tx_drive:0,5;')).toBe('');
    });
  it('never switches an established Legacy connection to shared in place', () => {
    const path = new TxPathNegotiator(); path.observe('ready;');
    path.observe('saturn_tx_owner:shared,1,ReadyOff,1,1,tci,true,none;');
    expect(path.path).toBe('blocked'); expect(path.ready).toBe(false);
  });
  it('bounds retained metadata and excludes pairing secrets and large monitor data', () => {
    const path = new TxPathNegotiator();
    path.observe('saturn_satp_key:secret;saturn_ps_spectrum:' + 'x'.repeat(20000) + ';ready;');
    expect(path.takeSnapshot()).toBe('ready;');
    const bad = new TxPathNegotiator();
    bad.observe('saturn_tx_media_caps:' + 'x'.repeat(17000) + ';ready;');
    expect(bad.path).toBe('blocked'); expect(bad.ready).toBe(false);
  });
  it('a new connection rediscovers a different host without carrying authority', () => {
    const first = new TxPathNegotiator(); first.observe('saturn_tx_media_caps:psg1,pcm;ready;');
    expect(first.path).toBe('managed');
    const next = new TxPathNegotiator(); expect(next.ready).toBe(false);
    next.observe('ready;'); expect(next.path).toBe('legacy');
  });
  it('automatic shared selection sends PCM negotiation only, never settings, media or an arm', () => {
    const path = new TxPathNegotiator();
    const messages = 'tx_enable:0,true;saturn_tx_media_caps:psg1,pcm,owner1,transactions1;'
      + 'saturn_tx_owner:shared,1,ReadyOff,1,1,tci,true,none;ready;';
    path.observe(messages);
    const controls: string[] = [], media: ArrayBuffer[] = [];
    let captures = 0;
    const session = new ManagedPcmSession({now:()=>0,changed:()=>{},capture:async()=>{
      captures++; throw new Error('must not capture during negotiation');
    }});
    session.select(path.path === 'managed');
    const epoch = session.connect(c=>controls.push(c),f=>media.push(f));
    session.text(path.takeSnapshot(),epoch);
    expect(controls).toEqual(['tx_codec_caps:0,pcm;']);
    expect(media).toEqual([]); expect(captures).toBe(0);
  });
  it('the shipped page defaults to Auto and no longer sends TX profiles at socket open', () => {
    const html = readFileSync(new URL('../../templates/saturn-remote-next.html',import.meta.url),'utf8');
    expect(html).toContain('<option value="auto" selected>');
    const opened = html.slice(html.indexOf('ws.onopen = () => {'),html.indexOf('ws.onmessage = (event) => {'));
    expect(opened).not.toContain('sendTxCodecCaps();');
    expect(opened).not.toContain('sendTxOpusLabTxAudioToBridgeOnce(');
    expect(html).toContain('text = txPathNegotiator.filterControl(text);');
  });
});
