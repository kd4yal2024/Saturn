import { readFileSync, writeFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it, vi } from 'vitest';
import { createAppState } from '../src/state/app-state';
import { radioPrefsFromState, displayPrefsFromState } from '../src/state/prefs-from-state';
import { applyRadioPrefsToState, applyDisplayPrefsToState } from '../src/state/apply-prefs';
import { normalizeBandMemory, normalizeRadioPrefs } from '../src/settings/normalize';
import { buildAllRadioPrefsCommands } from '../src/tci/commands';
import { managedSettingsMessage } from '../src/transport/managed-pcm';
import { bandKeyForFrequency } from '../src/radio/band';
import { PureSignalReports, pureSignalTestView } from '../src/puresignal/test-view';
import type { ManagedPcmStatus } from '../src/transport/managed-pcm';
import { clampPureSignalAttenuationDb } from '../src/settings/normalize';

const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
function source(name: string) {
  const start = template.indexOf(`    function ${name}(`);
  if (start < 0) throw Error(`Missing ${name}`);
  const end = template.indexOf('\n    }', start);
  return template.slice(start, end + 6);
}
function fixture() {
  const state = createAppState();
  Object.assign(state, { vfoA: 7_200_000, dds: 7_200_000, bandMemory: {}, connected: true,
    bridgeReady: true, remoteClientRole: 'operator', rxHeadphonesSupported: true, txMonitorSupported: true });
  const storage = new Map<string, string>();
  const sent: string[] = [], logs: string[] = [], tasks: Array<() => void> = [];
  const names = ['currentRadioPrefs', 'applyRadioPrefsObject', 'currentDisplayPrefs',
    'currentBandMemoryEntry', 'rememberCurrentBandSettings', 'applyBandMemoryForFrequency',
    'setFrequency', 'recallFrequencyEntryBand', 'saveRadioPrefs', 'scheduleRemoteSettingsSave', 'restoreLocalAudioPreferences',
    'sendCurrentRadioPrefsToBridge'];
  const context = {
    state, console, receiveOnlyValidationSession: false, REMOTE_FIXED_RX_ADC: 0,
    BAND_MEMORY_KEY: 'bandMemory', RADIO_PREFS_KEY: 'radioPrefs', REMOTE_SETTINGS_SAVE_DEBOUNCE_MS: 750,
    remoteSettingsSaveTimer: null,
    localStorage: { setItem: (k: string, v: string) => storage.set(k, v) },
    window: { setTimeout: (f: () => void) => { tasks.push(f); return tasks.length; }, clearTimeout: vi.fn() },
    _next: { radioPrefsFromState, displayPrefsFromState, applyRadioPrefsToState,
      buildAllRadioPrefsCommands, managedSettingsMessage },
    activeVfoFrequency: () => state.activeVfo === 'B' ? state.vfoB : state.vfoA,
    bandKeyForFrequency, applyDisplayPrefsObject: (prefs: Record<string, unknown>) => applyDisplayPrefsToState(prefs, state),
    logEvent: (s: string) => logs.push(s), $: () => null,
    currentRemoteSettings: () => ({ radioPrefs: radioPrefsFromState(state), bandMemory: state.bandMemory }),
    persistLocalSettingsFallback: vi.fn(), saveRemoteSettingsToServer: vi.fn(),
    shiftDisplayForCenterChange: vi.fn(), updateTuneVisuals: vi.fn(), updateUi: vi.fn(),
    selectReceiveMode: (mode: string) => { state.mode = mode; return true; },
    resumeRxMediaAfterBandChange: vi.fn(), closeFrequencyEntry: vi.fn(),
    sendTci: (s: string) => sent.push(s), managedPcmSelected: () => true,
    applyTxSourceOverrides: vi.fn(), generatedTxSourceActive: () => false,
    managedPcm: { status: () => ({ manualAttenuationSupported: true, autoAdaptationSupported: true }) },
  };
  const api = runInNewContext(`${names.map(source).join('\n')}\n({${names.join(',')}})`, context);
  return { state, storage, sent, logs, tasks, context, api };
}

describe('automatic band operating settings', () => {
  it('renders actual Auto ATT state, restores the manual preset and preserves an active slider edit',()=>{
    const f=fixture();f.state.pureSignalAttenuationDb=18;
    const m:ManagedPcmStatus={selected:true,phase:'ready',reason:'ready',pendingSettings:0,lastTransaction:'applied',
      source:'tci',sentFrames:0,ownerPhase:'ReadyOff',microphonePreparation:'ready',psCapability:'single',manualAttenuationSupported:true};
    const slider={value:'18',disabled:false},checkbox={checked:true,disabled:false},readout={textContent:''},label={textContent:''},status={textContent:''};
    const nodes:Record<string,unknown>={'setup-tx-puresignal-attenuation':slider,'setup-tx-puresignal-auto-attenuate':checkbox,
      'setup-tx-puresignal-attenuation-readout':readout,'setup-tx-puresignal-auto-attenuate-label':label,
      'setup-ps-auto-att-status':status,'setup-ps-attended':{closest:()=>({hidden:false})}};
    const doc:{activeElement:unknown}={activeElement:null};
    const sync=runInNewContext(`${source('syncPureSignalPanel')}\nsyncPureSignalPanel`,{state:f.state,document:doc,$:(id:string)=>nodes[id]??null,
      pureSignalPanelView:()=>pureSignalTestView(f.state,m,new PureSignalReports(),0),managedPcm:{status:()=>m},managedPcmSelected:()=>true,
      renderPsHealthMonitor:vi.fn(),performance:{now:()=>0},clampPureSignalAttenuationDb});
    sync();expect(checkbox.checked).toBe(false);expect(checkbox.disabled).toBe(true);expect(label.textContent).toContain('unavailable');
    m.autoAttenuationSupported=true;m.autoAttenuationEnabled=true;
    m.autoAttStatus={phase:'stable',db:12,appliedDb:12,info:152,frequencyHz:7_200_000,keyed:true};
    sync();expect(checkbox.checked).toBe(true);expect(checkbox.disabled).toBe(false);
    f.state.txPhase='keyed';m.phase='streaming';m.ownerPhase='Active';sync();expect(checkbox.disabled).toBe(true);
    expect(slider.value).toBe('12');expect(slider.disabled).toBe(true);expect(f.state.pureSignalAttenuationDb).toBe(18);
    f.state.txPhase='rx';m.phase='ready';m.ownerPhase='ReadyOff';m.autoAttenuationEnabled=false;sync();expect(slider.value).toBe('18');expect(slider.disabled).toBe(false);
    doc.activeElement=slider;slider.value='23';readout.textContent='23 dB requested';sync();
    expect(slider.value).toBe('23');expect(readout.textContent).toBe('23 dB requested');
  });
  it('saves the actual Auto ATT checkbox per band and sends it only to a capable shared host',()=>{
    const f=fixture();
    f.context.managedPcm.status=()=>({manualAttenuationSupported:true,autoAdaptationSupported:true,autoAttenuationSupported:true});
    const marker='$("setup-tx-puresignal-auto-attenuate").addEventListener("change", () => {';
    const start=template.indexOf(marker)+marker.length,end=template.indexOf('\n      });',start);
    let checked=true,allowed=true;
    const change=runInNewContext(`() => {${template.slice(start,end)}\n}`,{...f.context,...f.api,$:()=>({checked}),pureSignalPanelView:()=>({canSetAutoAttenuation:allowed}),syncSetupMenuFields:vi.fn()});
    f.state.pureSignalAttenuationDb=18;change();
    expect(f.sent.at(-1)).toBe('tx_puresignal_auto_attenuate:0,true;');
    f.api.setFrequency(14_200_000);checked=false;f.state.pureSignalAttenuationDb=7;change();
    f.sent.length=0;f.api.setFrequency(7_200_000);
    expect(f.state.pureSignalAutoAttenuate).toBe(true);expect(f.state.pureSignalAttenuationDb).toBe(18);
    expect(f.sent.join('')).toContain('tx_puresignal_auto_attenuate:0,true;');
    f.api.setFrequency(14_200_000);expect(f.state.pureSignalAutoAttenuate).toBe(false);expect(f.state.pureSignalAttenuationDb).toBe(7);
    const saved=f.storage.get('bandMemory');allowed=false;checked=true;change();
    expect(f.storage.get('bandMemory')).toBe(saved);expect(f.state.pureSignalAutoAttenuate).toBe(false);
  });
  it('saves changes immediately and restores distinct 40m / 20m setups through actual tuning functions', () => {
    const f = fixture();
    Object.assign(f.state, { mode: 'LSB', txDrive: 60, txMicGainDb: 0, rxAntenna: 2,
      rxAttenuationDb: 10, rxFilterShiftHz: -150, rxSsqlEnabled: true, rxSsqlThreshold: 37,
      preferredRxHeadphones: true, preferredTxMonitor: true, txMonitorLevelDb: -18,
      cfcEnabled: true, cfcPrecomp: 4, cfcBands: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
      txEqBands: [0, 1, -2, 3, -4, 5, -6, 7, -8, 9, -10], waterfallPalette: 'ember', displayZoom: 4,
      pureSignalEnabled: true, preferredPureSignalMode: 'auto', twoToneEnabled: true,
      txOpusOverrideEnabled: true });
    f.api.saveRadioPrefs();
    const saved = normalizeBandMemory(JSON.parse(f.storage.get('bandMemory')!));
    expect(saved['40m']?.radioPrefs.twoToneEnabled).toBe(false);
    expect(saved['40m']?.radioPrefs.cfcEnabled).toBe(true);
    expect(saved['40m']?.radioPrefs.txEqBands).toEqual(f.state.txEqBands);
    f.api.setFrequency(14_200_000);
    Object.assign(f.state, { mode: 'USB', txDrive: 25, txMicGainDb: 6, rxAntenna: 1,
      rxAttenuationDb: 0, rxFilterShiftHz: 300, rxSsqlEnabled: false, preferredRxHeadphones: false,
      preferredTxMonitor: false, cfcEnabled: false, cfcPrecomp: 0, waterfallPalette: 'ice', displayZoom: 8 });
    f.api.saveRadioPrefs();
    f.api.setFrequency(7_210_000);
    expect(f.state.mode).toBe('LSB'); expect(f.state.txDrive).toBe(60);
    expect(f.state.txMicGainDb).toBe(0); expect(f.state.rxAntenna).toBe(2);
    expect(f.state.rxAttenuationDb).toBe(10); expect(f.state.rxFilterShiftHz).toBe(-150);
    expect(f.state.rxSsqlEnabled).toBe(true); expect(f.state.rxSsqlThreshold).toBe(37);
    expect(f.state.preferredRxHeadphones).toBe(true); expect(f.state.preferredTxMonitor).toBe(true);
    expect(f.state.txMonitorLevelDb).toBe(-18); expect(f.state.cfcEnabled).toBe(true);
    expect(f.state.preferredPureSignalMode).toBe('auto');
    expect(f.state.waterfallPalette).toBe('ember'); expect(f.state.displayZoom).toBe(4);
    expect(f.state.vfoA).toBe(7_210_000); // tuning destination, not old saved frequency
    const commands = f.sent.join('');
    expect(commands).toContain('tx_drive:0,60;'); expect(commands).toContain('tx_mic_gain:0,0.0;');
    expect(commands).toContain('rx_attenuation:0,10;'); expect(commands).toContain('rx_headphones:0,true;');
    expect(commands).toContain('tx_monitor:0,true;'); expect(commands).toContain('tx_puresignal_mode:0,auto;');
    expect(commands).not.toContain('tx_two_tone:0,true;'); expect(commands).not.toContain('trx:0,true');
    f.api.setFrequency(14_250_000);
    expect(f.state.mode).toBe('USB'); expect(f.state.txDrive).toBe(25);
    expect(f.state.rxFilterShiftHz).toBe(300); expect(f.state.preferredRxHeadphones).toBe(false);
    expect(f.state.waterfallPalette).toBe('ice'); expect(f.state.displayZoom).toBe(8);
  });

  it('does not overwrite the destination memory before band-button or keypad recall applies it', () => {
    const f = fixture();
    f.state.bandMemory = normalizeBandMemory({ '20m': { frequency: 14_225_000,
      radioPrefs: { mode: 'USB', txDrive: 75, txMicGainDb: -3, rxAttenuationDb: 20,
        cfcEnabled: true, cfcPrecomp: 7, cfcBands: Array(11).fill(6) }, displayZoom: 6 } });
    const band = template.lastIndexOf('document.querySelectorAll("#band-grid .band-btn")');
    const callbackStart = template.indexOf('button.addEventListener("click", () => {', band)
      + 'button.addEventListener("click", () => {'.length;
    const callbackEnd = template.indexOf('\n        });', callbackStart);
    const recall = runInNewContext(`button => {${template.slice(callbackStart, callbackEnd)}\n}`, {
      ...f.context, ...f.api,
    });
    recall({ dataset: { band: '14200000', mode: 'USB' } });
    expect(f.state.vfoA).toBe(14_225_000); expect(f.state.txDrive).toBe(75);
    expect(f.state.rxAttenuationDb).toBe(20); expect(f.state.cfcPrecomp).toBe(7);
    expect(f.state.displayZoom).toBe(6);
    f.api.recallFrequencyEntryBand({ dataset: { freqBand: '7200000', freqMode: 'LSB' } });
    expect(f.state.txDrive).toBe(10); expect(f.state.vfoA).toBe(7_200_000);
    f.api.recallFrequencyEntryBand({ dataset: { freqBand: '14200000', freqMode: 'USB' } });
    expect(f.state.txDrive).toBe(75); expect(f.state.txMicGainDb).toBe(-3);
  });

  it('round-trips a persisted band into a fresh page without treating saved audio choices as host acknowledgement', () => {
    const f = fixture();
    Object.assign(f.state, { preferredRxHeadphones: true, preferredTxMonitor: true, txMonitorLevelDb: -12,
      rxAttenuationDb: 17, rxFilterShiftHz: 240, preferredPureSignalMode: 'auto', pureSignalEnabled: true });
    f.api.saveRadioPrefs();
    const next = fixture(); next.state.bandMemory = normalizeBandMemory(JSON.parse(f.storage.get('bandMemory')!));
    next.api.applyBandMemoryForFrequency(7_200_000);
    expect(next.state.rxHeadphonesEnabled).toBe(false); expect(next.state.txMonitorEnabled).toBe(false);
    expect(next.state.preferredRxHeadphones).toBe(true);
    next.state.txMonitorLevelDb = -30; // initial host snapshot must not erase saved level
    next.api.restoreLocalAudioPreferences();
    expect(next.sent).toEqual(['rx_headphones:0,true;', 'tx_monitor_level:0,-12;', 'tx_monitor:0,true;']);
    expect(next.api.currentRadioPrefs().pureSignalMode).toBe('auto');
    next.state.rxHeadphonesSupported = false; next.state.txMonitorSupported = false;
    next.sent.length = 0; next.api.restoreLocalAudioPreferences(); expect(next.sent).toEqual([]);
    expect(next.api.currentRadioPrefs().rxHeadphonesEnabled).toBe(true);
  });

  it('keeps viewer/receive-validation sessions from rewriting band memories', () => {
    const f = fixture(); f.state.remoteClientRole = 'viewer'; f.api.saveRadioPrefs();
    expect(f.storage.has('bandMemory')).toBe(false);
    const valid = fixture(); valid.context.receiveOnlyValidationSession = true;
    valid.api.rememberCurrentBandSettings(); expect(valid.storage.has('bandMemory')).toBe(false);
  });

  it('restores managed TX prefs only for an explicit recall; reconnect keeps the host TX configuration', () => {
    const f = fixture(); f.api.sendCurrentRadioPrefsToBridge();
    expect(f.sent.join('')).not.toContain('tx_drive:');
    f.sent.length = 0; f.api.sendCurrentRadioPrefsToBridge(undefined, { restoreTx: true });
    expect(f.sent.join('')).toContain('tx_drive:0,10;');
  });

  it('preserves a zero-dB gate threshold and distinct PS Auto/Off settings', () => {
    const f = fixture();
    Object.assign(f.state, { txNoiseGateThresholdDb: 0, preferredPureSignalMode: 'auto', pureSignalEnabled: true });
    f.api.saveRadioPrefs();
    const memory = normalizeBandMemory(JSON.parse(f.storage.get('bandMemory')!));
    expect(memory['40m']?.radioPrefs.txNoiseGateThresholdDb).toBe(0);
    f.state.bandMemory = memory; f.api.applyBandMemoryForFrequency(7_200_000);
    expect(f.state.txNoiseGateThresholdDb).toBe(0); expect(f.state.preferredPureSignalMode).toBe('auto');
    expect(normalizeRadioPrefs({ pureSignalMode: 'off', pureSignalEnabled: true }).pureSignalEnabled).toBe(false);
    expect(normalizeRadioPrefs({ pureSignalMode: 'auto', pureSignalEnabled: false }).pureSignalEnabled).toBe(true);
  });

  it('persists distinct PS mode, Auto ATT preference and manual attenuation per band across a fresh page', () => {
    const f = fixture();
    Object.assign(f.state, { pureSignalEnabled: true, preferredPureSignalMode: 'auto',
      pureSignalAutoAttenuate: true, pureSignalAttenuationDb: 17 });
    f.api.saveRadioPrefs();
    f.api.setFrequency(14_200_000);
    Object.assign(f.state, { pureSignalEnabled: false, preferredPureSignalMode: 'off',
      pureSignalAutoAttenuate: false, pureSignalAttenuationDb: 0 });
    f.api.saveRadioPrefs();

    const next = fixture();
    next.state.bandMemory = normalizeBandMemory(JSON.parse(f.storage.get('bandMemory')!));
    Object.assign(next.state, { vfoA: 3_899_000, dds: 3_899_000 });
    for (const [frequency, mode, autoAtt, attenuation] of [
      [7_200_000, 'auto', true, 17], [14_200_000, 'off', false, 0],
      [7_210_000, 'auto', true, 17],
    ] as const) {
      next.api.setFrequency(frequency);
      expect(next.state.preferredPureSignalMode).toBe(mode);
      expect(next.state.pureSignalEnabled).toBe(mode !== 'off');
      expect(next.state.pureSignalAutoAttenuate).toBe(autoAtt);
      expect(next.state.pureSignalAttenuationDb).toBe(attenuation);
      expect(next.sent.join('')).toContain(`tx_puresignal_attenuation:0,${attenuation};`);
    }
    // A saved preference is not a fabricated shared-host Auto ATT capability,
    // and recalling settings must not start two-tone or key the radio.
    expect(next.sent.join('')).not.toContain('tx_puresignal_auto_attenuate:');
    expect(next.sent.join('')).not.toMatch(/(?:trx|tx_two_tone):0,true/);
  });

  it('saves the actual PS attenuation slider choice before any host echo or band change', () => {
    const f = fixture();
    const marker = '$("setup-tx-puresignal-attenuation").addEventListener("change", () => {';
    const start = template.indexOf(marker) + marker.length;
    const end = template.indexOf('\n      });', start);
    let value = '23', allowed = true;
    const sync = vi.fn();
    const change = runInNewContext(`() => {${template.slice(start, end)}\n}`, {
      ...f.context, ...f.api, $: () => ({ value }),
      pureSignalPanelView: () => ({ canSetFeedbackAttenuation: allowed }),
      syncSetupMenuFields: sync,
    });
    change();
    expect(f.sent).toEqual(['tx_puresignal_attenuation:0,23;']);
    expect(normalizeBandMemory(JSON.parse(f.storage.get('bandMemory')!))['40m']?.radioPrefs.pureSignalAttenuationDb).toBe(23);
    expect(f.state.pureSignalAttenuationDb).toBe(23);
    f.api.setFrequency(14_200_000);
    value = '0'; change();
    f.api.setFrequency(7_200_000);
    expect(f.state.pureSignalAttenuationDb).toBe(23);
    f.api.setFrequency(14_200_000);
    expect(f.state.pureSignalAttenuationDb).toBe(0);

    const saved = f.storage.get('bandMemory');
    for (const invalid of ['32', '-1', '1.5', 'NaN']) {
      value = invalid; change();
    }
    allowed = false; value = '31'; change();
    expect(f.storage.get('bandMemory')).toBe(saved);
    expect(f.state.pureSignalAttenuationDb).toBe(0);
    expect(sync).toHaveBeenCalledTimes(5);
  });

  it('the legacy PS enable checkbox updates the same saved band mode as the PS tab', () => {
    const f = fixture();
    f.state.preferredPureSignalMode = 'auto'; f.state.pureSignalEnabled = true;
    const start = template.indexOf('$("setup-tx-puresignal-enabled").addEventListener("change", () => {')
      + '$("setup-tx-puresignal-enabled").addEventListener("change", () => {'.length;
    const end = template.indexOf('\n      });', start);
    let checked = false;
    const change = runInNewContext(`() => {${template.slice(start, end)}\n}`, {
      ...f.context, ...f.api, $: () => ({ checked }), pureSignalPanelView: () => ({ canCalibrate: true }),
      syncSetupMenuFields: vi.fn(), managedPcm: { preparePureSignal: vi.fn() },
    });
    change(); expect(f.api.currentRadioPrefs().pureSignalMode).toBe('off');
    checked = true; change(); expect(f.api.currentRadioPrefs().pureSignalMode).toBe('single');
  });

  it('keeps loaded memories intact during disconnected startup/profile saves while allowing explicit offline edits', () => {
    const f = fixture(); f.state.connected = false; f.state.bridgeReady = false;
    const memory = normalizeBandMemory({ '40m': { frequency: 7_250_000,
      radioPrefs: { mode: 'LSB', txDrive: 60 } } });
    f.state.bandMemory = memory;
    Object.assign(f.state, { vfoA: 14_200_000, dds: 14_200_000, mode: 'LSB', txDrive: 60 });
    f.api.scheduleRemoteSettingsSave(); f.tasks.at(-1)!();
    expect(f.state.bandMemory).toEqual(memory);
    expect(f.state.bandMemory['20m']).toBeUndefined();
    f.state.txDrive = 25; f.api.saveRadioPrefs();
    expect(normalizeBandMemory(JSON.parse(f.storage.get('bandMemory')!))['20m']?.radioPrefs.txDrive).toBe(25);
  });

  it('exports the actual 80m → 40m → 80m recall messages for the native runtime contract test', () => {
    const result: Record<string, Array<{band: string; frequency: number; watts: number; mic: number; messages: string[]}>> = {};
    for (const backend of ['direct', 'p2']) {
      const f = fixture(), rate = backend === 'direct' ? 384000 : 192000;
      Object.assign(f.state, { vfoA: 3_899_000, vfoB: 3_899_000, dds: 3_899_000,
        sampleRate: rate, mode: 'LSB', txDrive: 4, txMicGainDb: 0,
        rxHeadphonesSupported: false, txMonitorSupported: false });
      f.api.saveRadioPrefs();
      f.state.bandMemory['40m'] = { frequency: 7_218_000,
        radioPrefs: normalizeRadioPrefs({ ...radioPrefsFromState(f.state), rxAntenna: 2, rxAttenuationDb: 10, sampleRate: rate, mode: 'LSB', txDrive: 5,
          txMicGainDb: 3, pureSignalAutoAttenuate: true }), displayPrefs: {}, displayZoom: 3 };
      result[backend] = [];
      for (const [band, button, frequency, watts, mic] of [
        ['40m', '7200000', 7_218_000, 5, 3], ['80m', '3750000', 3_899_000, 4, 0],
        ['40m', '7200000', 7_218_000, 5, 3], ['80m', '3750000', 3_899_000, 4, 0],
      ] as const) {
        f.sent.length = 0;
        f.api.recallFrequencyEntryBand({ dataset: { freqBand: button, freqMode: 'LSB' } });
        const wire = f.sent.join('');
        expect(wire).not.toContain('NaN');
        expect(wire).not.toContain('tx_puresignal_auto_attenuate:');
        expect(wire).not.toMatch(/tx_two_tone(?:_|:)/);
        expect(wire).toContain(`vfo:0,0,${frequency};`);
        expect(wire).toContain(`tx_drive:0,${watts};`);
        expect(wire).not.toContain('trx:0,true');
        result[backend]!.push({ band, frequency, watts, mic, messages: [...f.sent] });
      }
    }
    const wireFixture = '# backend\tband\tfrequency\twatts\tmicDb\tmessage1\tmessage2\n' + Object.entries(result).flatMap(([backend, steps]) =>
      steps.map(step => [backend, step.band, step.frequency, step.watts, step.mic, ...step.messages].join('\t'))).join('\n') + '\n';
    const path = new URL('./fixtures/managed-band-recall.tsv', import.meta.url);
    if (process.env.SATURN_UPDATE_BAND_FIXTURE === '1') writeFileSync(path, wireFixture);
    expect(wireFixture).toBe(readFileSync(path, 'utf8'));
  });

  it('keeps Legacy setters and omits unadvertised shared attenuation without dropping ordinary TX settings', () => {
    const f = fixture();
    f.context.managedPcm.status = () => ({ manualAttenuationSupported: false, autoAdaptationSupported: true });
    f.api.sendCurrentRadioPrefsToBridge(undefined, { restoreTx: true });
    expect(f.sent.join('')).not.toContain('tx_puresignal_attenuation:');
    expect(f.sent.join('')).toContain('tx_drive:0,10;');
    f.context.managedPcmSelected = () => false;
    f.sent.length = 0; f.api.sendCurrentRadioPrefsToBridge(undefined, { restoreTx: true });
    expect(f.sent.join('')).toContain('tx_puresignal_auto_attenuate:0,true;');
    expect(f.sent.join('')).toContain('tx_two_tone_freq1:0,700;');
  });

  it('builds shifted filters, attenuation, squelch and gate commands without keying', () => {
    const prefs = normalizeRadioPrefs({ mode: 'USB', filterLow: 100, filterHigh: 3000,
      rxFilterShiftHz: 250, rxAttenuationDb: 12, rxSsqlEnabled: true, rxSsqlThreshold: 42,
      txNoiseGateEnabled: true, txNoiseGateThresholdDb: -35 });
    const commands = buildAllRadioPrefsCommands(prefs);
    expect(commands).toContain('rx_filter_band:0,350,3250;');
    expect(commands).toContain('rx_attenuation:0,12;'); expect(commands).toContain('rx_ssql:0,true;');
    expect(commands).toContain('rx_ssql_threshold:0,42;');
    expect(commands).toContain('tx_noise_gate:0,true;'); expect(commands).toContain('tx_noise_gate_threshold:0,-35.0;');
    expect(commands.some(s => s.startsWith('trx:'))).toBe(false);
  });
});
