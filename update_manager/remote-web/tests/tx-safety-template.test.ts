import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

describe('TX safety control surface', () => {
  it('has one separate arm, momentary PTT, MOX, and lock control', () => {
    for (const id of ['tx-arm-btn', 'ptt-btn', 'mox-btn', 'tx-lock-btn']) {
      expect(template.match(new RegExp(`id="${id}"`, 'g'))).toHaveLength(1);
    }
    expect(template).toContain('id="ptt-btn" type="button" aria-describedby="tx-zone-hint" aria-pressed="false" disabled');
    expect(template).toContain('id="mox-btn" type="button" aria-describedby="tx-zone-hint" aria-pressed="false" disabled');
  });

  it('arms through the existing readiness owner without keying', () => {
    const start = template.indexOf('const armTxFromButton = (event) => {');
    const end = template.indexOf('const startPointerPtt = (event) => {', start);
    const handler = template.slice(start, end);
    expect(handler).toContain('armTxReady("operator-arm")');
    expect(handler).not.toContain('setPtt(true)');
    expect(handler).not.toContain('sendTci(');
  });

  it('presents a TX fault distinctly and requires deliberate re-arming', () => {
    expect(template).toContain('faulted: state.txLockReason === "tx-fault"');
    expect(template).toContain('stateLabel = "TX FAULT"');
    expect(template).toContain('arm.textContent = "VERIFY & RE-ARM"');
    expect(template).toContain('.tx-zone[data-tx-state="fault"]');
  });

  it('keeps momentary PTT release guards intact', () => {
    expect(template).toContain('pttButton.addEventListener("pointerup", stopPointerPtt)');
    expect(template).toContain('pttButton.addEventListener("pointercancel", stopPointerPtt)');
    expect(template).toContain('pttButton.addEventListener("lostpointercapture"');
    expect(template).toContain('void setPtt(false, { lockAfter: "pointer-blur" })');
    expect(template).toContain('void setPtt(false, { lockAfter: "page-hidden" })');
  });
});
