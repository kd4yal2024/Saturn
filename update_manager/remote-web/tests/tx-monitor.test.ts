import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { createAppState } from '../src/state/app-state';

const html = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');

describe('G2 TX headphone MON', () => {
  it('starts disabled at a low level and requires a capability response', () => {
    const state = createAppState();
    expect(state.txMonitorEnabled).toBe(false);
    expect(state.txMonitorSupported).toBe(false);
    expect(state.txMonitorLevelDb).toBe(-30);
  });
  it('has an accessible MON toggle and independent bounded level', () => {
    expect(html).toMatch(/id="tx-mon-btn"[^>]*aria-pressed="false"[^>]*disabled/);
    expect(html).toContain('id="tx-mon-level" type="range" min="-60" max="-6"');
    expect(html).toContain('G2 headphones');
  });
  it('MON handlers only send monitor commands, never key TX or change mic gain', () => {
    const handlers = html.slice(html.indexOf('$("tx-mon-btn").addEventListener'), html.indexOf('$("tx-drive").addEventListener'));
    expect(handlers).toContain('state.remoteClientRole !== "operator"');
    expect(handlers).toContain('tx_monitor:0,');
    expect(handlers).toContain('tx_monitor_level:0,');
    expect(handlers).not.toMatch(/trx:|tx_drive:|tx_mic_gain:|localStorage|setItem|startMic/);
  });
});

describe('G2 rear RX headphones', () => {
  it('starts off and exposes an accessible waterfall checkbox', () => {
    const state = createAppState();
    expect(state.rxHeadphonesSupported).toBe(false);
    expect(state.rxHeadphonesEnabled).toBe(false);
    expect(html).toMatch(/id="waterfall-shell"[^>]*>[\s\S]*?id="rx-headphones-checkbox" type="checkbox" disabled aria-label="G2 rear headphone RX audio"/);
  });
  it('requires operator acknowledgement and does not key TX', () => {
    const handlers = html.slice(html.indexOf('$("rx-headphones-checkbox").addEventListener("change"'), html.indexOf('$("tx-mon-level").addEventListener("input"'));
    expect(handlers).toContain('state.remoteClientRole !== "operator"');
    expect(handlers).toContain('rx_headphones:0,');
    expect(handlers).not.toMatch(/trx:|tx_drive:|localStorage|setItem|startMic/);
  });
});
