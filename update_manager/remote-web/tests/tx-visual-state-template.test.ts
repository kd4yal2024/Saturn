import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

describe('TX visual state wiring in updateTxZone()', () => {
  it('computes the visual state from the shared tx-presentation module, not a local copy', () => {
    expect(template).toContain('const visualState = _next.txVisualState(presentationState, state.txLockReason);');
    expect(template).not.toMatch(/function txVisualState\(/);
  });

  it('stamps the visual state onto #tx-zone alongside the existing code-state attributes', () => {
    const start = template.indexOf('function updateTxZone()');
    const end = template.indexOf('\n    }\n', template.indexOf('const arm = $("tx-arm-btn")', start));
    const body = template.slice(start, end);
    expect(body).toContain('txZone.dataset.txState = txState;');
    expect(body).toContain('txZone.dataset.txPresentation = presentationState;');
    expect(body).toContain('txZone.dataset.txVisualState = visualState;');
  });

  it('distinguishes the Locked and Disarmed banner text and hint by reason', () => {
    expect(template).toContain('let stateLabel = visualState === "disarmed" ? "TX DISARMED" : "TX LOCKED";');
    expect(template).toContain('? "Reconfirm to arm transmit before you can key up"');
  });

  it('guards every app-bar hook lookup so it is a safe no-op before the app-bar patch lands', () => {
    const start = template.indexOf('function updateTxAppBarHooks(visualState) {');
    const end = template.indexOf('\n    }\n', start);
    const body = template.slice(start, end);
    expect(body).toContain('const badge = $("app-tx-state-badge");');
    expect(body).toMatch(/if \(badge\)/);
    expect(body).toContain('const onAirBar = $("tx-on-air-bar");');
    expect(body).toMatch(/if \(onAirBar\)/);
    expect(body).toContain('const liveRegion = $("tx-state-live");');
    expect(body).toMatch(/if \(liveRegion/);
  });

  it('only writes the live region on a real transition, and only ever calls it assertive text, never "on air" for engaging', () => {
    const start = template.indexOf('function updateTxAppBarHooks(visualState) {');
    const end = template.indexOf('\n    }\n', start);
    const body = template.slice(start, end);
    expect(body).toContain('visualState !== lastAnnouncedTxVisualState');
    expect(body).toContain('lastAnnouncedTxVisualState = visualState;');
  });

  it('collapses engaging to the armed color on the badge attribute while keeping its own text', () => {
    expect(template).toContain('badge.dataset.txVisualState = visualState === "engaging" ? "armed" : visualState;');
  });

  it('calls updateTxAppBarHooks from inside updateTxZone on every render', () => {
    const start = template.indexOf('function updateTxZone()');
    const end = template.indexOf('let stateLabel', start);
    const body = template.slice(start, end);
    expect(body).toContain('updateTxAppBarHooks(visualState);');
  });
});

describe('TX keyed-state color no longer mixes caution into the on-air treatment', () => {
  it('fixes .tx-zone[data-tx-state="keyed"] to be danger-dominant, not caution-dominant', () => {
    const block = template.match(/\.tx-zone\[data-tx-state="keyed"\] \{[^}]*\}/)?.[0] ?? '';
    expect(block).toContain('var(--danger)');
    expect(block).not.toContain('var(--caution) 72%');
  });

  it('fixes the compact border-color override for keyed to danger', () => {
    expect(template).toContain('.tx-zone[data-tx-state="keyed"] { border-color: var(--danger); }');
    expect(template).not.toContain('.tx-zone[data-tx-state="keyed"] { border-color: var(--tx-orange); }');
  });

  it('fixes the top-meter instrument-state-line for the true transmitting state to danger', () => {
    const block = template.match(/\.top-meter-bank\[data-operating-state="tx"\] \.instrument-state-line \{[^}]*\}/)?.[0] ?? '';
    expect(block).toContain('var(--danger)');
    expect(block).not.toContain('var(--tx-orange)');
  });

  it('separates the keyed operator-detail tone from the merely-tx-selected tone', () => {
    expect(template).toContain('.operator-detail-row[data-tone="keyed"] .operator-detail-value {\n      color: var(--saturn-alarm);\n    }');
    expect(template).not.toMatch(/\.operator-detail-row\[data-tone="tx"\][^{]*\[data-tone="keyed"\]/);
  });
});
