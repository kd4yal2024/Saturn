// Selectable host integration, off by default. No Opus producer or legacy tee.
import { buildTxMicPcmS16Frame } from './tx-uplink';
import { TIMING_PROBE_VERSION } from '../puresignal/timing-probe';
export const NATIVE_TWO_TONE_VERSION = 'wdsp-postgen-two-tone-v1';

export interface ManagedCapture {
  token: string;
  historyResetAtMs: number;
  close(): void;
}
export type ManagedCaptureFactory = ((token: string,
  deliver: (samples: Float32Array, oldestMs: number) => void,
  failed: (reason: string) => void) => Promise<ManagedCapture>) & {
    prepare?(): Promise<void>;
    cancel?(): void;
    dispose?(): void;
  };
type Phase = 'disabled' | 'disconnected' | 'negotiating' | 'unsupported' | 'unavailable'
  | 'ready' | 'arming' | 'streaming' | 'releasing' | 'fault';
type Source = 'tci' | 'satp';
export interface ManagedPcmStatus {
  selected: boolean; phase: Phase; reason: string; pendingSettings: number;
  lastTransaction: string; source: Source; sentFrames: number;
  ownerPhase: string | null;
  microphonePreparation: 'cold' | 'preparing' | 'ready' | 'unavailable';
  psCapability?: 'unavailable' | 'single' | 'measurement-only';
  psMode?: string;
  measurementLimitMs?: number | undefined;
  measurementFrequencyHz?: number | undefined;
  measurementCaptureMode?: string | undefined;
  measurementResult?: string;
  measurementActive?: boolean;
  timingSupported?: boolean;
  nativeTwoToneSupported?: boolean;
  manualAttenuationSupported?: boolean;
  canAdjustFaultedFeedback?: boolean;
  receiveTuningReady?: boolean;
  autoAdaptationSupported?: boolean;
  autoAttenuationSupported?: boolean;
  autoAttenuationEnabled?: boolean | undefined;
  canAdjustFaultedAutoAtt?: boolean;
  autoAttStatus?: { phase: string; db: number; appliedDb: number | null; info: number | null; frequencyHz: number; keyed: boolean } | undefined;
  twoToneOperatorControlled?: boolean;
  twoToneMaxWatts?: number | undefined;
  twoToneLimitMs?: number | undefined;
  twoToneFrequencyHz?: number | undefined;
  twoToneActive?: boolean;
  twoToneResult?: string;
  driveLimitWatts?: number | undefined;
  committedWatts?: number | undefined;
  audioDroppedFrames?: number;
  canAcknowledgeFault?: boolean;
  faultAcknowledgment?: string;
  generatedTrialsOnly?: boolean;
}
interface Measurement { id: string | null; stopAt: number; stopping: boolean; complete: boolean; timing: boolean; native: boolean }
interface Owner { serial: bigint; stop: bigint; phase: string; config: string; source: Source; clean: boolean; fault: string }
interface Transaction { config: string; queuedAt: number }
const SETTINGS = /^(?:modulation|vfo|vfo_active|split|split_enable|drive|tx_drive|tx_pa_calibration|tx_mic_gain|tx_filter_band|tx_eq_enable|tx_eq_band|tx_cfc_enable|tx_cfc_precomp|tx_cfc_band|tx_phase_rotator|tx_phase_rotator_auto|tx_phase_rotator_corner|tx_noise_gate|tx_noise_gate_threshold|tx_dexp|tx_dexp_threshold|tx_dexp_expansion|tx_speech_processor|tx_speech_processor_gain|tx_cessb|tx_fft_size|tx_low_latency|tx_puresignal|tx_puresignal_mode|tx_puresignal_auto_attenuate|tx_puresignal_attenuation|tx_puresignal_reset|tx_two_tone|tx_two_tone_freq1|tx_two_tone_freq2|tx_two_tone_level_db|tx_two_tone_invert_lsb|tx_two_tone_delay_ms):/;
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
// Only absolute tuning targets supersede one another. Never merge across a
// source/configuration barrier, relative tuning, queries, release or an arm.
function tuningKey(text: string): string | null {
  const commands = text.split(';').map(c => c.trim()).filter(Boolean);
  if (!commands.length || !commands.every(c => /^(?:vfo:0,[01],[0-9]+|dds:0,[0-9]+)$/.test(c))) return null;
  return commands.map(c => c.slice(0, c.lastIndexOf(','))).join(';');
}
// Power is an absolute target. A burst of slider/spinner edits must retain the
// latest unsent target, not turn ordinary UI input into a session-wide TX fault.
// Only adjacent, valid power setters coalesce; source/mode/PS actions and other
// settings remain ordering barriers. Sent transactions keep their real receipts.
function powerKey(text: string): string | null {
  const commands=text.split(';').map(c=>c.trim()).filter(Boolean);
  if(commands.length!==1)return null;
  const match=/^(?:tx_drive|drive):0,([0-9]+)$/.exec(commands[0]!);
  const value=Number(match?.[1]);
  return match && Number.isSafeInteger(value) && value>=1 && value<=100 ? 'power:0' : null;
}
function feedbackAttenuationKey(text: string): string | null {
  const commands=text.split(';').map(c=>c.trim()).filter(Boolean);
  if(commands.length!==1)return null;
  const m=/^tx_puresignal_attenuation:0,([0-9]+)$/.exec(commands[0]!);
  const db=Number(m?.[1]);
  return m && Number.isInteger(db) && db>=0 && db<=31 ? 'feedback-attenuation:0' : null;
}
function autoAttenuationKey(text: string): string | null {
  return /^tx_puresignal_auto_attenuate:0,(?:true|false);?$/.test(text.trim()) ? 'auto-attenuation:0' : null;
}
function tag(frame: ArrayBuffer, token: string): ArrayBuffer {
  const view = new DataView(frame);
  for (let i = 0; i < 16; i++) view.setUint8(44 + i, parseInt(token.slice(i * 2, i * 2 + 2), 16));
  view.setUint32(60, 0x50534731, true);
  return frame;
}
export class ManagedPcmSession {
  private audioDroppedFrames = 0;
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
  private microphonePreparation: ManagedPcmStatus['microphonePreparation'] = 'cold';
  private microphonePreparationVersion = 0;
  private microphoneDeviceChanged = false;
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
  // One settings transaction on the wire. The bounded local FIFO preserves
  // barriers; adjacent absolute tuning/power targets may supersede one another.
  private settingsQueue: { text: string; key: string | null }[] = [];
  private tuningEchoTargets = new Map<string, string>();
  private faultReleaseSent = false;
  private ackSupported = false;
  private faultedTuningSupported = false;
  private faultedAttenuationSupported = false;
  private ackStop: bigint | null = null;
  private nextAck = 0n;
  private ackPending: {id: string; stop: bigint; serial: bigint; at: number} | null = null;
  private ackResult = 'No fault acknowledgment requested';
  private driveLimit: number | undefined;
  private generatedOnly = false;
  private committedDrive: number | undefined;
  private control: ((text: string) => void) | null = null;
  private media: ((frame: ArrayBuffer) => void) | null = null;
  private psCapability: 'unavailable' | 'single' | 'measurement-only' = 'unavailable';
  private psMode = 'unavailable';
  private psConfig = 0n;
  private rfPermitted = false;
  private diagnostic: { maxMs: number; frequencyHz: number; captureMode: string } | null = null;
  private measurement: Measurement | null = null;
  private measurementResult = 'No measurement requested';
  private timingSupported = false;
  private nativeTwoToneSupported = false;
  private manualAttenuationSupported = false;
  private autoAdaptationSupported = false;
  private autoAttenuationSupported = false;
  private autoAttenuationEnabled: boolean | undefined;
  private autoAtt: {stop:bigint; config:string; phase:string;db:number;appliedDb:number|null;info:number|null;frequencyHz:number;keyed:boolean}|null=null;
  private twoToneLimits: { maxMs: number; frequencyHz: number; maxWatts: number; operatorControlled: boolean } | null = null;
  private twoTone: { id: string; stopAt: number; stop: bigint; queued: boolean; accepted: boolean; stopping: boolean; complete: boolean; native: boolean } | null = null;
  private nextTwoTone = 0n;
  private twoToneResult = 'No generated calibration trial requested';
  constructor(private readonly options: { now(): number; capture: ManagedCaptureFactory;
    measurementCapture?: ManagedCaptureFactory;
    twoToneCapture?: ManagedCaptureFactory;
    timingCapture?: ManagedCaptureFactory;
    nativeKeepaliveCapture?: ManagedCaptureFactory;
    changed(status: ManagedPcmStatus): void }) {}
  status(): ManagedPcmStatus {
    return { selected: this.selected, phase: this.phase, reason: this.reason,
      pendingSettings: this.awaitingQueue.length + this.transactions.size + this.settingsQueue.length,
      lastTransaction: this.lastTransaction, source: this.owner?.source ?? 'tci', sentFrames: this.sentFrames,
      ownerPhase: this.owner?.phase ?? null, microphonePreparation: this.microphonePreparation, psCapability: this.psCapability, psMode: this.psMode,
      measurementLimitMs: this.diagnostic?.maxMs, measurementFrequencyHz: this.diagnostic?.frequencyHz,
      measurementCaptureMode: this.diagnostic?.captureMode, measurementResult: this.measurementResult,
      measurementActive: this.measurement !== null, timingSupported: this.timingSupported,
      nativeTwoToneSupported: this.nativeTwoToneSupported,
      manualAttenuationSupported: this.manualAttenuationSupported,
      canAdjustFaultedFeedback: this.faultedAttenuationSupported && this.manualAttenuationSupported
        && this.faultedTuningReady() && this.statusPendingWire()===0 && this.settingsQueue.length===0,
      receiveTuningReady: this.faultedTuningReady(),
      autoAdaptationSupported: this.autoAdaptationSupported,
      autoAttenuationSupported: this.autoAttenuationSupported,
      autoAttenuationEnabled: this.autoAttenuationEnabled,
      canAdjustFaultedAutoAtt: this.autoAttenuationSupported && this.faultedTuningReady()
        && this.statusPendingWire()===0 && this.settingsQueue.length===0,
      autoAttStatus: this.autoAtt && this.owner?.stop===this.autoAtt.stop && this.owner.config===this.autoAtt.config
        ? {phase:this.autoAtt.phase,db:this.autoAtt.db,appliedDb:this.autoAtt.appliedDb,info:this.autoAtt.info,frequencyHz:this.autoAtt.frequencyHz,keyed:this.autoAtt.keyed} : undefined,
      twoToneOperatorControlled: this.twoToneLimits?.operatorControlled ?? false, twoToneMaxWatts: this.twoToneLimits?.maxWatts,
      twoToneLimitMs: this.twoToneLimits?.maxMs, twoToneFrequencyHz: this.twoToneLimits?.frequencyHz,
      twoToneActive: this.twoTone !== null, twoToneResult: this.twoToneResult,
      driveLimitWatts: this.driveLimit, committedWatts: this.committedDrive,
      audioDroppedFrames: this.audioDroppedFrames,
      canAcknowledgeFault: this.canAcknowledgeFault(), faultAcknowledgment: this.ackResult,
      generatedTrialsOnly: this.generatedOnly };
  }
  private publish(): void { this.prepareMicrophone(); this.options.changed(this.status()); }
  private resetMicrophone(): void {
    this.microphonePreparationVersion++;
    this.microphonePreparation = 'cold'; this.microphoneDeviceChanged = false;
    this.options.capture.dispose?.();
  }
  /** Device preparation is not a TX action. It retains no PCM and grants no
   * output generation; every press still creates a fresh worklet and track. */
  private prepareMicrophone(): void {
    if (this.microphoneDeviceChanged && !this.requested) this.resetMicrophone();
    const wanted = this.selected && this.operator && this.bridgeReady && this.supported && this.accepted
      && this.owner?.source === 'tci' && !this.generatedOnly && this.psCapability !== 'measurement-only'
      && !['disabled', 'disconnected', 'fault', 'unsupported', 'unavailable'].includes(this.phase);
    if (!wanted) {
      if (this.microphonePreparation !== 'cold') this.resetMicrophone();
      return;
    }
    if (!this.options.capture.prepare || this.microphonePreparation !== 'cold') return;
    const version = ++this.microphonePreparationVersion;
    this.microphonePreparation = 'preparing';
    void this.options.capture.prepare().then(() => {
      if (version !== this.microphonePreparationVersion) return;
      this.microphonePreparation = 'ready'; this.publish();
    }, () => {
      if (version !== this.microphonePreparationVersion) return;
      this.microphonePreparation = 'unavailable'; this.publish();
    });
  }
  refreshMicrophone(): void {
    if (this.requested) { this.microphoneDeviceChanged = true; return; }
    this.resetMicrophone(); this.publish();
  }
  private invalidate(): void {
    this.version++; this.token = null; this.sequence = 0;
    this.options.capture.cancel?.();
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
    this.invalidate(); this.resetMicrophone(); this.epoch++;
    this.control = null; this.media = null; this.requested = null; this.deadline = 0;
    this.bridgeReady = false; this.operator = false; this.supported = false; this.accepted = false;
    this.owner = null; this.offered = 0n; this.transactions.clear(); this.awaitingQueue = []; this.terminal.clear();
    this.initialTransaction = 0n;
    this.appliedConfig = null;
    this.cleanupAfterStop = null;
    this.settingsQueue = []; this.tuningEchoTargets.clear(); this.faultReleaseSent = false;
    this.faultedTuningSupported = false; this.faultedAttenuationSupported = false;
    this.autoAttenuationSupported=false;this.autoAttenuationEnabled=undefined;this.autoAtt=null;
    this.ackSupported = false; this.ackStop = null; this.ackPending = null; this.driveLimit = undefined;
    this.ackResult = 'No fault acknowledgment requested';
    this.committedDrive = undefined;
    this.audioDroppedFrames = 0;
    this.generatedOnly = false;
    this.psCapability = 'unavailable'; this.psMode = 'unavailable'; this.psConfig = 0n;
    this.rfPermitted = false; this.diagnostic = null; this.measurement = null; this.timingSupported = false;
    this.twoToneLimits = null; this.twoTone = null;
    this.nativeTwoToneSupported = false;
    this.manualAttenuationSupported = false;
    this.autoAdaptationSupported = false;
    this.twoToneResult = 'No generated calibration trial requested in this connection';
    this.measurementResult = 'No measurement requested in this connection';
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
    if (this.measurement || this.twoTone) return; // terminal reply AND completed cleanup precede another arm
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
      this.phase = 'ready'; this.reason = this.generatedOnly ? 'Managed native two-tone trials only; ordinary PTT/MOX unavailable'
        : `Managed ${this.owner.source === 'satp' ? 'SATP' : 'tagged PCM'} ready (review)`;
    }
    this.publish();
  }
  sendControl(text: string): void {
    if (!this.control) throw new Error('managed_disconnected');
    if (text.split(';').some(c => /^(?:trx:0,false|saturn_ps_measurement:0,stop)$/.test(c.trim()))) {
      this.settingsQueue = []; this.tuningEchoTargets.clear();
    }
    const tune = tuningKey(text);
    const attenuation = feedbackAttenuationKey(text);
    const autoAtt = autoAttenuationKey(text);
    const key = tune ?? attenuation ?? autoAtt ?? powerKey(text);
    if (this.selected && managedSettingsMessage(text)) {
      if (['fault', 'unsupported', 'unavailable'].includes(this.phase)
        && !((tune || (attenuation && this.faultedAttenuationSupported && this.manualAttenuationSupported)
          || (autoAtt && this.autoAttenuationSupported)) && this.faultedTuningReady())) return;
      if (tune) {
      for (const c of text.split(';').map(c => c.trim()).filter(Boolean)) {
        this.tuningEchoTargets.set(c.slice(0, c.lastIndexOf(',')), c);
      }
      }
      const tail = this.settingsQueue.at(-1);
      if (key && tail?.key === key) tail.text = text;
      else {
        if (this.settingsQueue.length >= 32) { this.fail('Local settings queue limit; unsent settings cancelled'); return; }
        this.settingsQueue.push({text, key});
      }
      this.flushTune(); this.publish(); return;
    }
    this.sendControlNow(text);
  }
  /** Keep absolute operator targets until release/fault/disconnect or a new
   * target. An untagged echo matching an oscillating knob value is NOT proof
   * that later queued echoes are current. Owner receipts govern admission;
   * this affects presentation only and never grants output authority. */
  filterTuningEcho(text: string): string {
    return text.split(';').filter(raw => {
      const c = raw.trim();
      const drive = /^saturn_tx_committed_drive:0,([0-9]+),([1-9][0-9]*)$/.exec(c);
      if (drive && this.owner?.config === drive[1] && Number(drive[2]) <= 100) this.committedDrive = Number(drive[2]);
      if (!tuningKey(c)) return true;
      const key = c.slice(0, c.lastIndexOf(','));
      const target = this.tuningEchoTargets.get(key);
      if (!target) return true;
      if (c !== target) return false;
      return true;
    }).join(';');
  }
  private faultedTuningReady(): boolean {
    return this.faultedTuningSupported && this.selected && this.operator && this.bridgeReady
      && this.supported && this.accepted && this.phase === 'fault' && this.ackPending === null
      && this.owner?.phase === 'ReadyOff' && this.owner.fault !== 'none' && this.ackStop === this.owner.stop;
  }
  private flushTune(): void {
    const nextText=this.settingsQueue[0]?.text ?? '';
    const faultedTune = this.faultedTuningReady() && (!!tuningKey(nextText)
      || (this.faultedAttenuationSupported && this.manualAttenuationSupported && !!feedbackAttenuationKey(nextText))
      || (this.autoAttenuationSupported && !!autoAttenuationKey(nextText)));
    if (!this.settingsQueue.length || !this.control
      || (['fault', 'unsupported', 'unavailable'].includes(this.phase) && !faultedTune)) return;
    if (this.awaitingQueue.length || this.transactions.size || (this.owner && !this.owner.clean && !faultedTune)
      || (this.appliedConfig !== null && this.owner?.config !== this.appliedConfig)) return;
    const next = this.settingsQueue.shift()!;
    this.sendControlNow(next.text);
  }
  private sendControlNow(text: string): void {
    if (!this.control) throw new Error('managed_disconnected');
    if (this.selected && managedSettingsMessage(text)) {
      if (this.awaitingQueue.length + this.transactions.size >= 32) { this.fail('Settings queue limit'); return; }
      this.awaitingQueue.push(this.options.now());
      // Release before tracking the request: cancellation discards only unsent
      // work; the transmitted request still requires its terminal receipt.
      if (this.requested) this.release();
      if (this.phase !== 'fault') { this.phase = 'releasing'; this.reason = 'Settings sent; awaiting queued/applied result'; }
      this.publish();
    }
    try { this.control(text); } catch { this.fail('Control transport unavailable'); }
  }
  arm(source: Source, hardwarePttToken?: string): boolean {
    if (hardwarePttToken !== undefined && !/^[1-9][0-9]{0,19}$/.test(hardwarePttToken)) return false;
    if (hardwarePttToken !== undefined && BigInt(hardwarePttToken) > 0xffffffffffffffffn) return false;
    if (this.generatedOnly) return false;
    if (this.psCapability === 'measurement-only') return false; // use the explicit finite action
    if (!this.selected || this.phase !== 'ready' || !this.operator || this.owner?.source !== source
      || this.status().pendingSettings || !this.control) return false;
    this.invalidate(); this.requested = source; this.deadline = this.options.now() + 5000;
    this.phase = 'arming'; this.reason = source === 'tci' ? 'Waiting for fresh generation and microphone' : 'Waiting for SATP owner key';
    this.publish();
    try { this.control(hardwarePttToken ? `saturn_ptt:0,${hardwarePttToken};` : 'trx:0,true;'); } catch { this.fail('Arm transport unavailable'); return false; }
    return true;
  }
  /** Configure one fit. This action never arms, keys, starts audio or retries. */
  preparePureSignal(enabled: boolean): boolean {
    return this.preparePureSignalMode(enabled ? 'single' : 'off');
  }
  /** An explicit native adaptation mode, independently negotiated from Auto ATT.
   * Settings-only: this never creates audio, arms, keys or retries RF. */
  preparePureSignalMode(mode: 'off' | 'single' | 'auto'): boolean {
    const enabled = mode !== 'off';
    if (!this.selected || this.phase !== 'ready' || !this.operator || this.measurement
      || (this.psCapability !== 'single' && !(this.psCapability === 'measurement-only' && !enabled))
      || (mode === 'auto' && !this.autoAdaptationSupported) || !this.control) return false;
    this.sendControl(mode === 'auto' ? 'tx_puresignal_mode:0,auto;' : `tx_puresignal:0,${enabled};`);
    return true;
  }
  /** Explicit attended RF action, not a settings completion or implicit PTT.
   * The host separately opts in after reviewing feedback scaling and limits. */
  startTwoTone(durationMs: number, frequencyHz: number, commandedWatts: number, attended: boolean): boolean {
    if (!this.selected || this.phase !== 'ready' || !this.operator || !this.control || this.measurement || this.twoTone
      || !this.options.twoToneCapture || !this.twoToneLimits || !this.rfPermitted
      || (this.generatedOnly && !this.nativeTwoToneSupported)
      || (this.nativeTwoToneSupported && !this.options.nativeKeepaliveCapture)
      || this.psCapability !== 'single' || (!['off', 'single'].includes(this.psMode ?? '') && !(this.psMode === 'auto' && this.autoAdaptationSupported)) || this.owner?.source !== 'tci'
      || this.status().pendingSettings || !attended || (!this.twoToneLimits.operatorControlled && frequencyHz !== this.twoToneLimits.frequencyHz)
      || !Number.isSafeInteger(commandedWatts) || commandedWatts < 1 || commandedWatts > this.twoToneLimits.maxWatts
      || !Number.isSafeInteger(durationMs) || durationMs < 0 || durationMs > 18_446_744_073_709
      || (!this.twoToneLimits.operatorControlled && (durationMs < 1 || durationMs > this.twoToneLimits.maxMs))
      || this.nextTwoTone === 0xffffffffffffffffn) return false;
    const id = String(++this.nextTwoTone);
    this.invalidate(); this.requested = 'tci';
    this.twoTone = {id, stopAt: durationMs === 0 ? Infinity : this.options.now() + durationMs, stop: this.owner.stop,
      queued: false, accepted: false, stopping: false, complete: false, native: this.nativeTwoToneSupported};
    this.deadline = this.options.now() + 15000;
    this.phase = 'arming'; this.reason = this.psMode === 'off'
      ? 'Attended PS-off two-tone baseline requested; awaiting host'
      : this.psMode === 'auto' ? 'Attended two-tone Auto trial requested; awaiting host'
        : 'Attended two-tone Single trial requested; awaiting host';
    this.twoToneResult = 'Waiting for host admission; correction and RF unverified'; this.publish();
    try { this.control(`saturn_ps_two_tone:0,${id},start,${durationMs}${this.twoTone.native ? ','+NATIVE_TWO_TONE_VERSION : ''};`); }
    catch { this.fail('Two-tone request transport unavailable'); return false; }
    return true;
  }
  /** This separate attended action CAN key RF, bounded independently by the host. */
  startMeasurement(durationMs: number, frequencyHz: number, commandedWatts: number, attended: boolean, timing = false, carrier = false): boolean {
    if (!this.selected || this.phase !== 'ready' || !this.operator || !this.control || this.measurement
      || !this.options.measurementCapture || this.psCapability !== 'measurement-only' || this.psMode !== 'off'
      || (timing && (!this.timingSupported || !this.options.timingCapture))
      || (!timing && !carrier && this.nativeTwoToneSupported && !this.options.nativeKeepaliveCapture)
      || !this.diagnostic || !this.rfPermitted || this.owner?.source !== 'tci'
      || this.status().pendingSettings || !attended || frequencyHz !== this.diagnostic.frequencyHz
      || !Number.isSafeInteger(commandedWatts) || commandedWatts < 1 || commandedWatts > 5
      || !Number.isSafeInteger(durationMs) || durationMs < 1 || durationMs > this.diagnostic.maxMs) return false;
    this.invalidate(); this.requested = 'tci';
    this.measurement = { id: null, stopAt: this.options.now() + durationMs, stopping: false, complete: false, timing,
      native: !timing && !carrier && this.nativeTwoToneSupported };
    this.deadline = this.options.now() + durationMs + 15000;
    this.phase = 'arming'; this.reason = 'MeasurementOnly requested: fitting/correction off; waiting for host';
    this.measurementResult = 'Awaiting host admission; RF output not yet established'; this.publish();
    try { this.control(`saturn_ps_measurement:0,start,${durationMs}${timing ? ',' + TIMING_PROBE_VERSION : this.measurement.native ? ','+NATIVE_TWO_TONE_VERSION : ''};`); }
    catch { this.fail('Measurement request transport unavailable'); return false; }
    return true;
  }
  private stopMeasurement(): void {
    const measurement = this.measurement;
    if (!measurement || measurement.stopping || measurement.complete) return;
    measurement.stopping = true;
    this.invalidate(); this.requested = null;
    if (this.cleanupAfterStop === null) this.cleanupAfterStop = this.owner?.stop ?? 0n;
    if (this.phase !== 'fault') { this.phase = 'releasing'; this.reason = 'Measurement stopped locally; waiting for host cleanup/export'; }
    this.deadline = this.options.now() + 15000;
    try { this.control?.('saturn_ps_measurement:0,stop;'); }
    catch { this.fail('Measurement stop transport unavailable; cleanup unverified'); }
    this.publish();
  }
  release(hardwarePttToken?: string): void {
    const alreadyClean = Boolean(hardwarePttToken && this.phase === 'ready' && !this.requested && this.owner?.clean);
    this.ackPending = null;
    this.settingsQueue = []; this.tuningEchoTargets.clear();
    if (this.measurement) { this.stopMeasurement(); return; }
    if (this.twoTone?.stopping || this.twoTone?.complete) return;
    if (this.twoTone) this.twoTone.stopping = true;
    this.invalidate(); this.requested = null; this.deadline = this.options.now() + 5000;
    if (this.cleanupAfterStop === null) this.cleanupAfterStop = this.owner?.stop ?? 0n;
    if (this.phase !== 'fault' && this.phase !== 'unavailable') this.phase = 'releasing';
    if (this.phase !== 'fault' && this.phase !== 'unavailable') this.reason = 'Output inhibited; waiting for owner stop/cleanup';
    try { this.control?.(hardwarePttToken ? `saturn_ptt:0,stop,${hardwarePttToken};` : 'trx:0,false;'); } catch { this.reason = 'Release transport unavailable; reconnect required'; }
    if (alreadyClean) { this.cleanupAfterStop = null; this.deadline = 0; this.refresh(); return; }
    this.publish();
  }
  private fail(reason: string): void {
    const firstFault = this.phase !== 'fault';
    this.settingsQueue = []; this.tuningEchoTargets.clear();
    this.invalidate(); this.requested = null; this.deadline = 0;
    this.phase = 'fault'; if (firstFault) this.reason = reason;
    // Owner fault status is periodic. One fault must not become an endless
    // release -> cleanup -> status -> release loop. Local output stays closed.
    if (!this.faultReleaseSent) {
      this.faultReleaseSent = true;
      try { this.control?.('trx:0,false;'); } catch { /* local inhibition is already complete */ }
    }
    this.publish();
  }
  tick(): void {
    if (!this.selected || !this.control) return;
    this.flushTune();
    const now = this.options.now();
    if (this.ackPending && now - this.ackPending.at > 5000) {
      this.ackPending = null; this.ackResult = 'Acknowledgment completion unavailable; reconnect required'; this.publish();
    }
    if ((!this.supported || !this.accepted || !this.owner || !this.bridgeReady) && now - this.connectedAt > 5000) {
      if (this.phase !== 'fault') { this.phase = 'unsupported'; this.reason = 'Unsupported: Bridge requires PSG1 PCM, owner1 and transactions1'; this.publish(); }
    }
    if (this.awaitingQueue.some(at => now - at > 5000)
      || [...this.transactions.values()].some(t => now - t.queuedAt > 5000)) this.fail('Settings completion unavailable; reconnect required');
    if (this.deadline && now > this.deadline) this.fail('Owner generation/cleanup deadline; reconnect required');
    if (this.measurement && now >= this.measurement.stopAt) this.stopMeasurement();
    if (this.twoTone && now >= this.twoTone.stopAt) this.release();
    if (this.capture && now - this.lastInputAt > 500) this.fail('Microphone capture stopped; reconnect required');
  }
  text(text: string, epoch: number): void {
    if (!this.selected || epoch !== this.epoch || !this.control) return;
    for (const raw of text.split(';')) {
      const c = raw.trim();
      const skips = /^saturn_tx_audio_drops:0,([0-9]+),([0-9]+),([0-9]+)$/.exec(c);
      if (skips) {
        const count = Number(skips[3]);
        if (this.owner && BigInt(skips[1]!) === this.owner.stop && skips[2] === this.owner.config
          && Number.isSafeInteger(count) && count >= this.audioDroppedFrames) this.audioDroppedFrames = count;
      }
      if (c.startsWith('saturn_ps_auto_att_caps:')) this.autoAttenuationSupported=c==='saturn_ps_auto_att_caps:0,start31-owner-v1';
      const autoEnabled=/^tx_puresignal_auto_attenuate:0,(true|false)$/.exec(c);
      if (autoEnabled && this.autoAttenuationSupported) this.autoAttenuationEnabled=autoEnabled[1]==='true';
      const att=/^saturn_ps_auto_att:0,([0-9]+),([0-9]+),([0-9]+),(true|false),(off|applying|settling|leveling|stable|fault),([0-9]+),(-1|[0-9]+),(-1|[0-9]+),([0-9]+),(true|false)$/.exec(c);
      if (att && this.autoAttenuationSupported && this.owner?.stop===BigInt(att[1]!) && this.owner.config===att[2]
        && Number.isSafeInteger(Number(att[3])) && Number(att[3])>0 && Number(att[6])<=31 && Number(att[7])<=31 && Number(att[8])<=65535) {
        this.autoAtt={stop:BigInt(att[1]!),config:att[2]!,frequencyHz:Number(att[3]),phase:att[5]!,db:Number(att[6]),
          appliedDb:att[7]==='-1'?null:Number(att[7]),info:att[8]==='-1'?null:Number(att[8]),keyed:att[10]==='true'};
        if (this.requested && this.owner.phase==='AwaitingKey' && this.autoAtt.keyed) this.deadline=0;
      }
      if (c.startsWith('saturn_tx_arm_policy:')) {
        const generated = c === 'saturn_tx_arm_policy:0,native-two-tone-only';
        if (!generated && c !== 'saturn_tx_arm_policy:0,standard') this.fail('Unsupported host arm policy');
        else if (this.requested && generated !== this.generatedOnly) this.fail('Host arm policy changed during TX');
        this.generatedOnly = generated;
      }
      if (c.startsWith('saturn_tx_drive_limit:')) {
        const m = /^saturn_tx_drive_limit:0,([1-9][0-9]*)$/.exec(c);
        this.driveLimit = m && Number(m[1]) <= 100 ? Number(m[1]) : undefined;
      }
      if (c.startsWith('saturn_ps_fault_attenuation_caps:')) this.faultedAttenuationSupported = c === 'saturn_ps_fault_attenuation_caps:0,latched-rx-off-v1';
      if (c.startsWith('saturn_rx_tuning_caps:')) this.faultedTuningSupported = c === 'saturn_rx_tuning_caps:0,fault-latched-v1';
      if (c.startsWith('saturn_tx_fault_ack_caps:')) this.ackSupported = c === 'saturn_tx_fault_ack_caps:0,stop-fence-v1';
      const ackState = /^saturn_tx_fault_ack_state:0,([0-9]+),(true|false)$/.exec(c);
      if (ackState) this.ackStop = ackState[2] === 'true' ? BigInt(ackState[1]!) : null;
      const ack = /^saturn_tx_fault_ack:0,([1-9][0-9]*),(applied|rejected),([A-Za-z]+),([0-9]+)$/.exec(c);
      if (ack && this.ackPending && this.ackPending.id === ack[1] && this.ackPending.stop === BigInt(ack[4]!)) {
        const pending = this.ackPending; this.ackPending = null;
        if (ack[2] === 'applied' && ack[3] === 'none' && this.owner?.stop === pending.stop
          && this.owner.serial > pending.serial && this.owner.clean && this.owner.phase === 'ReadyOff' && this.owner.fault === 'none') {
          this.measurement = null; this.twoTone = null; this.deadline = 0;
          this.cleanupAfterStop = null; this.faultReleaseSent = false;
          this.phase = 'negotiating'; this.ackResult = 'Fault acknowledged; output OFF. A fresh attended TX action is required.';
        } else this.ackResult = `Fault acknowledgment ${ack[2]}: ${ack[3]}; output remains inhibited`;
      }
      if (c.startsWith('saturn_ps_control_caps:')) {
        const match = /^saturn_ps_control_caps:0,(single|measurement-only)$/.exec(c);
        const capability = match?.[1] as 'single' | 'measurement-only' | undefined;
        if (this.requested && capability !== this.psCapability) this.fail('PS capability changed during TX');
        this.psCapability = capability ?? 'unavailable';
      }
      if (c.startsWith('saturn_ps_manual_attenuation_caps:')) {
        this.manualAttenuationSupported = c === 'saturn_ps_manual_attenuation_caps:0,rx-off-ordered-v1';
      }
      if (c.startsWith('saturn_ps_auto_caps:')) {
        this.autoAdaptationSupported = c === 'saturn_ps_auto_caps:0,continuous-wdsp-v1';
      }
      const mode = /^saturn_ps_mode:0,([1-9][0-9]*),(off|single|auto)$/.exec(c);
      if (mode && BigInt(mode[1]!) >= this.psConfig) {
        this.psConfig = BigInt(mode[1]!); this.psMode = mode[2]!;
      }
      if (c.startsWith('tx_enable:0,')) {
        this.rfPermitted = c === 'tx_enable:0,true';
        if ((this.measurement || this.twoTone) && !this.rfPermitted) this.fail('Measurement RF permission withdrawn');
      }
      if (c.startsWith('saturn_ps_two_tone_caps:')) {
        const m = /^saturn_ps_two_tone_caps:0,(tagged-two-tone-v1|operator-two-tone-v1),([0-9]+),([0-9]+),([0-9]+)$/.exec(c);
        const operatorControlled = m?.[1] === 'operator-two-tone-v1';
        const maxMs = Number(m?.[2]), frequencyHz = Number(m?.[3]), maxWatts = Number(m?.[4]);
        const valid = Boolean(m && (operatorControlled
          ? maxMs === 0 && frequencyHz === 0 && Number.isSafeInteger(maxWatts) && maxWatts >= 1 && maxWatts <= 100
          : Number.isSafeInteger(maxMs) && maxMs > 0 && maxMs <= 45000 && maxWatts === 5
            && Number.isSafeInteger(frequencyHz) && frequencyHz >= 1500000 && frequencyHz <= 35000000));
        if (this.twoTone && (!valid || this.twoToneLimits?.maxMs !== maxMs || this.twoToneLimits.frequencyHz !== frequencyHz
          || this.twoToneLimits.maxWatts !== maxWatts || this.twoToneLimits.operatorControlled !== operatorControlled))
          this.fail('Generated two-tone capability changed during TX');
        this.twoToneLimits = valid ? {maxMs, frequencyHz, maxWatts, operatorControlled} : null;
      }
      if (c.startsWith('saturn_ps_two_tone:')) this.twoToneReply(c);
      if (c.startsWith('saturn_ps_measurement_caps:')) {
        const a = c.slice('saturn_ps_measurement_caps:'.length).split(',');
        const maxMs = Number(a[2]), frequencyHz = Number(a[6]);
        const valid = a.length === 8 && a[0] === '0' && a[1] === 'true' && a[3] === 'rx-bypass'
          && /^[a-f0-9]{64}$/.test(a[4]!) && /^[a-f0-9]{64}$/.test(a[5]!)
          && Number.isSafeInteger(frequencyHz) && frequencyHz >= 1500000 && frequencyHz <= 35000000
          && ((a[7] === 'raw' && maxMs === 1000) || (a[7] === 'summary-only' && maxMs === 45000));
        if (this.requested && (!valid || this.diagnostic?.maxMs !== maxMs
          || this.diagnostic?.frequencyHz !== frequencyHz || this.diagnostic?.captureMode !== a[7]))
          this.fail('Measurement capability changed during TX');
        this.diagnostic = valid ? { maxMs, frequencyHz, captureMode: a[7]! } : null;
      }
      if (c.startsWith('saturn_ps_stimulus_caps:')) {
        const supported = c === `saturn_ps_stimulus_caps:0,${TIMING_PROBE_VERSION}`;
        if (this.measurement?.timing && !supported) this.fail('Timing stimulus capability withdrawn');
        this.timingSupported = supported;
      }
      if (c.startsWith('saturn_ps_native_two_tone_caps:')) {
        const supported = c === `saturn_ps_native_two_tone_caps:0,${NATIVE_TWO_TONE_VERSION}`;
        if (this.requested && supported !== this.nativeTwoToneSupported) this.fail('Native two-tone capability changed during TX');
        this.nativeTwoToneSupported = supported;
      }
      if (c.startsWith('saturn_ps_measurement:')) this.measurementReply(c);
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
        const previousFault = this.owner?.fault;
        this.owner = {serial: BigInt(owner[1]!), stop: BigInt(owner[3]!), phase: owner[2]!, config: owner[4]!, source: owner[5]! as Source,
          clean: owner[6] === 'true', fault: owner[7]!};
        if (this.owner.fault !== 'none') {
          // A tuning commit republishes the SAME latched fault. It must not
          // discard newer RX targets or turn faulted tuning into TX readiness.
          if (this.phase !== 'fault' || previousFault !== this.owner.fault) this.fail(`Owner fault latched: ${this.owner.fault}`);
        }
        else if (this.requested && ['AwaitingAuthority', 'CleaningNative', 'ReadyOff', 'Blocked', 'Closed'].includes(this.owner.phase)) {
          this.invalidate(); this.requested = null; this.phase = 'releasing';
        }
        if (this.twoTone?.complete && this.owner.clean && this.owner.phase === 'ReadyOff' && this.owner.stop > this.twoTone.stop) {
          this.twoTone = null;
        }
        if (!this.measurement && !this.twoTone && this.owner.clean && (this.cleanupAfterStop === null || this.owner.stop > this.cleanupAfterStop)) this.deadline = 0;
        if (this.requested === 'satp' && this.owner.phase === 'Active') {
          this.phase = 'streaming'; this.reason = 'Managed SATP active'; this.deadline = 0;
        }
      }
      const offer = /^saturn_tx_generation:psg1,(off|[a-f0-9]{32}),([1-9][0-9]*)$/.exec(c);
      if (offer) void this.generation(offer[1]!, BigInt(offer[2]!));
    }
    this.refresh(); this.publish();
  }
  private canAcknowledgeFault(): boolean {
    return this.selected && this.operator && this.bridgeReady && this.ackSupported && this.phase === 'fault'
      && this.ackPending === null && this.owner?.phase === 'ReadyOff' && this.ackStop === this.owner.stop
      && this.owner.fault !== 'none' && this.statusPendingWire() === 0;
  }
  private statusPendingWire(): number { return this.awaitingQueue.length + this.transactions.size; }
  /** Explicit acknowledgment only; never arm, retry settings, or start audio. */
  acknowledgeFault(): boolean {
    if (!this.canAcknowledgeFault() || !this.control || !this.owner || this.nextAck === 0xffffffffffffffffn) return false;
    this.ackPending = {id: String(++this.nextAck), stop: this.owner.stop, serial: this.owner.serial, at: this.options.now()};
    this.ackResult = 'Waiting for host fault acknowledgment; output remains OFF';
    try { this.control(`saturn_tx_fault_ack:0,${this.ackPending.id},${this.ackPending.stop};`); }
    catch { this.ackPending = null; this.ackResult = 'Fault acknowledgment transport unavailable'; }
    this.publish(); return this.ackPending !== null;
  }
  private measurementReply(command: string): void {
    const m = this.measurement;
    if (!m) return;
    const args = command.slice('saturn_ps_measurement:'.length).split(',');
    if (args[0] !== '0' || !/^[0-9]+$/.test(args[1] ?? '')) return;
    const id = args[1]!, outcome = args[2];
    if (outcome === 'queued' && m.id === null && id !== '0') m.id = id;
    if ((id === '0' && outcome === 'rejected') || (m.id === id && ['rejected', 'blocked', 'export-rejected'].includes(outcome ?? ''))) {
      this.measurementResult = `Host ${outcome}: ${args[3] ?? 'Unknown'}; cleanup not inferred`;
      this.fail(this.measurementResult); return;
    }
    if (id !== m.id) return;
    if (outcome === 'admitted') this.measurementResult = 'Host admitted MeasurementOnly; physical RF still unverified';
    if (outcome === 'complete' && !m.complete) {
      m.complete = true; this.invalidate(); this.requested = null;
      const blocks = Number(args[3]);
      if (args[4] !== 'none' || !Number.isSafeInteger(blocks) || blocks <= 0)
        this.fail(`Measurement completed with ${args[4] ?? 'invalid result'} / ${args[3]} blocks`);
      else if (this.phase !== 'fault') { this.phase = 'releasing'; this.reason = 'Host cleanup complete; awaiting evidence export'; }
      this.measurementResult = `Host complete: ${args[3]} blocks, ${args[4]}; RF/IMD unqualified`;
      this.deadline = this.options.now() + 15000;
      try { this.control?.(`saturn_ps_measurement:0,export,${id};`); }
      catch { this.fail('Evidence export transport unavailable'); }
    }
    if (outcome === 'exported' && m.complete && args.length === 4) {
      this.measurementResult += ` · Saved on Bridge: ${args[3]}`;
      this.measurement = null; this.deadline = 0;
    }
  }
  private twoToneReply(command: string): void {
    const t = this.twoTone;
    const a = command.slice('saturn_ps_two_tone:'.length).split(',');
    if (!t || a[0] !== '0' || a[1] !== t.id || t.complete) return;
    if (a[2] === 'queued' && a.length === 3) { t.queued = true; return; }
    if (a[2] === 'accepted' && a.length === 3 && t.queued && !t.accepted) {
      t.accepted = true;
      this.deadline = Number.isFinite(t.stopAt) ? t.stopAt + 15000 : 0;
      this.twoToneResult = `Host accepted ${this.psMode === 'auto' ? 'Auto' : this.psMode === 'off' ? 'PS-off' : 'Single'} two-tone trial; physical RF/correction unverified`;
      return;
    }
    if (a[2] === 'rejected' && a.length === 4) {
      this.twoToneResult = `Host rejected two-tone: ${a[3]}`;
      // Cancellation can retire a sealed request before the owner dequeues it.
      // Keep the actual rejection reason, but do not turn our own ordinary
      // release into an authorization fault. This is NOT a cleanup receipt:
      // the newer clean owner publication remains required. Genuine authority,
      // stale/native faults are never suppressed, including later owner faults.
      if (t.stopping && !t.accepted && ['GenerationMismatch','CleanupPending','InvalidConfig'].includes(a[3]!)) {
        t.complete = true;
        this.reason = 'Two-tone request cancelled before admission; waiting for completed owner cleanup';
        if (this.owner?.clean && this.owner.phase === 'ReadyOff' && this.owner.stop > t.stop) {
          this.twoTone = null; this.deadline = 0;
        }
        return;
      }
      this.fail(this.twoToneResult); return;
    }
    if (a[2] === 'complete' && a.length === 4 && t.accepted) {
      t.complete = true; this.invalidate(); this.requested = null;
      this.twoToneResult = `Host two-tone cleanup complete: ${a[3]}; RF/IMD unqualified`;
      if (a[3] !== 'none') this.fail(this.twoToneResult);
      else if (this.phase !== 'fault') { this.phase = 'releasing'; this.reason = 'Waiting for completed owner publication'; }
      if (this.owner?.clean && this.owner.phase === 'ReadyOff' && this.owner.stop > t.stop) {
        this.twoTone = null; this.deadline = 0;
      }
    }
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
      const factory = this.twoTone?.native || this.measurement?.native ? this.options.nativeKeepaliveCapture!
        : this.twoTone ? this.options.twoToneCapture!
        : this.measurement ? (this.measurement.timing ? this.options.timingCapture! : this.options.measurementCapture!) : this.options.capture;
      const capture = await factory(token, (samples, oldest) => this.input(version, token, samples, oldest),
        reason => { if (version === this.version) this.fail(reason); });
      if (version !== this.version || this.token !== token) { capture.close(); return; }
      if (capture.token !== token || !Number.isFinite(capture.historyResetAtMs)
        || capture.historyResetAtMs < requestedAt || capture.historyResetAtMs > this.options.now()) {
        capture.close(); throw new Error('capture_history_not_fresh');
      }
      this.capture = capture; this.lastInputAt = this.options.now();
      this.phase = 'streaming'; this.reason = this.twoTone?.native || this.measurement?.native ? 'Balanced WDSP two-tone active; tagged zero-input keepalive; RF/correction unverified'
        : this.twoTone ? 'Single-fit two-tone PCM active; RF/correction unverified'
        : this.measurement ? 'MeasurementOnly two-tone PCM active; RF unverified' : 'Managed tagged PCM capture active (review)';
      if (!this.measurement && !this.twoTone) this.deadline = 0;
      this.publish();
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
