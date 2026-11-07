import { describe, expect, it } from 'vitest';
import { PureSignalReports, pureSignalTestView, type PsTestState } from '../src/puresignal/test-view';
import type { ManagedPcmStatus } from '../src/transport/managed-pcm';
import { CorrectionMonitorState, type CorrectionFrame } from '../src/puresignal/correction-monitor';

const state = (): PsTestState => ({ connected: true, bridgeReady: true, connectionGeneration: 1,
  remoteClientRole: 'operator', txPhase: 'rx', moxRequested: false, txEnabled: false,
  pureSignalEnabled: false, pureSignalAutoAttenuate: true, pureSignalAttenuationDb: 0 });
const managed = (): ManagedPcmStatus => ({ selected: false, phase: 'disabled', reason: 'Legacy TX selected',
  pendingSettings: 0, lastTransaction: 'No managed settings result', source: 'tci', sentFrames: 0, ownerPhase: null, microphonePreparation: 'cold' });

describe('PS tab presentation without invented measurements', () => {
  it('shows host Auto ATT rather than a checked saved preference on an unsupported host',()=>{
    const s=state(),m={...managed(),selected:true,phase:'ready' as const,psCapability:'single' as const,manualAttenuationSupported:true};
    let view=pureSignalTestView(s,m,new PureSignalReports(),0);
    expect(view.autoAttenuationEnabled).toBe(false);expect(view.canSetAutoAttenuation).toBe(false);
    const capable={...m,autoAttenuationSupported:true,autoAttenuationEnabled:true};
    view=pureSignalTestView(s,capable,new PureSignalReports(),0);
    expect(view.autoAttenuationEnabled).toBe(true);expect(view.canSetAutoAttenuation).toBe(true);
    expect(view.canSetFeedbackAttenuation).toBe(false);
    expect(pureSignalTestView(s,{...capable,autoAttenuationEnabled:false},new PureSignalReports(),0).canSetFeedbackAttenuation).toBe(true);
  });
  it('can turn Auto ATT off after fault cleanup without unlocking unrelated TX controls',()=>{
    const m:ManagedPcmStatus={...managed(),selected:true,phase:'fault',psCapability:'single',autoAttenuationSupported:true,autoAttenuationEnabled:true,canAdjustFaultedAutoAtt:true};
    const view=pureSignalTestView(state(),m,new PureSignalReports(),0);
    expect(view.canSetAutoAttenuation).toBe(true);expect(view.canSetFeedbackAttenuation).toBe(false);
    expect(view.canConfigure).toBe(false);expect(view.canTwoTone).toBe(false);
  });
  it('operator-controlled duration zero keeps Start available without relaxing role or RX gates', () => {
    const m: ManagedPcmStatus={...managed(), selected:true, phase:'ready', psCapability:'single', psMode:'auto',
      autoAdaptationSupported:true, twoToneOperatorControlled:true, twoToneLimitMs:0, twoToneMaxWatts:100};
    const s={...state(), mode:'LSB'};
    expect(pureSignalTestView(s,m,new PureSignalReports(),0).canTwoTone).toBe(true);
    expect(pureSignalTestView({...s,remoteClientRole:'viewer'},m,new PureSignalReports(),0).canTwoTone).toBe(false);
    expect(pureSignalTestView({...s,txEnabled:true},m,new PureSignalReports(),0).canTwoTone).toBe(false);
  });

  it('allows manual feedback attenuation only after host support, while clean/off and operator', () => {
    const s=state(),m={...managed(),selected:true,phase:'ready' as const,psCapability:'single' as const};
    expect(pureSignalTestView(s,m,new PureSignalReports(),0).canSetFeedbackAttenuation).toBe(false);
    const capable={...m,manualAttenuationSupported:true};
    expect(pureSignalTestView(s,capable,new PureSignalReports(),0).canSetFeedbackAttenuation).toBe(true);
    for(const changed of [{...s,txEnabled:true},{...s,remoteClientRole:'viewer'},{...s,connected:false}])
      expect(pureSignalTestView(changed,capable,new PureSignalReports(),0).canSetFeedbackAttenuation).toBe(false);
  });
  it('permits only the negotiated manual feedback adjustment after completed fault cleanup',()=>{
    const s=state();const m:ManagedPcmStatus={...managed(),selected:true,phase:'fault',psCapability:'single',manualAttenuationSupported:true,canAdjustFaultedFeedback:true};
    const v=()=>pureSignalTestView(s,m,new PureSignalReports(),0);
    expect(v().canSetFeedbackAttenuation).toBe(true);expect(v().canConfigure).toBe(false);expect(v().canTwoTone).toBe(false);expect(v().canCalibrate).toBe(false);
    m.canAdjustFaultedFeedback=false;expect(v().canSetFeedbackAttenuation).toBe(false);
    m.canAdjustFaultedFeedback=true;s.txEnabled=true;expect(v().canSetFeedbackAttenuation).toBe(false);
    s.txEnabled=false;s.remoteClientRole='viewer';expect(v().canSetFeedbackAttenuation).toBe(false);
  });
  function nativeReport(keyed = false) {
    const monitor = new CorrectionMonitorState();
    monitor.reset(1);
    monitor.observe('saturn_ps_correction_caps:wdsp-ampview-v1;', 1, 50);
    monitor.current(80, true, keyed);
    monitor.request('1', 1, 100);
    monitor.observe('saturn_ps_spectrum_clock:1,10,100000000;', 1, 120);
    const frame: CorrectionFrame = {
      format: 'saturn-ps-correction-v1', boot: '10', serial: '1', snapshotNs: '250000000', publishedNs: '300000000',
      sourcePin: 'synthetic-not-rf', backend: 'DirectXdma', centerHz: 7200000, commandedWatts: 5, attenuationCommandDb: 0,
      psRequested: true, keyed, nativeApplied: true, calibrations: 2, nativeState: 7,
      nativeSamples: 0, phaseReferenceDeg: 0, diagnosticDrops: '0', curveAge: 'unknown', rfImdQualified: false,
      measured: [], correction: [],
    };
    monitor.observe(`saturn_ps_correction:${JSON.stringify(frame)};`, 1, 400);
    return monitor;
  }
  it('shows verified native state/count/run flag without claiming RF improvement or fit age', () => {
    const m = { ...managed(), selected: true };
    const view = pureSignalTestView(state(), m, new PureSignalReports(), 400, nativeReport());
    expect(view.state).toContain('WDSP state code 7');
    expect(view.calibrations).toBe('2');
    expect(view.correcting).toBe('WDSP correction enabled · RF unqualified');
    expect(view.timing).toContain('calibration age and RF acquisition age unknown');
    expect(view.feedback).toBe('Unavailable');
    expect(view.qualification).toContain('remain unqualified');
  });
  it('does not use old, disconnected, other-session or stalled native snapshots as live PS status', () => {
    const m = { ...managed(), selected: true };
    for (const [s, now] of [[{ ...state(), connected: false }, 400],
      [{ ...state(), connectionGeneration: 2 }, 400], [state(), 1800]] as const) {
      const view = pureSignalTestView(s, m, new PureSignalReports(), now, nativeReport());
      expect(view.calibrations).toBe('Unavailable');
      expect(view.correcting).toBe('Unqualified');
    }
  });
  it('requires post-key native evidence and preserves the legacy report source', () => {
    const m = { ...managed(), selected: true };
    const keyed = { ...state(), txEnabled: true, txPhase: 'keyed' };
    expect(pureSignalTestView(keyed, m, new PureSignalReports(), 400, nativeReport()).calibrations).toBe('Unavailable');
    expect(pureSignalTestView(keyed, m, new PureSignalReports(), 400, nativeReport(true)).calibrations).toBe('2');
    const reports = new PureSignalReports();
    reports.observe('tx_puresignal_calibration_count:0,3;', 1, 390);
    expect(pureSignalTestView(state(), managed(), reports, 400, nativeReport()).calibrations).toBe('3');
  });
  it('labels native feedback in WDSP units and makes it unavailable after staleness', () => {
    const monitor=nativeReport();
    const info=Array(16).fill(0);info[4]=152;info[5]=2;info[15]=7;
    monitor.observe(`saturn_ps_correction:${JSON.stringify({...monitor.frame,serial:'2',nativeInfo:info})};`,1,410);
    const m={...managed(),selected:true};
    expect(pureSignalTestView(state(),m,new PureSignalReports(),410,monitor).feedback).toBe('152 (dimensionless WDSP units)');
    expect(pureSignalTestView(state(),m,new PureSignalReports(),1800,monitor).feedback).toBe('Unavailable');
  });
  it('offers PS-off baseline and prepared calibration with negotiated limits and SSB', () => {
    const s={...state(),mode:'USB'};
    const m: ManagedPcmStatus={...managed(),selected:true,phase:'ready',psCapability:'single',psMode:'single'};
    const view=()=>pureSignalTestView(s,m,new PureSignalReports(),0);
    expect(view().canTwoTone).toBe(false);m.twoToneLimitMs=45000;
    expect(view().canTwoTone).toBe(true);m.psMode='off';expect(view().canTwoTone).toBe(true);
    delete m.psMode;expect(view().canTwoTone).toBe(false);m.psMode='single';
    s.mode='AM';expect(view().canTwoTone).toBe(false);
    s.mode='LSB';m.psCapability='measurement-only';expect(view().canTwoTone).toBe(false);
  });
  it('requires negotiated timing support and SSB, without granting calibration', () => {
    const s={...state(),mode:'USB',measurementTiming:true};
    const m: ManagedPcmStatus={...managed(),selected:true,phase:'ready',psCapability:'measurement-only',psMode:'off',measurementLimitMs:45000};
    const view=()=>pureSignalTestView(s,m,new PureSignalReports(),1);
    expect(view().canMeasure).toBe(false); expect(view().measurementStimulus).toContain('unavailable');
    m.timingSupported=true; expect(view().canMeasure).toBe(true); expect(view().canCalibrate).toBe(false);
    expect(view().measurementStimulus).toContain('two-tone IMD unavailable');
    s.mode='LSB'; expect(view().canMeasure).toBe(true);
    for(const mode of ['AM','SAM','FM']) {s.mode=mode; expect(view().canMeasure).toBe(false);}
  });
  it('shows unity measurement gain only when the current host advertises it', () => {
    const reports = new PureSignalReports();
    const s = state();
    expect(pureSignalTestView(s, managed(), reports, 1).measurementMicGainDb).toBeNull();
    reports.observe('saturn_ps_measurement_mic_gain_db:0,0;', 1, 10);
    const view = pureSignalTestView(s, managed(), reports, 11);
    expect(view.measurementMicGainDb).toBe(0);
    expect(view.measurementGain).toContain('normal microphone profile preserved');
    s.connected = false;
    expect(pureSignalTestView(s, managed(), reports, 12).measurementMicGainDb).toBeNull();
    s.connected = true; s.connectionGeneration = 2;
    expect(pureSignalTestView(s, managed(), reports, 12).measurementMicGainDb).toBeNull();
    reports.observe('saturn_ps_measurement_mic_gain_db:0,-12;', 2, 12);
    expect(pureSignalTestView(s, managed(), reports, 12).measurementMicGainDb).toBeNull();
  });
  it('identifies the native AM carrier test separately from SSB tones and calibration', () => {
    const s=state(); s.mode='AM';
    const m=managed();m.selected=true;m.phase='ready';m.psCapability='measurement-only';m.psMode='off';m.measurementLimitMs=45000;
    const view=pureSignalTestView(s,m,new PureSignalReports(),1);
    expect(view.canMeasure).toBe(true);expect(view.canCalibrate).toBe(false);
    expect(view.measurementStimulus).toContain('AM silent carrier');
    expect(view.controlContract).toContain('feedback level/scaling/delay');
    s.mode='USB';expect(pureSignalTestView(s,m,new PureSignalReports(),1).measurementStimulus).toContain('700/1900');
    s.mode='FM';expect(pureSignalTestView(s,m,new PureSignalReports(),1).canMeasure).toBe(false);
  });
  it('waits for scheduled RX restoration, even when the TX owner is already ready', () => {
    const s = state(); s.rxRecoveryPending = true;
    const m = managed(); m.selected = true; m.phase = 'ready'; m.psCapability = 'measurement-only'; m.psMode = 'off'; m.measurementLimitMs = 1000;
    const view = pureSignalTestView(s, m, new PureSignalReports(), 1);
    expect(view.canMeasure).toBe(false); expect(view.canConfigure).toBe(false);
    expect(view.lockReason).toContain('restoration pending');
    s.rxRecoveryPending = false;
    expect(pureSignalTestView(s, m, new PureSignalReports(), 1).canMeasure).toBe(true);
  });
  it('does not turn default state zeros into received measurements', () => {
    const view = pureSignalTestView(state(), managed(), new PureSignalReports(), 1);
    expect(view.state).toBe('Unavailable');
    expect(view.feedback).toBe('Unavailable');
    expect(view.calibrations).toBe('Unavailable');
    expect(view.correcting).toBe('Unavailable');
    expect(view.applied).toBe('No host enable report');
  });
  it('keeps a genuine reported zero distinct from no report', () => {
    const reports = new PureSignalReports();
    reports.observe('tx_puresignal_feedback:0,0;tx_puresignal_calibration_count:0,0;tx_puresignal_correcting:0,false;', 1, 10);
    const view = pureSignalTestView(state(), managed(), reports, 25);
    expect(view.feedback).toBe('0'); expect(view.calibrations).toBe('0');
    expect(view.correcting).toBe('Reported bypassed');
    expect(view.timing).toContain('observation/delivery age unverified');
  });
  it('separates an optimistic request from the host enable report', () => {
    const s = state(); s.pureSignalEnabled = true;
    const reports = new PureSignalReports(); reports.observe('tx_puresignal:0,false;', 1, 10);
    const view = pureSignalTestView(s, managed(), reports, 12);
    expect(view.requested).toBe('Enable requested'); expect(view.applied).toBe('Host reports disabled');
  });
  it('clears report availability on disconnect and generation change', () => {
    const reports = new PureSignalReports(); reports.observe('tx_puresignal_state:0,correcting;', 1, 10);
    const s = state(); s.connected = false;
    expect(pureSignalTestView(s, managed(), reports, 20).state).toBe('Unavailable');
    s.connected = true; s.connectionGeneration = 2;
    expect(pureSignalTestView(s, managed(), reports, 20).state).toBe('Unavailable');
    reports.observe('ready;', 2, 20);
    expect(pureSignalTestView(s, managed(), reports, 20).state).toBe('Unavailable');
  });
  it('does not accept malformed, foreign-receiver or nonfinite reports', () => {
    const reports = new PureSignalReports();
    reports.observe('tx_puresignal_feedback:1,22;tx_puresignal_feedback:0,NaN;tx_puresignal_feedback:0,-1;tx_puresignal_state:0,fresh;tx_puresignal_correcting:0,1;tx_puresignal:0,true,extra;', 1, 10);
    expect(reports.snapshot(true, 1, 20).values).toEqual({});
  });
  it('does not mistake other incoming commands for PS status', () => {
    const reports = new PureSignalReports(); reports.observe('saturn_satp_key:secret;trx:0,true;', 1, 10);
    expect(reports.snapshot(true, 1, 20).values).toEqual({});
  });
  it('never claims that managed native telemetry or RF qualification exists', () => {
    const reports = new PureSignalReports(); reports.observe('tx_puresignal_state:0,correcting;tx_puresignal_feedback:0,100;', 1, 10);
    const m = managed(); m.selected = true; m.phase = 'ready'; m.ownerPhase = 'ReadyOff';
    const view = pureSignalTestView(state(), m, reports, 20);
    expect(view.state).toBe('Native telemetry unavailable'); expect(view.feedback).toBe('Unavailable');
    expect(view.correcting).toBe('Unqualified'); expect(view.qualification).toContain('remain unqualified');
  });
  it.each(['viewer', null])('keeps %s read-only', role => {
    const s = state(); s.remoteClientRole = role;
    expect(pureSignalTestView(s, managed(), new PureSignalReports(), 0).editable).toBe(false);
  });
  it.each(['armed', 'keyed'])('requires release before editing in %s', phase => {
    const s = state(); s.txPhase = phase;
    expect(pureSignalTestView(s, managed(), new PureSignalReports(), 0).editable).toBe(false);
  });
  it('waits for actual managed settings/cleanup readiness', () => {
    const m = managed(); m.selected = true; m.phase = 'releasing'; m.reason = 'Cleanup pending';
    const view = pureSignalTestView(state(), m, new PureSignalReports(), 0);
    expect(view.editable).toBe(false); expect(view.lockReason).toContain('Cleanup pending');
  });
  it('shows SATP pairing as a report, not proof of microphone signal or ownership', () => {
    const s = state(); s.satp = { source: 'satp', enabled: true, paired: true, ready: false,
      health: 'waiting', peakDb: -140, bufferFrames: 0, missingPackets: 0, generation: 1, pending: false, receivedAt: 0 };
    const view = pureSignalTestView(s, managed(), new PureSignalReports(), 0);
    expect(view.source).toBe('Native SATP · paired · waiting (host report)');
    expect(JSON.stringify(view)).not.toContain('secret');
  });
});
