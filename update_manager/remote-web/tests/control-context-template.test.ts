import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/saturn-remote-next.html'),
  'utf8',
);

describe('responsive radio control context', () => {
  it('assigns existing controls to RX and DSP without duplicating them', () => {
    expect(template.match(/data-control-context="rx"/g)).toHaveLength(3);
    expect(template.match(/data-control-context="dsp"/g)).toHaveLength(1);
    expect(template.match(/id="ptt-btn"/g)).toHaveLength(1);
    expect(template.match(/id="mox-btn"/g)).toHaveLength(1);
  });

  it('keeps the authoritative TX zone in the rail when RX or DSP is selected', () => {
    expect(template).toContain('.context-rail:not([data-active-context="tx"]) .tx-zone');
    expect(template).toContain('rail.appendChild(rightRail)');
    expect(template).toContain('applyControlContext(controlContext)');
  });

  it('persists a separate UI context preference', () => {
    expect(template).toContain('saturn.remote.controlContext');
    expect(template).toContain('normalizeRadioControlContext(value)');
  });
});
