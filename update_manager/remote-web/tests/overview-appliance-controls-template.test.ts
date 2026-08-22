import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const overviewPath = fileURLToPath(
  new URL('../../templates/overview.html', import.meta.url),
);
const overview = readFileSync(overviewPath, 'utf8');

describe('Saturn Go overview appliance controls', () => {
  it('exposes separate guarded P2 and XDMA start/stop controls', () => {
    expect(overview).toContain('id="radio-p2-start"');
    expect(overview).toContain('id="radio-p2-stop"');
    expect(overview).toContain('id="radio-xdma-start"');
    expect(overview).toContain('id="radio-xdma-stop"');
    expect(overview).toContain("postJson('./radio_backend', { action, backend })");
  });

  it('requires explicit POWER OFF confirmation for G2 shutdown', () => {
    expect(overview).toContain('id="g2-poweroff-confirm"');
    expect(overview).toContain("confirmation !== 'POWER OFF'");
    expect(overview).toContain("postJson('./appliance_power', { action:'poweroff', confirmation })");
  });

  it('renders selected backend and operational status independently of p2app', () => {
    expect(overview).toContain("fetchJson('./radio_backend')");
    expect(overview).toContain("radioBackend?.operational_status");
    expect(overview).toContain("setService('xdma'");
  });
});
