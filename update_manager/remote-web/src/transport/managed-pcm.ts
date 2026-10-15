// Selectable host integration, off by default. No Opus producer or legacy tee.
import { buildTxMicPcmS16Frame } from './tx-uplink';

export interface ManagedCapture {
  token: string;
  historyResetAtMs: number;
  close(): void;
}
export type ManagedCaptureFactory = (token: string,
  deliver: (samples: Float32Array, oldestMs: number) => void,
  failed: (reason: string) => void) => Promise<ManagedCapture>;
type Phase = 'disabled' | 'disconnected' | 'negotiating' | 'unsupported' | 'unavailable'
  | 'ready' | 'arming' | 'streaming' | 'releasing' | 'fault';
type Source = 'tci' | 'satp';
export interface ManagedPcmStatus {
  selected: boolean; phase: Phase; reason: string; pendingSettings: number;
  lastTransaction: string; source: Source; sentFrames: number;
  ownerPhase: string | null;
}
interface Owner { serial: bigint; stop: bigint; phase: string; config: string; source: Source; clean: boolean; fault: string }
interface Transaction { config: string; queuedAt: number }
const SETTINGS = /^(?:modulation|vfo|vfo_active|split|split_enable|drive|tx_drive|tx_mic_gain|tx_filter_band|tx_eq_enable|tx_eq_band|tx_cfc_enable|tx_cfc_precomp|tx_cfc_band|tx_phase_rotator|tx_phase_rotator_auto|tx_phase_rotator_corner|tx_noise_gate|tx_noise_gate_threshold|tx_dexp|tx_dexp_threshold|tx_dexp_expansion|tx_speech_processor|tx_speech_processor_gain|tx_cessb|tx_fft_size|tx_low_latency|tx_puresignal|tx_puresignal_auto_attenuate|tx_puresignal_attenuation|tx_puresignal_reset|tx_two_tone|tx_two_tone_freq1|tx_two_tone_freq2|tx_two_tone_level_db|tx_two_tone_invert_lsb|tx_two_tone_delay_ms):/;
export function managedSettingsMessage(text: string): boolean {
  return text.split(';').some(raw => {
    const c = raw.trim();
    // These geometry messages are setters even in their one-argument form.
    // The fixed-route host can reject before queueing; track that result too.
    if (/^(?:rx_antenna|rx_adc|rx_attenuation|iq_samplerate):[^;]+$/.test(c)) return true;
    if (/^dds:[^,]+,[^,]+$/.test(c)) return true;
    if (/^saturn_satp_control:(?:tci|satp)(?:,|$)/.test(c)) return true;
    if (!SETTINGS.test(c)) return false;
    const [name, args = ''] = c.split(':');
    if (name === 'tx_puresignal_reset') return true;
    return args.split(',').length >= (name === 'vfo' ? 3 : 2);
  });
}
function tag(frame: ArrayBuffer, token: string): ArrayBuffer {
  const view = new DataView(frame);
  for (let i = 0; i < 16; i++) view.setUint8(44 + i, parseInt(token.slice(i * 2, i * 2 + 2), 16));
  view.setUint32(60, 0x50534731, true);
  return frame;
}
export class ManagedPcmSession {
  private selected = false;
  private epoch = 0;
  private version = 0;
  private connectedAt = 0;
  private bridgeReady = false;
  private operator = false;
  private supported = false;
  private accepted = false;
  private owner: Owner | null = null;
  private offered = 0n;
  private token: string | null = null;
  private capture: ManagedCapture | null = null;
  private requested: Source | null = null;
  private deadline = 0;
  private sequence = 0;
  private sentFrames = 0;
  private lastInputAt = 0;
  private terminal = new Set<string>();
  private initialTransaction = 0n;
  private transactions = new Map<string, Transaction>();
  private awaitingQueue: number[] = [];
  private phase: Phase = 'disabled';
  private reason = 'Legacy TX selected';
  private lastTransaction = 'No managed settings result';
  private appliedConfig: string | null = null;
  private cleanupAfterStop: bigint | null = null;
  private control: ((text: string) => void) | null = null;
  private media: ((frame: ArrayBuffer) => void) | null = null;
  constructor(private readonly options: { now(): number; capture: ManagedCaptureFactory;
    changed(status: ManagedPcmStatus): void }) {}
  status(): ManagedPcmStatus {
    return { selected: this.selected, phase: this.phase, reason: this.reason,
      pendingSettings: this.awaitingQueue.length + this.transactions.size,
      lastTransaction: this.lastTransaction, source: this.owner?.source ?? 'tci', sentFrames: this.sentFrames,
      ownerPhase: this.owner?.phase ?? null };
  }
  private publish(): void { this.options.changed(this.status()); }
  private invalidate(): void {
    this.version++; this.token = null; this.sequence = 0;
    const old = this.capture; this.capture = null; old?.close();
  }
  select(enabled: boolean): void {
    this.disconnect(); this.selected = enabled;
    this.phase = enabled ? 'disconnected' : 'disabled';
    this.reason = enabled ? 'Managed PCM selected; reconnect to negotiate' : 'Legacy TX selected';
    this.publish();
  }
  connect(control: (text: string) => void, media: (frame: ArrayBuffer) => void): number {
    this.disconnect(); this.control = control; this.media = media;
    this.connectedAt = this.options.now();
    if (this.selected) {
      this.phase = 'negotiating'; this.reason = 'Waiting for shared-owner capabilities and PCM acceptance';
      // No bank4, no Opus advertisement, and no implicit fallback.
      control('tx_codec_caps:0,pcm;');
    }
    this.publish(); return this.epoch;
  }
  disconnect(): void {
    this.invalidate(); this.epoch++;
    this.control = null; this.media = null; this.requested = null; this.deadline = 0;
    this.bridgeReady = false; this.operator = false; this.supported = false; this.accepted = false;
    this.owner = null; this.offered = 0n; this.transactions.clear(); this.awaitingQueue = []; this.terminal.clear();
    this.initialTransaction = 0n;
    this.appliedConfig = null;
    this.cleanupAfterStop = null;
    this.lastTransaction = 'No managed settings result';
    this.phase = this.selected ? 'disconnected' : 'disabled'; this.reason = 'Disconnected; output invalidated';
    this.publish();
  }
  setOperator(operator: boolean): void {
    if (this.operator && !operator && this.requested) this.fail('Operator authority unavailable');
    this.operator = operator;
    if (operator && this.phase === 'unavailable') this.phase = 'negotiating';
    this.refresh();
  }
  private refresh(): void {
    if (!this.selected || !this.control || ['fault', 'unsupported', 'unavailable'].includes(this.phase)) return;
    if (this.requested) return;
    if (!this.bridgeReady || !this.supported || !this.accepted || !this.owner) {
      this.phase = 'negotiating'; this.reason = 'Waiting for managed PCM negotiation';
    } else if (!this.operator) {
      this.phase = 'unavailable'; this.reason = 'Managed TX unavailable: operator role required';
    } else if (this.owner.fault !== 'none') {
      this.phase = 'fault'; this.reason = `Owner fault latched: ${this.owner.fault}`;
    } else if (!this.owner.clean || this.status().pendingSettings
      || (this.cleanupAfterStop !== null && this.owner.stop <= this.cleanupAfterStop)
      || (this.appliedConfig !== null && this.appliedConfig !== this.owner.config)) {
      this.phase = 'releasing'; this.reason = 'Waiting for committed settings and completed owner cleanup';
    } else {
      this.cleanupAfterStop = null; this.deadline = 0;
      this.phase = 'ready'; this.reason = `Managed ${this.owner.source === 'satp' ? 'SATP' : 'tagged PCM'} ready (review)`;
    }
    this.publish();
  }
  sendControl(text: string): void {
    if (!this.control) throw new Error('managed_disconnected');
    if (this.selected && managedSettingsMessage(text)) {
      if (this.awaitingQueue.length + this.transactions.size >= 32) { this.fail('Settings queue limit'); return; }
      this.awaitingQueue.push(this.options.now());
      if (this.requested) this.release();
      this.phase = 'releasing'; this.reason = 'Settings sent; awaiting queued/applied result'; this.publish();
    }
    try { this.control(text); } catch { this.fail('Control transport unavailable'); }
  }
  arm(source: Source): boolean {
    if (!this.selected || this.phase !== 'ready' || !this.operator || this.owner?.source !== source
      || this.status().pendingSettings || !this.control) return false;
    this.invalidate(); this.requested = source; this.deadline = this.options.now() + 5000;
    this.phase = 'arming'; this.reason = source === 'tci' ? 'Waiting for fresh generation and microphone' : 'Waiting for SATP owner key';
    this.publish();
    try { this.control('trx:0,true;'); } catch { this.fail('Arm transport unavailable'); return false; }
    return true;
  }
  release(): void {
    this.invalidate(); this.requested = null; this.deadline = this.options.now() + 5000;
    if (this.cleanupAfterStop === null) this.cleanupAfterStop = this.owner?.stop ?? 0n;
    if (this.phase !== 'fault' && this.phase !== 'unavailable') this.phase = 'releasing';
    if (this.phase !== 'fault' && this.phase !== 'unavailable') this.reason = 'Output inhibited; waiting for owner stop/cleanup';
    try { this.control?.('trx:0,false;'); } catch { this.reason = 'Release transport unavailable; reconnect required'; }
    this.publish();
  }
  private fail(reason: string): void {
    this.invalidate(); this.requested = null; this.deadline = 0;
    this.phase = 'fault'; this.reason = reason;
    try { this.control?.('trx:0,false;'); } catch { /* local inhibition is already complete */ }
    this.publish();
  }
  tick(): void {
    if (!this.selected || !this.control) return;
    const now = this.options.now();
    if ((!this.supported || !this.accepted || !this.owner || !this.bridgeReady) && now - this.connectedAt > 5000) {
      if (this.phase !== 'fault') { this.phase = 'unsupported'; this.reason = 'Unsupported: Bridge requires PSG1 PCM, owner1 and transactions1'; this.publish(); }
    }
    if (this.awaitingQueue.some(at => now - at > 5000)
      || [...this.transactions.values()].some(t => now - t.queuedAt > 5000)) this.fail('Settings completion unavailable; reconnect required');
    if (this.deadline && now > this.deadline) this.fail('Owner generation/cleanup deadline; reconnect required');
    if (this.capture && now - this.lastInputAt > 500) this.fail('Microphone capture stopped; reconnect required');
  }
  text(text: string, epoch: number): void {
    if (!this.selected || epoch !== this.epoch || !this.control) return;
    for (const raw of text.split(';')) {
      const c = raw.trim();
      if (c === 'ready') this.bridgeReady = true;
      if (c.startsWith('saturn_tx_media_caps:')) {
        const caps = c.slice('saturn_tx_media_caps:'.length).split(',');
        this.supported = ['psg1', 'pcm', 'owner1', 'transactions1'].every(v => caps.includes(v));
        if (!this.supported) {
          this.fail('Unsupported managed PCM contract; no legacy fallback');
          this.phase = 'unsupported';
        }
      }
      if (c.startsWith('tx_codec_accept:0,')) {
        this.accepted = c === 'tx_codec_accept:0,pcm';
        if (!this.accepted) this.fail('Managed session requires PCM acceptance; no Opus fallback');
      }
      if (c.startsWith('tx_fault:') && c !== 'tx_fault:0,psg1,GenerationMismatch') this.fail(`Bridge fault: ${c.slice(9)}`);
      if (c.startsWith('tx_config:')) this.transaction(c);
      if (c.startsWith('saturn_shared_control_reject:'))
        this.lastTransaction = `Host control unavailable: ${c.slice('saturn_shared_control_reject:'.length)}`;
      const owner = /^saturn_tx_owner:shared,([1-9][0-9]*),([A-Za-z]+),([0-9]+),([0-9]+),(tci|satp),(true|false),([A-Za-z]+)$/.exec(c);
      if (owner && (!this.owner || BigInt(owner[1]!) > this.owner.serial)) {
        this.owner = {serial: BigInt(owner[1]!), stop: BigInt(owner[3]!), phase: owner[2]!, config: owner[4]!, source: owner[5]! as Source,
          clean: owner[6] === 'true', fault: owner[7]!};
        if (this.owner.fault !== 'none') this.fail(`Owner fault latched: ${this.owner.fault}`);
        else if (this.requested && ['AwaitingAuthority', 'CleaningNative', 'ReadyOff', 'Blocked', 'Closed'].includes(this.owner.phase)) {
          this.invalidate(); this.requested = null; this.phase = 'releasing';
        }
        if (this.owner.clean && (this.cleanupAfterStop === null || this.owner.stop > this.cleanupAfterStop)) this.deadline = 0;
        if (this.requested === 'satp' && this.owner.phase === 'Active') {
          this.phase = 'streaming'; this.reason = 'Managed SATP active'; this.deadline = 0;
        }
      }
      const offer = /^saturn_tx_generation:psg1,(off|[a-f0-9]{32}),([1-9][0-9]*)$/.exec(c);
      if (offer) void this.generation(offer[1]!, BigInt(offer[2]!));
    }
    this.refresh(); this.publish();
  }
  private transaction(c: string): void {
    const m = /^tx_config:0,([1-9][0-9]*),(queued|applied|rejected)(?:,([A-Za-z]+))?,([0-9]+)$/.exec(c);
    if (!m) { this.fail('Malformed settings completion'); return; }
    const [, id, outcome, reason, config] = m;
    if (this.terminal.has(id!)) return;
    if (outcome === 'queued') {
      if (!this.transactions.has(id!)) {
        if (!this.awaitingQueue.length || BigInt(id!) <= this.initialTransaction) {
          this.fail('Unexpected settings transaction'); return;
        }
        this.initialTransaction = BigInt(id!);
        this.awaitingQueue.shift(); this.transactions.set(id!, {config: config!, queuedAt: this.options.now()});
      }
    } else {
      const pending = this.transactions.get(id!);
      // The Bridge serializes initial replies on this control session. A
      // validation rejection has no queued receipt and uses configuration 0;
      // it completes the oldest request still awaiting its initial reply.
      // The monotonic initial-result watermark prevents an evicted old ID
      // from consuming a newer request. It does NOT order queued completions.
      const prequeueRejected = !pending && outcome === 'rejected' && config === '0' && reason
        && this.awaitingQueue.length > 0 && BigInt(id!) > this.initialTransaction;
      if (prequeueRejected) {
        this.initialTransaction = BigInt(id!);
        this.awaitingQueue.shift();
      } else if (!pending || pending.config !== config) {
        this.fail('Unmatched settings completion'); return;
      }
      this.transactions.delete(id!); this.terminal.add(id!);
      if (this.terminal.size > 64) this.terminal.delete(this.terminal.values().next().value!);
      this.lastTransaction = `Settings ${id}: ${outcome}${reason ? ` (${reason})` : ''}; ${prequeueRejected ? 'before queueing' : `configuration ${config}`}`;
      if (outcome === 'applied') this.appliedConfig = config!;
      if (outcome === 'rejected') this.fail(this.lastTransaction);
    }
  }
  private async generation(token: string, serial: bigint): Promise<void> {
    if (serial < this.offered || serial > 0xffffffffffffffffn) return;
    if (token === 'off') {
      if (this.requested && this.cleanupAfterStop === null) this.cleanupAfterStop = this.owner?.stop ?? 0n;
      this.offered = serial; this.invalidate(); this.requested = null;
      if (this.phase !== 'fault') this.phase = 'releasing';
      this.refresh(); return;
    }
    if (serial === this.offered) return;
    this.offered = serial;
    // A publication already in transit may arrive after local release. It
    // carries no fresh operator action: inhibit it without inventing a fault.
    if (!this.requested && this.phase === 'releasing') return;
    if (this.requested !== 'tci' || !this.supported || !this.accepted || !this.operator) {
      this.fail('Unsolicited output generation'); return;
    }
    this.invalidate(); this.token = token;
    const version = this.version, requestedAt = this.options.now();
    try {
      const capture = await this.options.capture(token, (samples, oldest) => this.input(version, token, samples, oldest),
        reason => { if (version === this.version) this.fail(reason); });
      if (version !== this.version || this.token !== token) { capture.close(); return; }
      if (capture.token !== token || !Number.isFinite(capture.historyResetAtMs)
        || capture.historyResetAtMs < requestedAt || capture.historyResetAtMs > this.options.now()) {
        capture.close(); throw new Error('capture_history_not_fresh');
      }
      this.capture = capture; this.lastInputAt = this.options.now();
      this.phase = 'streaming'; this.reason = 'Managed tagged PCM capture active (review)';
      this.deadline = 0; this.publish();
    } catch (e) {
      if (version === this.version) this.fail(`Microphone unavailable: ${e instanceof Error ? e.message : e}`);
    }
  }
  private input(version: number, token: string, samples: Float32Array, oldest: number): void {
    if (version !== this.version || token !== this.token || !this.capture || this.requested !== 'tci') return;
    const now = this.options.now();
    this.lastInputAt = now;
    if (!Number.isFinite(oldest) || oldest < this.capture.historyResetAtMs || oldest > now
      || now - oldest > 150 || samples.length !== 512 || samples.some(v => !Number.isFinite(v))
      || this.sequence === 0xffffffff) { this.fail('PCM capture age/format/sequence invalid'); return; }
    try {
      this.media!(tag(buildTxMicPcmS16Frame(samples, this.sequence++).frame, token)); this.sentFrames++;
    } catch { this.fail('PCM output unavailable; no packet dropping or legacy fallback'); }
  }
}
