import type { ManagedPcmStatus } from '../transport/managed-pcm';
import type { SatpState } from '../audio/satp';
import type { CorrectionMonitorState } from './correction-monitor';

type Value = boolean | number | string;
const FIELDS = new Set(['tx_puresignal', 'tx_puresignal_state', 'tx_puresignal_feedback',
  'saturn_ps_measurement_mic_gain_db',
  'tx_puresignal_calibration_count', 'tx_puresignal_correcting', 'tx_puresignal_feedback_packets',
  'tx_puresignal_feedback_gaps']);

/** Receipt-scoped observations, NOT a server observation-time/freshness certificate.
 * No commands, credentials, samples, pairing keys or TX authority live here. */
export class PureSignalReports {
  private generation = -1;
  private values = new Map<string, Value>();
  private receivedAt: number | null = null;
  observe(text: string, generation: number, now: number): void {
    if (!Number.isSafeInteger(generation) || !Number.isFinite(now) || now < 0) return;
    if (generation !== this.generation) {
      this.generation = generation; this.values.clear(); this.receivedAt = null;
    }
    for (const raw of text.split(';')) {
      const [name, payload] = raw.trim().split(':');
      if (!name || !FIELDS.has(name) || !payload) continue;
      const args = payload.split(',');
      if (args.length !== 2 || args[0] !== '0') continue;
      const value = args[1]!;
      if (name === 'saturn_ps_measurement_mic_gain_db') {
        if (!value.trim() || Number(value) !== 0) continue;
        this.values.set(name, 0);
      } else if (name === 'tx_puresignal_state') {
        if (!['off', 'waiting', 'calibrating', 'correcting', 'fault'].includes(value)) continue;
        this.values.set(name, value);
      } else if (name === 'tx_puresignal' || name === 'tx_puresignal_correcting') {
        if (value !== 'true' && value !== 'false') continue;
        this.values.set(name, value === 'true');
      } else {
        const number = Number(value);
        if (!value.trim() || !Number.isSafeInteger(number) || number < 0) continue;
        this.values.set(name, number);
      }
      this.receivedAt = now;
    }
  }
  snapshot(connected: boolean, generation: number, now: number) {
    const available = connected && generation === this.generation;
    const values = available ? Object.fromEntries(this.values) : {};
    return { values, receiptAgeMs: available && this.receivedAt !== null
      ? Math.max(0, now - this.receivedAt) : null, observationAgeVerified: false as const };
  }
}

export interface PsTestState {
  mode?: string;
  connected: boolean; bridgeReady: boolean; connectionGeneration: number;
  remoteClientRole?: string | null; txPhase: string; moxRequested: boolean; txEnabled: boolean;
  pureSignalEnabled: boolean; pureSignalAutoAttenuate: boolean; pureSignalAttenuationDb: number;
  satp?: SatpState;
  rxRecoveryPending?: boolean;
  measurementTiming?: boolean;
}

export function pureSignalTestView(state: PsTestState, managed: ManagedPcmStatus,
  reports: PureSignalReports, now: number, native?: CorrectionMonitorState, transmitting?: boolean) {
  const snapshot = reports.snapshot(state.connected, state.connectionGeneration, now);
  const get = (name: string) => snapshot.values[name];
  const numeric = (name: string) => typeof get(name) === 'number' ? String(get(name)) : 'Unavailable';
  const available = state.connected && state.bridgeReady;
  const nativeFrame = managed.selected ? native?.status(state.connectionGeneration, now, available,
    transmitting ?? state.txEnabled) : null;
  const busy = state.txPhase !== 'rx' || state.moxRequested || state.txEnabled;
  const editable = available && state.remoteClientRole === 'operator' && !busy && !state.rxRecoveryPending
    && (!managed.selected || managed.phase === 'ready');
  const hostEnabled = get('tx_puresignal');
  const correction = get('tx_puresignal_correcting');
  const shared = managed.selected && managed.psCapability && managed.psCapability !== 'unavailable';
  const measurementMode = state.mode ?? 'USB';
  const carrier = ['AM', 'SAM'].includes(measurementMode);
  const timing = state.measurementTiming === true;
  const measurementGain = get('saturn_ps_measurement_mic_gain_db') === 0;
  return {
    editable,
    canConfigure: Boolean(editable && shared),
    canSetFeedbackAttenuation: Boolean(available && state.remoteClientRole === 'operator' && !busy && !state.rxRecoveryPending
      && shared && managed.manualAttenuationSupported && (editable || managed.canAdjustFaultedFeedback)),
    canAutoAdaptation: Boolean(editable && managed.psCapability === 'single' && managed.autoAdaptationSupported),
    adaptationMode: managed.psMode ?? 'unavailable',
    canCalibrate: Boolean(editable && managed.psCapability === 'single'),
    canTwoTone: Boolean(editable && managed.psCapability === 'single' && (['off','single'].includes(managed.psMode ?? '') || (managed.psMode === 'auto' && managed.autoAdaptationSupported))
      && managed.twoToneLimitMs !== undefined && managed.source === 'tci' && ['USB','LSB'].includes(measurementMode)),
    twoTone: managed.twoToneResult ?? 'No generated calibration trial requested',
    canMeasure: Boolean(editable && managed.psCapability === 'measurement-only' && managed.psMode === 'off'
      && managed.source === 'tci' && managed.measurementLimitMs && ['USB','LSB','AM','SAM'].includes(measurementMode)
      && (!timing || (managed.timingSupported === true && ['USB','LSB'].includes(measurementMode)))),
    measurementStimulus: timing ? (managed.timingSupported ? 'Timing Measurement — deterministic multisine, 1000 ms repeat; two-tone IMD unavailable'
      : 'Timing Measurement unavailable — host stimulus capability not advertised')
      : carrier ? 'AM silent carrier — generated zero PCM, no microphone required'
      : managed.nativeTwoToneSupported ? '700/1900 Hz balanced WDSP generator — tagged zero-input keepalive; voice profile preserved'
      : '700/1900 Hz two-tone — generated PCM, no microphone required',
    measurementMicGainDb: measurementGain ? 0 : null,
    measurementGain: measurementGain ? 'Host measurement gain: fixed 0 dB (unity); normal microphone profile preserved.'
      : 'Measurement gain policy unavailable from this host.',
    controlContract: !managed.selected ? 'Select Managed while disconnected, then connect to a shared-owner host. Legacy Direct XDMA cannot calibrate.'
      : managed.psCapability === 'measurement-only' ? 'Host MeasurementOnly: fitting and correction forced OFF. Use the separate attended RF test first. Calibration needs a separately configured shared host and reviewed feedback level/scaling/delay; this button does not unlock them.'
        : managed.psCapability === 'single' ? managed.generatedTrialsOnly
          ? 'Experimental native two-tone-only host. Calibrate prepares one fit without keying; use the separate attended two-tone action. Ordinary PTT/MOX is unavailable.'
          : managed.autoAdaptationSupported ? 'Host supports Single and Auto adaptation. Mode changes configure WDSP; PTT is a separate action. Auto adaptation does not enable Auto ATT.'
            : 'Host single calibration supported. Calibrate configures one fit; PTT is a separate action.'
          : 'Shared PS control capability not advertised; controls unavailable.',
    committedMode: `Host committed mode: ${managed.psMode ?? 'unavailable'} (not proof of calibration)`,
    measurement: managed.measurementResult ?? 'No measurement requested',
    owner: managed.selected ? `Managed selected · ${managed.ownerPhase ?? 'host not negotiated'}`
      : 'Legacy selected · host RF permission is separate',
    source: !available ? 'Unavailable while disconnected'
      : state.satp?.source === 'satp'
        ? `Native SATP · ${state.satp.paired ? 'paired' : 'unpaired'} · ${state.satp.health} (host report)`
        : managed.generatedTrialsOnly ? 'Native balanced two-tone · tagged zero-input keepalive; no microphone' : 'Browser Microphone selected',
    requested: state.pureSignalEnabled ? 'Enable requested' : 'Bypass requested',
    applied: managed.selected ? `Host committed mode: ${managed.psMode ?? 'unavailable'}`
      : typeof hostEnabled === 'boolean' ? `Host reports ${hostEnabled ? 'enabled' : 'disabled'}` : 'No host enable report',
    state: managed.selected ? nativeFrame ? `WDSP state code ${nativeFrame.nativeState} (native readback)`
      : 'Native telemetry unavailable' : String(get('tx_puresignal_state') ?? 'Unavailable'),
    feedback: managed.selected ? nativeFrame?.nativeInfo && nativeFrame.nativeInfo[4]! >= 0
      ? `${nativeFrame.nativeInfo[4]} (dimensionless WDSP units)` : 'Unavailable'
      : numeric('tx_puresignal_feedback'),
    calibrations: managed.selected ? nativeFrame ? String(nativeFrame.calibrations) : 'Unavailable'
      : numeric('tx_puresignal_calibration_count'),
    correcting: managed.selected ? nativeFrame
      ? `WDSP correction ${nativeFrame.nativeApplied ? 'enabled' : 'disabled'} · RF unqualified`
      : 'Unqualified' : typeof correction === 'boolean'
      ? correction ? 'Reported correcting · RF unqualified' : 'Reported bypassed' : 'Unavailable',
    packets: managed.selected ? 'Unavailable' : numeric('tx_puresignal_feedback_packets'),
    gaps: managed.selected ? 'Unavailable' : numeric('tx_puresignal_feedback_gaps'),
    timing: nativeFrame ? 'Fresh WDSP readback · bounded host snapshot age; calibration age and RF acquisition age unknown'
      : snapshot.receiptAgeMs === null ? 'No PS report received in this connection'
      : `Last PS message received ${(snapshot.receiptAgeMs / 1000).toFixed(1)} s ago · observation/delivery age unverified`,
    qualification: 'RF output, feedback calibration and IMD improvement remain unqualified. LP700 power is not supplied by this panel.',
    lockReason: editable ? 'Changes apply only while RX/off. This panel does not key TX.'
      : !available ? 'Connect to a ready Bridge to change PS settings.'
        : state.remoteClientRole !== 'operator' ? 'Read-only viewer: operator authority required.'
          : state.rxRecoveryPending ? 'RX session settings restoration pending; wait for committed settings before TX.'
          : busy ? 'Release PTT/MOX and wait for RX before changing PS settings.'
            : `Managed settings unavailable: ${managed.reason}`,
  };
}
