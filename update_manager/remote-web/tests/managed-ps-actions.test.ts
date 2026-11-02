import { describe, expect, it, vi } from 'vitest';
import { ManagedPcmSession } from '../src/transport/managed-pcm';
import { twoToneManagedCapture } from '../src/audio/managed-two-tone';
import { createTimingProbePcm, TIMING_PROBE_VERSION } from '../src/puresignal/timing-probe';
const TOKEN = '00112233445566778899aabbccddeeff';
const PIN = 'a'.repeat(64), NATIVE = 'b'.repeat(64);
const settle = async () => { await Promise.resolve(); await Promise.resolve(); };
function fixture(capability = 'measurement-only', captureMode = 'raw') {
  let now = 100, input!: (s: Float32Array, t: number) => void;
  const controls: string[] = [], media: ArrayBuffer[] = [], close = vi.fn(), microphone = vi.fn();
  const timingCapture = vi.fn(async (token: string, deliver: (s: Float32Array, t: number) => void) => { input = deliver; return {token, historyResetAtMs: now, close}; });
  const twoToneCapture = vi.fn(async (token: string, deliver: (s: Float32Array, t: number) => void) => { input = deliver; return {token, historyResetAtMs: now, close}; });
  const nativeKeepaliveCapture = vi.fn(async (token: string, deliver: (s: Float32Array, t: number) => void) => { input = deliver; return {token, historyResetAtMs: now, close}; });
  const session = new ManagedPcmSession({ now: () => now, changed: vi.fn(), capture: microphone,
    measurementCapture: async (token, deliver) => { input = deliver; return { token, historyResetAtMs: now, close }; }, timingCapture, twoToneCapture, nativeKeepaliveCapture });
  session.select(true);
  const epoch = session.connect(c => controls.push(c), frame => media.push(frame));
  session.setOperator(true);
  const text = (value: string) => session.text(value, epoch);
  text(`saturn_tx_media_caps:psg1,pcm,owner1,transactions1;tx_codec_accept:0,pcm;tx_enable:0,true;
    saturn_ps_control_caps:0,${capability};saturn_ps_mode:0,11,off;
    saturn_ps_measurement_caps:0,true,${captureMode === 'raw' ? 1000 : 45000},rx-bypass,${PIN},${NATIVE},7200000,${captureMode};
    saturn_tx_owner:shared,1,ReadyOff,2,11,tci,true,none;ready;`);
  return { session, epoch, text, controls, media, close, microphone, timingCapture, twoToneCapture, nativeKeepaliveCapture,
    start: (duration = 500, frequency = 7200000, watts = 5, attended = true) => session.startMeasurement(duration, frequency, watts, attended),
    input: () => input(new Float32Array(512).fill(.2), now),
    offer: () => text(`saturn_tx_generation:psg1,${TOKEN},1;`),
    advance: (ms: number) => { now += ms; session.tick(); } };
}
describe('explicit PS actions through the one managed owner', () => {
  it('operator two-tone follows radio settings, runs until Stop, and retains release cleanup', async () => {
    const h=fixture('single');
    h.text('saturn_ps_two_tone_caps:0,operator-two-tone-v1,0,0,100;saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
    expect(h.session.status().twoToneOperatorControlled).toBe(true);
    expect(h.session.status().twoToneMaxWatts).toBe(100);
    expect(h.session.startTwoTone(0,3897000,60,true)).toBe(true);
    expect(h.controls.at(-1)).toBe('saturn_ps_two_tone:0,1,start,0,wdsp-postgen-two-tone-v1;');
    h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,accepted;');h.offer();await settle();
    h.input();
    // Deliver fresh input before each service turn: elapsed TX time alone is
    // not a timeout, whereas a dead source still has its original watchdog.
    for(let n=0;n<2000;n++){h.advance(100);h.input();}
    expect(h.session.status().phase).not.toBe('fault');
    expect(h.session.status().twoToneActive).toBe(true);
    h.session.release();h.input();expect(h.close).toHaveBeenCalledOnce();
    expect(h.controls.at(-1)).toBe('trx:0,false;');
    h.text('saturn_ps_two_tone:0,1,complete,none;saturn_tx_owner:shared,2,ReadyOff,3,11,tci,true,none;');
    expect(h.session.status().twoToneActive).toBe(false);
  });
  it('operator capability does not bypass source loss, disconnect, or host power limits', async () => {
    const h=fixture('single');
    h.text('saturn_ps_two_tone_caps:0,operator-two-tone-v1,0,0,100;');
    expect(h.session.startTwoTone(0,27385000,101,true)).toBe(false);
    expect(h.session.startTwoTone(90000,27385000,100,true)).toBe(true);
    h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,accepted;');h.offer();await settle();
    h.advance(600);expect(h.session.status().phase).toBe('fault');
    expect(h.controls).toContain('trx:0,false;');h.session.disconnect();
    expect(h.session.status().twoToneOperatorControlled).toBe(false);
  });

  it('Auto is separately negotiated, settings-only, and cleared on disconnect', () => {
    const h=fixture('single');
    expect(h.session.preparePureSignalMode('auto')).toBe(false);
    expect(h.controls).toEqual(['tx_codec_caps:0,pcm;']);
    h.text('saturn_ps_auto_caps:0,continuous-wdsp-v1;');
    expect(h.session.preparePureSignalMode('auto')).toBe(true);
    expect(h.controls.at(-1)).toBe('tx_puresignal_mode:0,auto;');
    expect(h.media).toHaveLength(0);expect(h.microphone).not.toHaveBeenCalled();
    h.text('tx_config:0,1,queued,12;saturn_tx_owner:shared,2,ReadyOff,3,12,tci,true,none;saturn_ps_mode:0,12,auto;tx_config:0,1,applied,12;');
    h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
    expect(h.session.status().psMode).toBe('auto');
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(true);
    h.session.release();h.session.disconnect();
    expect(h.session.status().autoAdaptationSupported).toBe(false);
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(false);
  });
  it('MeasurementOnly cannot select Auto even with an unrelated capability packet', () => {
    const h=fixture();h.text('saturn_ps_auto_caps:0,continuous-wdsp-v1;');
    expect(h.session.preparePureSignalMode('auto')).toBe(false);
    expect(h.controls).toEqual(['tx_codec_caps:0,pcm;']);
  });
  it('native-two-tone-only host cannot authorize ordinary microphone PTT or a legacy generated fallback', () => {
    const h=fixture('single');
    h.text('saturn_tx_arm_policy:0,native-two-tone-only;saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_mode:0,12,single;');
    expect(h.session.status().generatedTrialsOnly).toBe(true);
    expect(h.session.arm('tci')).toBe(false);expect(h.session.arm('satp')).toBe(false);
    expect(h.controls).not.toContain('trx:0,true;');
    expect(h.session.startTwoTone(15000,7200000,5,true)).toBe(false);
    h.text('saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
    expect(h.session.startTwoTone(15000,7200000,5,true)).toBe(true);
    expect(h.controls.at(-1)).toBe('saturn_ps_two_tone:0,1,start,15000,wdsp-postgen-two-tone-v1;');
    expect(h.media).toHaveLength(0);expect(h.microphone).not.toHaveBeenCalled();
  });
  it('cannot erase a preceding fault with successful measurement completion/export', () => {
    const h=fixture(); expect(h.start()).toBe(true);
    h.text('saturn_ps_measurement:0,1,queued;saturn_ps_measurement:0,1,admitted;');
    h.text('tx_fault:0,shared_config,InvalidConfig;');
    h.text('saturn_ps_measurement:0,1,complete,20,none;saturn_ps_measurement:0,1,exported,/offline/capture.json;saturn_tx_owner:shared,2,ReadyOff,3,11,tci,true,none;');
    expect(h.session.status().phase).toBe('fault');
    expect(h.session.status().reason).toContain('InvalidConfig');
    expect(h.start()).toBe(false); expect(h.media).toHaveLength(0);
  });
  it.each(['measurement-only','single'])('negotiates explicit native tones and fences keepalive release: %s', async capability => {
    const h=fixture(capability);
    h.text('saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
    if(capability==='single')h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_mode:0,12,single;');
    expect(h.session.status().nativeTwoToneSupported).toBe(true);
    expect(h.nativeKeepaliveCapture).not.toHaveBeenCalled();
    expect(capability==='single' ? h.session.startTwoTone(500,7200000,5,true) : h.start()).toBe(true);
    expect(h.controls.at(-1)).toContain(',500,wdsp-postgen-two-tone-v1;');
    if(capability==='single')h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,accepted;');
    h.offer();await settle();h.input();
    expect(h.nativeKeepaliveCapture).toHaveBeenCalledOnce();expect(h.microphone).not.toHaveBeenCalled();expect(h.twoToneCapture).not.toHaveBeenCalled();
    h.text('saturn_ps_native_two_tone_caps:0,unsupported;');h.input();
    expect(h.session.status().phase).toBe('fault');expect(h.media).toHaveLength(1);expect(h.close).toHaveBeenCalledOnce();
    h.session.disconnect();h.offer();await settle();expect(h.nativeKeepaliveCapture).toHaveBeenCalledOnce();
    expect(h.session.status().nativeTwoToneSupported).toBe(false);
  });
  it('keeps silent AM and timing separate from native two-tone negotiation',()=>{
    const am=fixture();am.text('saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
    expect(am.session.startMeasurement(500,7200000,5,true,false,true)).toBe(true);
    expect(am.controls.at(-1)).toBe('saturn_ps_measurement:0,start,500;');
    const t=fixture();t.text('saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;saturn_ps_stimulus_caps:0,saturn-timing-multisine-v1;');
    expect(t.session.startMeasurement(500,7200000,5,true,true)).toBe(true);
    expect(t.controls.at(-1)).toBe('saturn_ps_measurement:0,start,500,saturn-timing-multisine-v1;');
  });
  it('generated actions require explicit host opt-in; negotiation never keys', () => {
    const h=fixture('single');
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(false);
    h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;');
    h.text('saturn_ps_mode:0,12,single;');
    for(const [duration,freq,watts,attended] of [[500,7200000,5,false],[45001,7200000,5,true],
      [500,14200000,5,true],[500,7200000,6,true]] as const)
      expect(h.session.startTwoTone(duration,freq,watts,attended)).toBe(false);
    expect(h.controls).toEqual(['tx_codec_caps:0,pcm;']);
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(true);
    expect(h.controls.at(-1)).toBe('saturn_ps_two_tone:0,1,start,500;');
    expect(h.twoToneCapture).not.toHaveBeenCalled();expect(h.microphone).not.toHaveBeenCalled();
    const diagnostic=fixture();diagnostic.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_mode:0,12,single;');
    expect(diagnostic.session.startTwoTone(500,7200000,5,true)).toBe(false);
  });
  it('PS-off baseline uses the same native generator without requesting calibration', async () => {
    const h=fixture('single');
    h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_native_two_tone_caps:0,wdsp-postgen-two-tone-v1;');
    expect(h.session.status().psMode).toBe('off');
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(true);
    expect(h.controls.at(-1)).toBe('saturn_ps_two_tone:0,1,start,500,wdsp-postgen-two-tone-v1;');
    expect(h.controls).not.toContain('tx_puresignal:0,true;');
    h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,accepted;');h.offer();await settle();h.input();
    expect(h.nativeKeepaliveCapture).toHaveBeenCalledOnce();expect(h.microphone).not.toHaveBeenCalled();
    expect(h.session.status().psMode).toBe('off');
  });
  it('single-fit uses only the generated factory and finite release waits for terminal reply AND new clean owner', async () => {
    const h=fixture('single');h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_mode:0,12,single;');
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(true);
    h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,accepted;');h.offer();await settle();h.input();
    expect(h.media).toHaveLength(1);expect(h.twoToneCapture).toHaveBeenCalledOnce();expect(h.microphone).not.toHaveBeenCalled();
    h.advance(500);h.input();expect(h.media).toHaveLength(1);expect(h.close).toHaveBeenCalledOnce();
    expect(h.controls.at(-1)).toBe('trx:0,false;');expect(h.session.status().phase).toBe('releasing');
    h.text('saturn_ps_two_tone:0,1,complete,none;');expect(h.session.status().twoToneActive).toBe(true);
    expect(h.session.arm('tci')).toBe(false);
    h.text('saturn_tx_owner:shared,2,ReadyOff,3,11,tci,true,none;');
    expect(h.session.status().twoToneActive).toBe(false);expect(h.session.status().phase).toBe('ready');
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(true);
    h.text('saturn_ps_two_tone:0,1,accepted;saturn_ps_two_tone:0,1,complete,none;');
    expect(h.session.status().twoToneActive).toBe(true);expect(h.session.status().phase).toBe('arming');
  });
  it('ordinary release before grant cannot be undone by accepted reply, callback or old generation', async () => {
    const h=fixture('single');h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_mode:0,12,single;');
    h.session.startTwoTone(500,7200000,5,true);h.session.release();
    h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,accepted;');h.offer();await settle();
    expect(h.twoToneCapture).not.toHaveBeenCalled();expect(h.media).toEqual([]);
    h.session.disconnect();h.text('saturn_ps_two_tone:0,1,complete,none;');
    expect(h.session.status().twoToneActive).toBe(false);expect(h.session.status().twoToneLimitMs).toBeUndefined();
  });
  it('owner rejection of a locally cancelled queued arm is not a false authorization fault or cleanup receipt', () => {
    const h=fixture('single');h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_mode:0,12,single;');
    h.session.startTwoTone(500,7200000,5,true);h.session.release();
    h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,rejected,GenerationMismatch;');
    expect(h.session.status().phase).toBe('releasing');expect(h.session.status().twoToneActive).toBe(true);
    h.text('saturn_tx_owner:shared,2,ReadyOff,3,11,tci,true,none;');
    expect(h.session.status().phase).toBe('ready');expect(h.session.status().twoToneActive).toBe(false);
    expect(h.session.startTwoTone(500,7200000,5,true)).toBe(true);
    h.text('saturn_ps_two_tone:0,1,rejected,GenerationMismatch;');
    expect(h.session.status().phase).toBe('arming');
    h.text('saturn_tx_owner:shared,3,ReadyOff,4,11,tci,true,TxAuthority;');
    expect(h.session.status().phase).toBe('fault');
  });
  it('pre-queue rejection preserves reason; withdrawn capability and cleanup fault close output', async () => {
    for (const result of ['saturn_ps_two_tone:0,1,rejected,InvalidConfig;',
      'saturn_ps_two_tone_caps:0,unsupported;',
      'saturn_ps_two_tone:0,1,complete,NativeFailure;']) {
      const h=fixture('single');h.text('saturn_ps_two_tone_caps:0,tagged-two-tone-v1,45000,7200000,5;saturn_ps_mode:0,12,single;');
      h.session.startTwoTone(500,7200000,5,true);
      if(!result.includes('rejected'))h.text('saturn_ps_two_tone:0,1,queued;saturn_ps_two_tone:0,1,accepted;');
      h.text(result);expect(h.session.status().phase).toBe('fault');
      if(result.includes('InvalidConfig'))expect(h.session.status().reason).toContain('InvalidConfig');
      h.offer();await settle();expect(h.twoToneCapture).not.toHaveBeenCalled();expect(h.media).toEqual([]);
    }
  });
  it('requires explicit timing support and pins the timing source to the granted generation', async () => {
    const h = fixture();
    expect(h.session.startMeasurement(500,7200000,5,true,true)).toBe(false);
    h.text(`saturn_ps_stimulus_caps:0,${TIMING_PROBE_VERSION};`);
    expect(h.session.status().timingSupported).toBe(true);
    expect(h.session.startMeasurement(500,7200000,5,false,true)).toBe(false);
    expect(h.session.startMeasurement(500,7200000,5,true,true)).toBe(true);
    expect(h.controls.at(-1)).toBe(`saturn_ps_measurement:0,start,500,${TIMING_PROBE_VERSION};`);
    expect(h.timingCapture).not.toHaveBeenCalled();
    h.offer(); await settle(); h.input();
    expect(h.timingCapture).toHaveBeenCalledOnce(); expect(h.microphone).not.toHaveBeenCalled(); expect(h.media).toHaveLength(1);
    h.session.release(); h.input(); expect(h.media).toHaveLength(1); expect(h.close).toHaveBeenCalledOnce();
    h.session.disconnect(); expect(h.session.status().timingSupported).toBe(false);
    h.offer(); await settle(); expect(h.timingCapture).toHaveBeenCalledOnce();
  });
  it('withdrawn timing capability inhibits output without falling back', async () => {
    const h=fixture(); h.text(`saturn_ps_stimulus_caps:0,${TIMING_PROBE_VERSION};`);
    h.session.startMeasurement(500,7200000,5,true,true); h.offer(); await settle();
    h.text('saturn_ps_stimulus_caps:0,unknown;'); h.input();
    expect(h.media).toEqual([]); expect(h.close).toHaveBeenCalledOnce(); expect(h.session.status().phase).toBe('fault');
  });
  it('negotiation never starts RF, microphone or generated audio', () => {
    const h = fixture(); expect(h.controls).toEqual(['tx_codec_caps:0,pcm;']);
    expect(h.microphone).not.toHaveBeenCalled(); expect(h.media).toEqual([]);
    expect(h.session.status().psMode).toBe('off');
  });
  it('single calibration is a settings transaction, never an arm', () => {
    const h = fixture('single'); expect(h.session.preparePureSignal(true)).toBe(true);
    expect(h.controls.at(-1)).toBe('tx_puresignal:0,true;');
    expect(h.controls).not.toContain('trx:0,true;'); expect(h.media).toEqual([]);
    expect(h.session.status().psMode).toBe('off');
    h.text('tx_config:0,1,queued,12;tx_config:0,1,applied,12;');
    expect(h.session.status().phase).not.toBe('ready');
    h.text('saturn_tx_owner:shared,2,ReadyOff,3,12,tci,true,none;saturn_ps_mode:0,12,single;');
    expect(h.session.status().phase).toBe('ready'); expect(h.session.status().psMode).toBe('single');
    h.text('saturn_ps_mode:0,11,off;'); expect(h.session.status().psMode).toBe('single');
  });
  it('retains the actual host settings rejection and does not claim calibration', () => {
    const h = fixture('single'); h.session.preparePureSignal(true);
    h.text('tx_config:0,1,rejected,InvalidConfig,0;');
    expect(h.session.status().reason).toContain('InvalidConfig'); expect(h.session.status().psMode).toBe('off');
  });
  it('measurement-only cannot enable fitting or ordinary PTT', () => {
    const h = fixture(); expect(h.session.preparePureSignal(true)).toBe(false);
    expect(h.session.arm('tci')).toBe(false); expect(h.controls).toEqual(['tx_codec_caps:0,pcm;']);
    expect(h.session.preparePureSignal(false)).toBe(true);
  });
  it.each([[500, 7200000, 5, false], [1001, 7200000, 5, true], [500, 14200000, 5, true],
    [500, 7200000, 6, true], [NaN, 7200000, 5, true]] as const)('refuses invalid/unattended request %j', (...args) => {
    const h = fixture();
    const [duration, frequency, watts, attended] = args as [number, number, number, boolean];
    expect(h.start(duration, frequency, watts, attended)).toBe(false); expect(h.media).toEqual([]);
    expect(h.controls).toEqual(['tx_codec_caps:0,pcm;']);
  });
  it('requires fresh operator, RF permission, ready settings and TCI source', () => {
    for (const change of ['tx_enable:0,false;', 'saturn_tx_owner:shared,2,ReadyOff,3,11,satp,true,none;',
      'saturn_ps_control_caps:0,single;', 'saturn_ps_mode:0,12,single;', 'saturn_ps_measurement_caps:0,false;']) {
      const h = fixture(); h.text(change); expect(h.start()).toBe(false);
    }
    const h = fixture(); h.session.setOperator(false); expect(h.start()).toBe(false);
  });
  it('streams only generated tone after the fresh host generation, then exports after completion', async () => {
    const h = fixture(); expect(h.start()).toBe(true); expect(h.media).toEqual([]);
    expect(h.controls.at(-1)).toBe('saturn_ps_measurement:0,start,500;');
    h.text('saturn_ps_measurement:0,1,queued;'); h.offer(); await settle(); h.input();
    expect(h.microphone).not.toHaveBeenCalled(); expect(h.media).toHaveLength(1);
    expect(new DataView(h.media[0]!).getUint32(60, true)).toBe(0x50534731);
    h.text('saturn_tx_owner:shared,2,ReadyOff,3,11,tci,true,none;');
    expect(h.session.status().phase).not.toBe('ready'); expect(h.start()).toBe(false);
    h.text('saturn_ps_measurement:0,1,complete,3,none;');
    expect(h.controls.at(-1)).toBe('saturn_ps_measurement:0,export,1;');
    h.text('saturn_ps_measurement:0,1,exported,/captures/measurement-1-1.json;');
    expect(h.session.status().phase).toBe('ready'); expect(h.session.status().measurementResult).toContain('RF/IMD unqualified');
    expect(h.session.status().measurementResult).toContain('/captures/measurement-1-1.json');
  });
  it('finite duration is a normal stop, not a fault or completed cleanup', async () => {
    const h = fixture(); h.start(); h.offer(); await settle(); h.input(); h.advance(500);
    expect(h.close).toHaveBeenCalledOnce(); expect(h.controls.at(-1)).toBe('saturn_ps_measurement:0,stop;');
    expect(h.session.status().phase).toBe('releasing'); expect(h.session.status().measurementActive).toBe(true);
    h.advance(100); expect(h.controls.filter(c => c === 'saturn_ps_measurement:0,stop;')).toHaveLength(1);
    h.advance(15001); expect(h.session.status().phase).toBe('fault');
  });
  it('release cancels queued and retained tone output; stale callbacks do not re-arm', async () => {
    const h = fixture(); h.start(); h.offer(); await settle(); h.session.release(); h.input();
    expect(h.media).toEqual([]); expect(h.close).toHaveBeenCalledOnce();
    h.text(`saturn_tx_generation:psg1,${'f'.repeat(32)},2;`); await settle();
    expect(h.media).toEqual([]); expect(h.start()).toBe(false);
  });
  it('disconnect invalidates generated output and old replies', async () => {
    const h = fixture(); h.start(); h.offer(); await settle(); h.session.disconnect(); h.input();
    h.text('saturn_ps_measurement:0,1,complete,5,none;');
    expect(h.media).toEqual([]); expect(h.controls.some(c => c.includes('export'))).toBe(false);
    expect(h.session.status().psCapability).toBe('unavailable');
  });
  it('RF permission loss and queue faults close tone output', async () => {
    const h = fixture(); h.start(); h.offer(); await settle(); h.text('tx_enable:0,false;'); h.input();
    expect(h.media).toEqual([]); expect(h.session.status().phase).toBe('fault');
  });
  it('zero paired blocks cannot become a successful measurement', () => {
    const h = fixture(); h.start(); h.text('saturn_ps_measurement:0,1,queued;saturn_ps_measurement:0,1,complete,0,none;');
    expect(h.session.status().phase).toBe('fault');
    expect(h.controls.at(-1)).toBe('saturn_ps_measurement:0,export,1;');
  });
  it('summary-only permits a longer stimulus without pretending raw samples exist', () => {
    const h = fixture('measurement-only', 'summary-only'); expect(h.start(45000)).toBe(true);
    expect(h.session.status().measurementCaptureMode).toBe('summary-only');
    expect(h.session.status().measurementLimitMs).toBe(45000);
  });
});
describe('generated two-tone provenance', () => {
  it('timing delivers the exact prepared record with one-second repetition, fresh re-arm and cancellation', async () => {
    vi.useFakeTimers();
    try {
      const expected=createTimingProbePcm(1000).samples;
      const deliver=vi.fn(),failed=vi.fn();
      const factory=twoToneManagedCapture({now:()=>Date.now(),later:setTimeout,cancel:clearTimeout},()=> 'timing');
      const capture=await factory(TOKEN,deliver,failed);
      vi.advanceTimersByTime(1040);
      const all=Float32Array.from(deliver.mock.calls.flatMap(([s])=>Array.from(s as Float32Array)));
      expect(all.length).toBeGreaterThan(48000); expect(all.slice(0,48000)).toEqual(expected);
      expect(all.slice(48000)).toEqual(expected.slice(0,all.length-48000));
      expect(failed).not.toHaveBeenCalled(); capture.close(); const count=deliver.mock.calls.length;
      vi.advanceTimersByTime(100); expect(deliver).toHaveBeenCalledTimes(count);
      const second=vi.fn(), fresh=await factory('f'.repeat(32),second,failed);
      vi.advanceTimersByTime(2); expect(second.mock.calls[0]![0]).toEqual(expected.slice(0,512));fresh.close();
    } finally {vi.useRealTimers();}
  });
  it('AM carrier sends exact silence with fresh timestamps, not microphone/DC audio, and snapshots its waveform', async () => {
    vi.useFakeTimers();
    try {
      let waveform: 'two-tone' | 'am-carrier' = 'am-carrier';
      const deliver=vi.fn(), failed=vi.fn();
      const factory=twoToneManagedCapture({now:()=>Date.now(),later:setTimeout,cancel:clearTimeout},()=>waveform);
      const capture=await factory(TOKEN,deliver,failed);
      waveform='two-tone';
      vi.advanceTimersByTime(40);
      expect(deliver.mock.calls.length).toBeGreaterThan(2);
      for(const [samples,observed] of deliver.mock.calls) {
        expect(samples).toHaveLength(512); expect([...samples].every(v=>v===0)).toBe(true);
        expect(Number.isFinite(observed)).toBe(true);
      }
      expect(deliver.mock.calls.at(-1)![1]).toBeGreaterThan(deliver.mock.calls[0]![1]);
      const count=deliver.mock.calls.length;capture.close();vi.advanceTimersByTime(1000);
      expect(deliver).toHaveBeenCalledTimes(count);expect(failed).not.toHaveBeenCalled();
    } finally {vi.useRealTimers();}
  });
  it('uses fresh phase, exact tones and <=0.4 peak, and closed callbacks cannot deliver', async () => {
    vi.useFakeTimers();
    try {
      const deliver = vi.fn(), failed = vi.fn();
      const factory = twoToneManagedCapture({ now: () => Date.now(), later: setTimeout, cancel: clearTimeout });
      const capture = await factory(TOKEN, deliver, failed);
      vi.advanceTimersByTime(1); expect(deliver).toHaveBeenCalledOnce();
      const block = deliver.mock.calls[0]![0] as Float32Array;
      expect(block.length).toBe(512);
      for (let i = 0; i < block.length; i++)
        expect(block[i]).toBeCloseTo(.2 * (Math.sin(2 * Math.PI * 700 * i / 48000) + Math.sin(2 * Math.PI * 1900 * i / 48000)), 6);
      expect(Math.max(...block.map(Math.abs))).toBeLessThanOrEqual(.4000001); // float32 rounding
      capture.close(); vi.advanceTimersByTime(1000); expect(deliver).toHaveBeenCalledOnce(); expect(failed).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });
  it.each(['two-tone','timing','am-carrier'] as const)('a browser stall in %s PCM fails without a catch-up burst', async waveform => {
    let now = 0, pending!: () => void;
    const deliver = vi.fn(), failed = vi.fn();
    const capture = await twoToneManagedCapture({ now: () => now,
      later: callback => { pending = callback; return 1 as unknown as ReturnType<typeof setTimeout>; }, cancel: vi.fn() },()=>waveform)(TOKEN, deliver, failed);
    now = 100; pending(); expect(deliver).not.toHaveBeenCalled(); expect(failed).toHaveBeenCalledOnce();
    capture.close(); pending(); expect(failed).toHaveBeenCalledOnce();
  });
  it('native keepalive resumes with one fresh zero block, without replay or a catch-up burst', async () => {
    let now=0, pending!:()=>void, delay=0;
    const deliver=vi.fn(),failed=vi.fn();
    const capture=await twoToneManagedCapture({now:()=>now,later:(callback,ms)=>{
      pending=callback;delay=ms;return 1 as unknown as ReturnType<typeof setTimeout>;
    },cancel:vi.fn()},()=> 'keepalive')(TOKEN,deliver,failed);
    now=100;pending();expect(failed).not.toHaveBeenCalled();expect(deliver).toHaveBeenCalledOnce();
    expect(deliver.mock.calls[0]![1]).toBe(100);
    expect([...deliver.mock.calls[0]![0] as Float32Array].every(v=>v===0)).toBe(true);
    expect(delay).toBeCloseTo(512/48);
    now=300;pending();expect(deliver).toHaveBeenCalledTimes(2);expect(delay).toBeCloseTo(512/48);
    expect(deliver.mock.calls[1]![1]).toBe(300);
    capture.close();now=320;pending();expect(deliver).toHaveBeenCalledTimes(2);
  });
  it('an expired native keepalive closes and cannot be revived by its queued callback', async () => {
    let now=0,pending!:()=>void;
    const deliver=vi.fn(),failed=vi.fn();
    const capture=await twoToneManagedCapture({now:()=>now,later:callback=>{
      pending=callback;return 1 as unknown as ReturnType<typeof setTimeout>;
    },cancel:vi.fn()},()=> 'keepalive')(TOKEN,deliver,failed);
    now=1;pending();expect(deliver).toHaveBeenCalledOnce();
    now=502;pending();expect(failed).toHaveBeenCalledWith('Native generator keepalive expired; no catch-up burst');
    now=503;pending();expect(deliver).toHaveBeenCalledOnce();expect(failed).toHaveBeenCalledOnce();capture.close();
  });
});
