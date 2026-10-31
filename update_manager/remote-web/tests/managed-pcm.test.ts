import { describe, expect, it, vi } from 'vitest';
import { ManagedPcmSession, managedSettingsMessage, type ManagedCaptureFactory } from '../src/transport/managed-pcm';
const TOKEN = '00112233445566778899aabbccddeeff';
function setup(capture?: ManagedCaptureFactory) {
  let now = 100, deliver!: (s: Float32Array, t: number) => void;
  const control: string[] = [], media: ArrayBuffer[] = [], closed = vi.fn();
  const session = new ManagedPcmSession({now: () => now, changed: vi.fn(),
    capture: capture ?? (async (token, callback) => {
      deliver = callback; return {token, historyResetAtMs: now, close: closed};
    })});
  session.select(true);
  const epoch = session.connect(s => control.push(s), f => media.push(f));
  session.setOperator(true);
  function ready(source = 'tci', serial = 1) {
    session.text(`saturn_tx_media_caps:psg1,pcm,owner1,transactions1;tx_codec_accept:0,pcm;ready;saturn_tx_owner:shared,${serial},ReadyOff,2,11,${source},true,none;`, epoch);
  }
  function offer(token = TOKEN, serial = 1) {
    session.text(`saturn_tx_generation:psg1,${token},${serial};`, epoch);
  }
  return {session, epoch, control, media, closed, ready, offer,
    advance: (ms: number) => { now += ms; session.tick(); },
    input: (age = 0, values = new Float32Array(512).fill(0.2)) => deliver(values, now - age)};
}
const settle = async () => { await Promise.resolve(); await Promise.resolve(); };
describe('selectable managed PCM controller', () => {
  it('scopes manual feedback-attenuation support to the negotiated connection', () => {
    const h=setup();h.ready();
    expect(h.session.status().manualAttenuationSupported).toBe(false);
    h.session.text('saturn_ps_manual_attenuation_caps:0,rx-off-ordered-v1;',h.epoch);
    expect(h.session.status().manualAttenuationSupported).toBe(true);
    h.session.connect(s=>h.control.push(s),f=>h.media.push(f));
    expect(h.session.status().manualAttenuationSupported).toBe(false);
    h.session.text('saturn_ps_manual_attenuation_caps:0,rx-off-ordered-v1;',h.epoch);
    expect(h.session.status().manualAttenuationSupported).toBe(false);
    expect(h.media).toHaveLength(0);
  });
  it('backpressures an 18-message settings burst to one wire transaction and commits all in order', () => {
    const h = setup(); h.ready();
    const expected = Array.from({length:18}, (_,i) => `tx_mic_gain:0,${-i};`);
    for (const command of expected) h.session.sendControl(command);
    expect(h.control.slice(1)).toEqual(expected.slice(0,1));
    expect(h.session.status().pendingSettings).toBe(18);
    for (let id=1; id<=18; id++) {
      h.session.text(`tx_config:0,${id},queued,${11+id};tx_config:0,${id},applied,${11+id};`, h.epoch);
      h.advance(1);
      expect(h.control.slice(1)).toHaveLength(id); // stale owner cannot drain
      h.session.text(`saturn_tx_owner:shared,${id+1},ReadyOff,${id+2},${11+id},tci,true,none;`, h.epoch);
      h.advance(1);
    }
    expect(h.control.slice(1)).toEqual(expected);
    expect(h.session.status().pendingSettings).toBe(0);
    expect(h.session.status().phase).toBe('ready');
    expect(h.media).toHaveLength(0);
  });
  it('requires an explicit current-stop acknowledgment result, never keys, and discards old settings', () => {
    const h=setup(); h.ready();
    h.session.text('saturn_tx_fault_ack_caps:0,stop-fence-v1;saturn_tx_owner:shared,2,ReadyOff,3,11,tci,false,QueueOverrun;saturn_tx_fault_ack_state:0,3,true;',h.epoch);
    h.session.sendControl('tx_mic_gain:0,-4;');
    expect(h.control).not.toContain('tx_mic_gain:0,-4;');
    expect(h.session.status().phase).toBe('fault');
    expect(h.session.status().canAcknowledgeFault).toBe(true);
    expect(h.session.acknowledgeFault()).toBe(true);
    expect(h.control.at(-1)).toBe('saturn_tx_fault_ack:0,1,3;');
    h.session.text('saturn_tx_fault_ack:0,99,applied,none,3;',h.epoch);
    expect(h.session.status().phase).toBe('fault');
    h.session.text('saturn_tx_owner:shared,3,ReadyOff,3,11,tci,true,none;saturn_tx_fault_ack_state:0,3,false;saturn_tx_fault_ack:0,1,applied,none,3;',h.epoch);
    expect(h.session.status().phase).toBe('ready');
    expect(h.control).not.toContain('trx:0,true;'); expect(h.media).toHaveLength(0);
    expect(h.session.arm('tci')).toBe(true); // separate fresh action
  });
  it.each(['release','disconnect','stale-stop','rejected','no-commit'])('cannot recover from a stale or cancelled fault acknowledgment: %s', how => {
    const h=setup(); h.ready();
    h.session.text('saturn_tx_fault_ack_caps:0,stop-fence-v1;saturn_tx_owner:shared,2,ReadyOff,3,11,tci,false,QueueOverrun;saturn_tx_fault_ack_state:0,3,true;',h.epoch);
    expect(h.session.acknowledgeFault()).toBe(true);
    if(how==='release') h.session.release();
    if(how==='disconnect') h.session.disconnect();
    if(how==='stale-stop') h.session.text('saturn_tx_owner:shared,3,ReadyOff,4,11,tci,true,none;',h.epoch);
    h.session.text(`saturn_tx_fault_ack:0,1,${how==='rejected'?'rejected,CleanupPending':'applied,none'},3;`,h.epoch);
    expect(h.session.arm('tci')).toBe(false); expect(h.media).toHaveLength(0);
  });
  it('bounds a tuning burst to one transaction and the latest unsent absolute target', () => {
    const h = setup(); h.ready();
    for (let i = 0; i < 1000; i++) h.session.sendControl(`vfo:0,0,${7200000+i};dds:0,${7200000+i};`);
    expect(h.control.filter(c => c.startsWith('vfo:'))).toEqual(['vfo:0,0,7200000;dds:0,7200000;']);
    expect(h.session.status().pendingSettings).toBe(2);
    for (let i=0; i<100; i++) h.session.sendControl(`saturn_ping:${i},100;rx_smeter:0,0;`);
    expect(h.control.filter(c => c.startsWith('vfo:'))).toHaveLength(1);
    h.session.text('tx_config:0,1,queued,12;tx_config:0,1,applied,12;', h.epoch);
    h.advance(1); // an old owner snapshot cannot release the next target
    expect(h.control.filter(c => c.startsWith('vfo:'))).toHaveLength(1);
    h.session.text('saturn_tx_owner:shared,2,ReadyOff,2,12,tci,true,none;', h.epoch);
    h.advance(1);
    expect(h.control.at(-1)).toBe('vfo:0,0,7200999;dds:0,7200999;');
    h.session.text('tx_config:0,2,queued,13;saturn_tx_owner:shared,3,ReadyOff,2,13,tci,true,none;tx_config:0,2,applied,13;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(0);
    expect(h.session.status().phase).toBe('ready');
    expect(h.control).not.toContain('trx:0,true;');
  });
  it('never coalesces tuning across settings barriers or different VFO routes', () => {
    const h = setup(); h.ready();
    h.session.sendControl('vfo:0,0,7200000;dds:0,7200000;');
    h.session.sendControl('vfo:0,0,7200100;dds:0,7200100;');
    h.session.sendControl('tx_eq_enable:0,true;');
    h.session.sendControl('vfo:0,0,7200200;');
    h.session.sendControl('vfo:0,1,7200300;');
    expect(h.control).toHaveLength(2);
    for (let id = 1; id <= 5; id++) {
      h.session.text(`tx_config:0,${id},queued,${11+id};saturn_tx_owner:shared,${id+1},ReadyOff,${id+2},${11+id},tci,true,none;tx_config:0,${id},applied,${11+id};`, h.epoch);
      h.advance(1);
    }
    expect(h.control.slice(1)).toEqual(['vfo:0,0,7200000;dds:0,7200000;', 'vfo:0,0,7200100;dds:0,7200100;',
      'tx_eq_enable:0,true;', 'vfo:0,0,7200200;', 'vfo:0,1,7200300;']);
    expect(h.session.status().pendingSettings).toBe(0);
  });
  it('does not rewind the visible VFO from an older echo while a later target is retained', () => {
    const h = setup(); h.ready();
    h.session.sendControl('vfo:0,0,7200000;dds:0,7200000;');
    h.session.sendControl('vfo:0,0,7201000;dds:0,7201000;');
    expect(h.session.filterTuningEcho('vfo:0,0,7200000;dds:0,7200000;vfo:0,1,7100000;ready;'))
      .toBe('vfo:0,1,7100000;ready;');
    expect(h.session.filterTuningEcho('vfo:0,0,7201000;dds:0,7201000;'))
      .toBe('vfo:0,0,7201000;dds:0,7201000;');
    // Returning to a previously used frequency does not timestamp an echo.
    expect(h.session.filterTuningEcho('vfo:0,0,7200000;dds:0,7200000;')).toBe('');
    h.session.release();
    expect(h.session.filterTuningEcho('vfo:0,0,7100000;')).toBe('vfo:0,0,7100000;');
  });
  it.each(['release', 'disconnect', 'fault', 'wire release'])('discards unsent tuning on %s without replaying it', how => {
    const h = setup(); h.ready();
    h.session.sendControl('vfo:0,0,7200000;');
    h.session.sendControl('vfo:0,0,7200100;');
    if (how === 'release') h.session.release();
    else if (how === 'disconnect') h.session.disconnect();
    else if (how === 'wire release') h.session.sendControl('trx:0,false;');
    else h.session.text('saturn_tx_owner:shared,2,ReadyOff,2,11,tci,true,QueueOverrun;', h.epoch);
    h.advance(1);
    expect(h.control).not.toContain('vfo:0,0,7200100;');
  });
  it('sends only one fault release through repeated owner reports and deadline ticks', () => {
    const h = setup(); h.ready();
    h.session.sendControl('tx_mic_gain:0,-3;');
    h.session.text('tx_config:0,1,rejected,InvalidConfig,0;tx_fault:0,shared_config,InvalidConfig;', h.epoch);
    for (let serial=2; serial<102; serial++) {
      h.session.text(`saturn_tx_owner:shared,${serial},ReadyOff,${serial},11,tci,true,QueueOverrun;`, h.epoch);
      h.advance(100);
    }
    expect(h.control.filter(c => c === 'trx:0,false;')).toHaveLength(1);
    expect(h.session.status().reason).toContain('InvalidConfig');
    expect(h.session.arm('tci')).toBe(false);
    expect(h.media).toHaveLength(0);
  });
  it('defaults off and cannot authorize TX or advertise a codec', () => {
    const send = vi.fn();
    const s = new ManagedPcmSession({now: () => 0, capture: vi.fn(), changed: vi.fn()});
    s.connect(send, vi.fn()); expect(send).not.toHaveBeenCalled(); expect(s.arm('tci')).toBe(false);
    expect(s.status().selected).toBe(false);
  });
  it('requires capability contract, explicit PCM acceptance, owner cleanup and operator', () => {
    const h = setup(); expect(h.control).toEqual(['tx_codec_caps:0,pcm;']);
    expect(h.session.arm('tci')).toBe(false);
    h.ready(); expect(h.session.status().phase).toBe('ready');
    h.session.setOperator(false); expect(h.session.arm('tci')).toBe(false);
    h.session.setOperator(true); expect(h.session.arm('tci')).toBe(true);
  });
  it('labels older/legacy contracts unsupported without arming or falling back', () => {
    const h = setup(); h.session.text('saturn_tx_media_caps:psg1,pcm,opus_wb,bank4;ready;tx_codec_accept:0,pcm;', h.epoch);
    expect(h.session.status().phase).toBe('unsupported'); expect(h.session.arm('tci')).toBe(false);
    expect(h.control).toEqual(['tx_codec_caps:0,pcm;', 'trx:0,false;']);
  });
  it('times out an absent managed contract visibly', () => {
    const h = setup(); h.advance(5001); expect(h.session.status().phase).toBe('unsupported');
  });
  it('shows unavailable operator status and requires fresh authorization after role arrival', () => {
    const h = setup(); h.session.setOperator(false); h.ready();
    expect(h.session.status().phase).toBe('unavailable'); expect(h.session.arm('tci')).toBe(false);
    h.session.setOperator(true); expect(h.session.status().phase).toBe('ready');
    expect(h.control).not.toContain('trx:0,true;');
  });
  it('capability loss during capture closes it and never silently falls back', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); h.offer(); await settle();
    h.session.text('saturn_tx_media_caps:pcm;', h.epoch); h.input();
    expect(h.closed).toHaveBeenCalledOnce(); expect(h.media).toHaveLength(0);
    expect(h.session.status().phase).toBe('unsupported');
  });
  it('never reuses the old clean stop receipt after local release', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); h.offer(); await settle();
    h.session.release(); h.session.text('saturn_tx_owner:shared,2,ReadyOff,2,11,tci,true,none;', h.epoch);
    expect(h.session.arm('tci')).toBe(false);
    h.session.text('saturn_tx_owner:shared,3,ReadyOff,3,11,tci,true,none;', h.epoch);
    expect(h.session.arm('tci')).toBe(true);
  });
  it('a genuinely unsolicited generation remains an authorization fault', () => {
    const h = setup(); h.ready(); h.offer(); expect(h.session.status().phase).toBe('fault');
    expect(h.media).toHaveLength(0);
  });
  it('rejects accepted Opus instead of silently switching codec', () => {
    const h = setup(); h.ready(); h.session.text('tx_codec_accept:0,opus_wb;', h.epoch);
    expect(h.session.status().phase).toBe('fault'); expect(h.control.at(-1)).toBe('trx:0,false;');
  });
  it('requires fresh generation, stamps PCM with token and zero sequence, closes on release', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); expect(h.media).toHaveLength(0);
    h.offer(); await settle(); h.input();
    const view = new DataView(h.media[0]!);
    expect(view.getUint32(60, true)).toBe(0x50534731); expect(view.getUint32(32, true)).toBe(0);
    expect(view.getUint32(20, true)).toBe(512);
    h.session.release(); h.input(); expect(h.media).toHaveLength(1); expect(h.closed).toHaveBeenCalledOnce();
    expect(h.session.arm('tci')).toBe(false); // cleanup receipt still missing
    h.session.text('saturn_tx_owner:shared,2,ReadyOff,3,11,tci,true,none;', h.epoch);
    expect(h.session.arm('tci')).toBe(true);
  });
  it('drops a late capture acquisition after release, never granting new authority', async () => {
    let complete!: (v: {token: string; historyResetAtMs: number; close(): void}) => void;
    const close = vi.fn(), h = setup(() => new Promise(resolve => { complete = resolve; }));
    h.ready(); h.session.arm('tci'); h.offer(); h.session.release();
    complete({token: TOKEN, historyResetAtMs: 100, close}); await settle();
    expect(close).toHaveBeenCalledOnce(); expect(h.media).toHaveLength(0);
    expect(h.session.status().phase).not.toBe('streaming');
  });
  it('inhibits an in-transit offer after release without a false unsolicited-generation fault', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); h.offer(); await settle();
    h.session.release(); h.offer('11223344556677889900aabbccddeeff', 2); await settle();
    h.input(); expect(h.media).toHaveLength(0); expect(h.session.status().phase).toBe('releasing');
    h.session.text('saturn_tx_owner:shared,2,ReadyOff,3,11,tci,true,none;', h.epoch);
    expect(h.session.arm('tci')).toBe(true);
  });
  it('requires an owner commit matching applied settings, not an old clean snapshot', () => {
    const h = setup(); h.ready(); h.session.sendControl('tx_eq_enable:0,true;');
    h.session.text('tx_config:0,1,queued,12;tx_config:0,1,applied,12;', h.epoch);
    expect(h.session.arm('tci')).toBe(false);
    h.session.text('saturn_tx_owner:shared,2,ReadyOff,3,12,tci,true,none;', h.epoch);
    expect(h.session.arm('tci')).toBe(true);
  });
  it('preserves sample age, refuses a queued old block and does not drop-and-continue', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); h.offer(); await settle();
    h.advance(151); h.input(151); expect(h.session.status().phase).toBe('fault');
    h.input(); expect(h.media).toHaveLength(0);
  });
  it('stops capture when periodic ticks see no microphone packets', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); h.offer(); await settle(); h.advance(501);
    expect(h.session.status().phase).toBe('fault'); expect(h.closed).toHaveBeenCalledOnce();
  });
  it('tracks separate settings messages through queued/applied and waits for clean owner', () => {
    const h = setup(); h.ready();
    h.session.sendControl('tx_mic_gain:0,-3;'); h.session.sendControl('tx_eq_enable:0,true;');
    expect(h.session.status().pendingSettings).toBe(2); expect(h.session.arm('tci')).toBe(false);
    expect(h.control).not.toContain('tx_eq_enable:0,true;');
    h.session.text('tx_config:0,1,queued,12;', h.epoch);
    h.session.text('saturn_tx_owner:shared,2,CleaningNative,4,11,tci,false,none;tx_config:0,1,applied,12;', h.epoch);
    expect(h.session.arm('tci')).toBe(false);
    h.session.text('saturn_tx_owner:shared,3,ReadyOff,4,12,tci,true,none;', h.epoch);
    h.advance(1);
    expect(h.control.at(-1)).toBe('tx_eq_enable:0,true;');
    h.session.text('tx_config:0,2,queued,13;saturn_tx_owner:shared,4,ReadyOff,5,13,tci,true,none;tx_config:0,2,applied,13;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(0); expect(h.session.arm('tci')).toBe(true);
    expect(h.session.status().lastTransaction).toContain('Settings 2: applied');
  });
  it('shows rejected settings reason and keeps output inhibited', () => {
    const h = setup(); h.ready(); h.session.sendControl('tx_mic_gain:0,-3;');
    h.session.text('tx_config:0,1,queued,12;tx_config:0,1,rejected,InvalidConfig,12;', h.epoch);
    h.session.release(); expect(h.session.status().reason).toContain('InvalidConfig');
    expect(h.session.arm('tci')).toBe(false);
  });
  it('completes a pre-queue rejection and retains the Bridge reason past the acknowledgment deadline', () => {
    const h = setup(); h.ready(); h.session.sendControl('tx_drive:0,6;');
    h.session.text('tx_config:0,1,rejected,InvalidConfig,0;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(0);
    expect(h.session.status().lastTransaction).toContain('rejected (InvalidConfig)');
    expect(h.session.status().reason).toContain('InvalidConfig');
    h.advance(5001);
    expect(h.session.status().reason).toContain('InvalidConfig');
    expect(h.session.arm('tci')).toBe(false);
  });
  it('rejects one transmitted request and cancels unsent settings without replay after a fault', () => {
    const h = setup(); h.ready();
    h.session.sendControl('tx_drive:0,6;'); h.session.sendControl('tx_mic_gain:0,-3;');
    h.session.text('tx_config:0,1,rejected,InvalidConfig,0;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(0);
    const rejected = h.session.status().lastTransaction;
    h.session.text('tx_config:0,1,rejected,InvalidConfig,0;tx_config:0,1,queued,12;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(0);
    expect(h.session.status().lastTransaction).toBe(rejected);
    h.session.sendControl('tx_mic_gain:0,-9;'); h.advance(1);
    expect(h.control).not.toContain('tx_mic_gain:0,-3;');
    expect(h.control).not.toContain('tx_mic_gain:0,-9;');
    expect(h.session.status().reason).toContain('InvalidConfig');
    expect(h.session.arm('tci')).toBe(false); // rejection does not grant new authority
  });
  it('does not match unsolicited or non-prequeue terminal results to a request awaiting acknowledgment', () => {
    for (const reply of ['tx_config:0,1,rejected,InvalidConfig,12;', 'tx_config:0,1,rejected,0;',
      'tx_config:0,1,applied,0;']) {
      const h = setup(); h.ready(); h.session.sendControl('tx_drive:0,6;');
      h.session.text(reply, h.epoch);
      expect(h.session.status().pendingSettings).toBe(1);
      expect(h.session.status().reason).toBe('Unmatched settings completion');
    }
    const h = setup(); h.ready(); h.session.text('tx_config:0,1,rejected,InvalidConfig,0;', h.epoch);
    expect(h.session.status().reason).toBe('Unmatched settings completion');
    expect(h.session.status().lastTransaction).toBe('No managed settings result');
  });
  it('cannot consume a fresh request with an old result after the bounded terminal cache evicts it', () => {
    const h = setup(); h.ready();
    for (let id = 1; id <= 65; id++) {
      h.session.sendControl('tx_mic_gain:0,-3;');
      h.session.text(`tx_config:0,${id},queued,${11+id};saturn_tx_owner:shared,${id+1},ReadyOff,${id+2},${11+id},tci,true,none;tx_config:0,${id},applied,${11+id};`, h.epoch);
      expect(h.session.status().pendingSettings).toBe(0);
    }
    h.session.sendControl('tx_mic_gain:0,-3;');
    h.session.text('tx_config:0,1,rejected,InvalidConfig,0;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(1);
    expect(h.session.status().lastTransaction).toContain('Settings 65: applied');
    h.session.text('tx_config:0,66,queued,77;tx_config:0,66,applied,77;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(0);
  });
  it('ignores an old-session prequeue rejection without consuming the new-session request', () => {
    const h = setup(); h.ready(); h.session.sendControl('tx_drive:0,6;');
    const epoch = h.session.connect(s => h.control.push(s), f => h.media.push(f));
    h.session.sendControl('tx_mic_gain:0,-3;');
    h.session.text('tx_config:0,1,rejected,InvalidConfig,0;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(1);
    expect(h.session.status().lastTransaction).toBe('No managed settings result');
    h.session.text('tx_config:0,2,queued,12;tx_config:0,2,applied,12;', epoch);
    expect(h.session.status().pendingSettings).toBe(0);
  });
  it('allows an older queued transaction to complete after a newer initial result', () => {
    const h = setup(); h.ready();
    h.session.sendControl('tx_mic_gain:0,-3;'); h.session.sendControl('tx_eq_enable:0,true;');
    h.session.text('tx_config:0,1,queued,12;tx_config:0,2,queued,13;', h.epoch);
    h.session.text('tx_config:0,2,applied,13;tx_config:0,1,rejected,GenerationMismatch,12;', h.epoch);
    expect(h.session.status().pendingSettings).toBe(0);
    expect(h.session.status().lastTransaction).toContain('Settings 1: rejected (GenerationMismatch)');
  });
  it('fails closed for missing, unmatched or malformed completion', () => {
    for (const reply of ['', 'tx_config:0,5,applied,12;', 'tx_config:bad;']) {
      const h = setup(); h.ready(); h.session.sendControl('tx_mic_gain:0,-3;');
      h.session.text(reply, h.epoch); h.advance(5001); expect(h.session.status().phase).toBe('fault');
    }
  });
  it('revokes capture on owner fault and does not acknowledge the host fault', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); h.offer(); await settle();
    h.session.text('saturn_tx_owner:shared,2,AwaitingAuthority,3,11,tci,false,TxAuthority;', h.epoch);
    h.input(); expect(h.media).toHaveLength(0); expect(h.session.status().phase).toBe('fault');
    expect(h.control.some(c => c.includes('ack'))).toBe(false);
  });
  it('ignores older offers, stale-session settings results and callbacks after reconnect', async () => {
    const h = setup(); h.ready(); h.session.arm('tci'); h.offer(TOKEN, 2); await settle();
    h.session.text('saturn_tx_generation:psg1,off,1;', h.epoch); h.input(); expect(h.media).toHaveLength(1);
    const epoch = h.session.connect(s => h.control.push(s), f => h.media.push(f));
    h.session.text('tx_config:0,4,applied,14;', h.epoch); h.input();
    expect(h.media).toHaveLength(1); expect(epoch).not.toBe(h.epoch); expect(h.session.status().phase).toBe('negotiating');
  });
  it('retains SATP with no browser microphone or parallel mic path', () => {
    const capture = vi.fn(), h = setup(capture); h.ready('satp');
    expect(h.session.arm('tci')).toBe(false); expect(h.session.arm('satp')).toBe(true);
    h.session.text('saturn_tx_owner:shared,2,Active,2,11,satp,false,none;', h.epoch);
    expect(h.session.status().phase).toBe('streaming'); expect(capture).not.toHaveBeenCalled();
    h.session.release(); expect(h.control.at(-1)).toBe('trx:0,false;');
  });
  it('does not mistake read-only settings queries and heartbeat/stats for transactions', () => {
    for (const c of ['vfo:0,0;', 'dds:0;', 'modulation:0;', 'tx_mic_gain:0;', 'tx_codec_caps:0,pcm;', 'tx_uplink_stats:0,false,1,0,0,0;'])
      expect(managedSettingsMessage(c)).toBe(false);
    for (const c of ['vfo:0,0,7200000;', 'dds:0,7200000;', 'rx_antenna:0,2;', 'rx_adc:1;', 'rx_attenuation:0,12;', 'iq_samplerate:384000;', 'tx_phase_rotator:0,true;', 'tx_puresignal:0,true;', 'tx_puresignal_reset:0;', 'saturn_satp_control:satp;'])
      expect(managedSettingsMessage(c)).toBe(true);
  });
  it('retains explicit unavailable RX-control evidence without inventing a transaction', () => {
    const h=setup(); h.ready();
    h.session.text('saturn_shared_control_reject:unsupported-control;',h.epoch);
    expect(h.session.status().lastTransaction).toBe('Host control unavailable: unsupported-control');
    expect(h.session.status().pendingSettings).toBe(0);
  });
});
