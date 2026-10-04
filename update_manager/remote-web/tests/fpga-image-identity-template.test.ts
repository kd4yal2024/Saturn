import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(resolve(process.cwd(), '../templates/p23test.html'), 'utf8');
const begin = template.indexOf('/* FPGA_IMAGE_IDENTITY_PRESENTATION_BEGIN */');
const end = template.indexOf('/* FPGA_IMAGE_IDENTITY_PRESENTATION_END */');
const context = createContext({});
runInContext(template.slice(begin, end), context);
const present = context.fpgaImageIdentityPresentation as (
  fpga: Record<string, unknown> | undefined,
) => { summary: string; detail: string };

describe('radio telemetry FPGA image identity', () => {
  it('shows the verified 1.30.002 baseline without claiming saturation', () => {
    const result = present({
      available: true,
      build_identity_status: 'identified',
      build_id_hex: '0x53460002',
      firmware_display: '1.30.002',
      rx_filter: '22/Q24',
    });
    expect(result.summary).toBe('1.30.002 · 22/Q24');
    expect(result.detail).toContain('USR_ACCESS 0x53460002');
    expect(result.detail).not.toContain('saturating');
  });

  it('shows an unknown 1.31 image without inventing an image match', () => {
    const result = present({
      available: true,
      build_identity_status: 'unidentified',
      build_id_hex: '0x53460004',
      firmware_major_version: 1,
      firmware_version: 31,
      firmware_display: '1.31 — build unidentified',
      rx_filter: null,
    });
    expect(result.summary).toContain('build unidentified');
    expect(result.detail).toContain('no exact host-manifest match');
    expect(result.detail).not.toContain('22/Q24');
  });

  it('only describes saturation for an identified 1.31.001 entry', () => {
    const result = present({
      available: true,
      build_identity_status: 'identified',
      build_id_hex: '0x53460003',
      firmware_display: '1.31.001',
      rx_filter: '22/Q24 saturated',
    });
    expect(result.summary).toBe('1.31.001 · 22/Q24 saturated');
    expect(result.detail).toContain('no clamp-event counter is exported');
  });

  it('does not identify stale or unavailable telemetry', () => {
    expect(present(undefined).summary).toBe('unavailable');
    const stale = present({
      available: true,
      build_identity_status: 'stale',
      build_id_hex: '0x53460002',
    });
    expect(stale.summary).toBe('stale');
    expect(stale.detail).not.toContain('manifest match');
  });

  it('includes identity in both captured snapshot forms and the live dashboard', () => {
    expect(template).toContain('fpga_image_identity: jsonSafe(');
    expect(template).toContain('latest_fpga_image_identity: jsonSafe(');
    expect(template).toContain("runtimeRow('FPGA image identity', fpgaIdentity.summary, fpgaIdentity.detail)");
  });
});
