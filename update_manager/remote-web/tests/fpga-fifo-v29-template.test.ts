import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/p23test.html'),
  'utf8',
);
const begin = template.indexOf('/* FPGA_FIFO_V29_PRESENTATION_BEGIN */');
const end = template.indexOf('/* FPGA_FIFO_V29_PRESENTATION_END */');
const context = createContext({});
runInContext(template.slice(begin, end), context);
const present = context.fpgaFifoV29Presentation as (
  value: Record<string, unknown> | undefined,
  firmware: number,
) => { state: string; summary: string; detail: string };

describe('V29 FPGA FIFO telemetry presentation', () => {
  it('handles absent fields compatibly', () => {
    expect(present(undefined, 28).state).toBe('unsupported');
    expect(present(undefined, 29).state).toBe('unavailable');
  });

  it('labels a marker mismatch without implying that data registers were read', () => {
    const result = present({
      available: false,
      status: 'marker_mismatch',
      build_id: 0x56323800,
    }, 29);
    expect(result.state).toBe('marker_mismatch');
    expect(result.detail).toContain('0x56323800');
    expect(result.detail).toContain('were not probed');
  });

  it('shows raw word locations and transition counters', () => {
    const result = present({
      available: true,
      status: 'available',
      snapshot_valid: true,
      snapshot_generation: 7,
      snapshot_timeout_count: 2,
      occupancy_words: { ddc: 1, duc: 2, mic: 3, speaker: 4 },
      minimum_words: { ddc: 0, duc: 0, mic: 0, speaker: 0 },
      maximum_words: { ddc: 11, duc: 12, mic: 13, speaker: 14 },
      event_transitions: { ddc: 21, duc: 22, mic: 23, speaker: 24 },
    }, 29);
    expect(result.state).toBe('available');
    expect(result.detail).toContain('occupancy words: ddc 1');
    expect(result.detail).toContain('event transitions: ddc 21');
  });

  it('includes the FIFO object in captured telemetry JSON', () => {
    expect(template).toContain('fpga_fifo_v29: jsonSafe(');
    expect(template).toContain('id="p23-fpga-fifo-v29-summary"');
  });
});
