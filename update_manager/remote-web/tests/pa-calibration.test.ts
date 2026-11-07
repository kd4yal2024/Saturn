import { describe, expect, it } from 'vitest';
import { G2_PA_STARTING_GAINS, normalizePaCalibration, paCalibrationCommand, parsePaCalibration } from '../src/settings/pa-calibration';
import { normalizeBandMemory, normalizeRadioPrefs, normalizeRemoteSettings } from '../src/settings/normalize';
import { createAppState } from '../src/state/app-state';
import { applyRadioPrefsToState } from '../src/state/apply-prefs';
import { radioPrefsFromState } from '../src/state/prefs-from-state';
import { applyTciText } from '../src/tci/apply';
import type { TciRadioState } from '../src/tci/state';
import { managedSettingsMessage } from '../src/transport/managed-pcm';

describe('PA calibration equipment profile', () => {
  it('keeps legacy compatibility and rejects incomplete/nonfinite/out of range profiles', () => {
    expect(normalizeRadioPrefs({}).txPaCalibration).toBeNull();
    for (const bad of [[], [46.5], Array(12).fill(NaN), Array(12).fill(39), Array(12).fill(81), Array(12).fill('50')])
      expect(normalizePaCalibration(bad)).toBeNull();
    expect(parsePaCalibration('')).toBeUndefined();
    expect(parsePaCalibration('legacy')).toBeNull();
  });
  it('preserves every band, serializes one atomic managed settings transaction and round-trips saved state', () => {
    const gains: number[] = [...G2_PA_STARTING_GAINS]; gains[9] = 48.5;
    const state = createAppState(); applyRadioPrefsToState({ txPaCalibration: gains }, state);
    const prefs = radioPrefsFromState(state);
    expect(prefs.txPaCalibration).toEqual(gains);
    const wire = paCalibrationCommand(gains);
    expect(wire.split(';').filter(Boolean)).toHaveLength(1);
    expect(managedSettingsMessage(wire)).toBe(true);
    expect(parsePaCalibration(wire.slice('tx_pa_calibration:0,'.length, -1))).toEqual(gains);
    const saved = normalizeRemoteSettings({ radioPrefs: prefs }, { protocol:'https:', host:'g2:8443', hostname:'g2', href:'https://g2:8443/remote-next' });
    expect(saved.radioPrefs.txPaCalibration).toEqual(gains);
    expect(paCalibrationCommand(null)).toBe('tx_pa_calibration:0,legacy;');
  });
  it('does not let older band-memory snapshots overwrite the equipment calibration', () => {
    const memories = normalizeBandMemory({ '40m': { frequency:7_200_000, radioPrefs:{txPaCalibration:[...G2_PA_STARTING_GAINS],txDrive:40} } });
    expect(memories['40m']?.radioPrefs.txPaCalibration).toBeUndefined();
    const state = createAppState(); state.txPaCalibration = [...G2_PA_STARTING_GAINS]; state.txPaCalibration[9] = 49;
    applyRadioPrefsToState(memories['40m']!.radioPrefs, state);
    expect(state.txPaCalibration[9]).toBe(49);
  });
  it('keeps host-confirmed calibration separate from the requested profile and labels drive as a command', () => {
    let state = createAppState() as unknown as TciRadioState & { txPaCalibration: number[] | null }; state.txPaCalibration = [...G2_PA_STARTING_GAINS];
    state = applyTciText('tx_pa_calibration_supported:0,true;tx_pa_calibration:0,legacy;tx_pa_drive:0,27426000,40,68,11m,46.5;', state).state as typeof state;
    expect(state.txPaCalibrationSupported).toBe(true); expect(state.txPaCalibrationHost).toBeNull();
    expect(state.txPaCalibration).toEqual(G2_PA_STARTING_GAINS);
    expect(state.txPaDriveReport).toContain('host drive command 68/255');
    expect(state.txPaDriveReport).toContain('RF unmeasured');
    state = applyTciText(`tx_pa_calibration:0,${G2_PA_STARTING_GAINS.join('|')};`, state).state as typeof state;
    expect(state.txPaCalibrationHost).toEqual(G2_PA_STARTING_GAINS);
    expect(applyTciText('tx_pa_calibration:0,NaN;', state).state.txPaCalibrationHost).toEqual(G2_PA_STARTING_GAINS);
  });
});
