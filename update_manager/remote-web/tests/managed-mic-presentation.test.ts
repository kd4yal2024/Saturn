import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';
import { TxPowerDraft } from '../src/settings/tx-power';

const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
const start = template.indexOf('      changed: status => {', template.indexOf('    const managedPcm ='));
const end = template.indexOf('\n      },\n    });', start);
if (start < 0 || end < start) throw Error('Managed PCM status callback missing');
const callback = template.slice(start, end).replace('      changed:', '');

function fixture() {
  const state = { micCapturing: false, txMicLevel: 0.3, txMicLevelVisual: 0.3,
    txMicLevelAt: 100, moxRequested: false, psMeasurementOwned: false };
  const changed = runInNewContext(`(${callback}\n})`, {
    state, txPowerDraft: new TxPowerDraft(), $: () => null, scheduleUiRefresh() {}, setPtt() {},
    setTxMediaPriority() {}, recoverRxAfterMox() {},
  }) as (status: Record<string, unknown>) => void;
  return { state, changed };
}

describe('managed microphone meter presentation', () => {
  it('shows the sole managed voice capture and clears the meter after release', () => {
    const f = fixture();
    f.changed({ selected: true, phase: 'streaming', source: 'tci' });
    expect(f.state.micCapturing).toBe(true);
    expect(f.state.txMicLevel).toBe(0.3);
    f.changed({ selected: true, phase: 'releasing', source: 'tci' });
    expect(f.state.micCapturing).toBe(false);
    expect(f.state.txMicLevel).toBeNull();
    expect(f.state.txMicLevelVisual).toBe(0);
    expect(f.state.txMicLevelAt).toBe(0);
  });
  it('keeps SATP, generated tests, pending capture and Legacy distinct', () => {
    const f = fixture();
    for (const status of [
      { phase: 'streaming', source: 'satp' },
      { phase: 'streaming', source: 'tci', measurementActive: true },
      { phase: 'streaming', source: 'tci', twoToneActive: true },
      { phase: 'arming', source: 'tci' },
      { phase: 'fault', source: 'tci' },
      { phase: 'disconnected', source: 'tci' },
    ]) {
      f.state.micCapturing = true;
      f.changed({ selected: true, ...status });
      expect(f.state.micCapturing).toBe(false);
    }
    f.state.micCapturing = true;
    f.changed({ selected: false, phase: 'disabled', source: 'tci' });
    expect(f.state.micCapturing).toBe(true);
  });
});
