import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/p23test.html'),
  'utf8',
);
const begin = template.indexOf('/* DIRECT_XDMA_DUC_PRESENTATION_BEGIN */');
const end = template.indexOf('/* DIRECT_XDMA_DUC_PRESENTATION_END */');
const context = createContext({});
runInContext(template.slice(begin, end), context);
const present = context.directXdmaDucPresentation as (
  value: Record<string, unknown> | undefined,
) => { state: string; summary: string; detail: string };

describe('Direct-XDMA DUC telemetry presentation', () => {
  it('labels missing bridge telemetry as unavailable', () => {
    const result = present(undefined);
    expect(result.state).toBe('unavailable');
    expect(result.detail).toContain('V29 FIFO monitor');
  });

  it('renders coherent FPGA occupancy and real bridge TX counters', () => {
    const result = present({
      available: true,
      snapshot_valid: true,
      snapshot_generation: 101,
      fifo_occupancy_words: 77,
      fifo_minimum_words: 0,
      fifo_maximum_words: 992,
      fifo_event_transitions: 5,
      stream_active: false,
      keyed: false,
      dma_writes: 8,
      frames_written: 16,
      tx_fifo_lwm: 120,
      tx_fifo_hwm: 900,
      fifo_faults: 0,
      startup_underflows: 0,
    });
    expect(result.state).toBe('available');
    expect(result.summary).toContain('77 FPGA FIFO words');
    expect(result.summary).toContain('RX idle');
    expect(result.detail).toContain('generation 101');
    expect(result.detail).toContain('boot min/max 0/992');
    expect(result.detail).toContain('TX writes/frames 8/16');
    expect(result.detail).toContain('FIFO low/high 120/900');
    expect(result.detail).toContain('host queue depth/age/mode not instrumented');
  });

  it('includes the Direct-XDMA DUC object in captures and baseline exports', () => {
    expect(template).toContain('direct_xdma_duc: jsonSafe(');
    expect(template).toContain('latest_direct_xdma_duc: jsonSafe(');
  });
});
