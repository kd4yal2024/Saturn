import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(new URL('../../templates/p23test.html', import.meta.url), 'utf8');
const BEGIN = '/* PER_DDC_ALMOST_FULL_PRESENTATION_BEGIN */';
const END = '/* PER_DDC_ALMOST_FULL_PRESENTATION_END */';
function extractPresentation(source: string): string {
  const begin = source.indexOf(BEGIN);
  const end = source.indexOf(END);
  if (begin < 0 || end <= begin || source.indexOf(BEGIN, begin + BEGIN.length) >= 0
    || source.indexOf(END, end + END.length) >= 0) {
    throw new Error('Per-DDC presentation source markers missing, duplicated or reordered');
  }
  return source.slice(begin + BEGIN.length, end);
}
const context = createContext({});
runInContext(extractPresentation(template), context);
if (typeof context.perDdcAlmostFullPresentation !== 'function') {
  throw new Error('Per-DDC presentation function missing from marked source');
}
const present = context.perDdcAlmostFullPresentation as (
  value: Record<string, unknown> | undefined,
  monitor: Record<string, unknown> | undefined,
) => { state: string; summary: string; detail: string };

describe('direct-XDMA per-DDC almost-full presentation', () => {
  it('refuses missing, duplicated or reordered source markers', () => {
    for (const source of [
      template.replace(BEGIN, ''), template.replace(END, ''),
      `${END}${BEGIN}`, `${BEGIN}${template}`, `${template}${END}`,
    ]) {
      expect(() => extractPresentation(source)).toThrow(/source markers/);
    }
  });

  it('preserves a successfully observed zero separately from missing data', () => {
    const result = present({
      status: 'ok', last_mask: 0, successful_reads: 1, read_failures: 0,
      observations: { ddc0: 0, ddc6: 0 },
      startup: { status: 'ok', mask: 0, after_fifo_reset_us: 0, after_enable_us: 0 },
    }, { threshold_words: 0, seen_empty_observations: 0,
      startup_seen_empty_observations: 0, poll_high_water_words: 0 });
    expect(result.state).toBe('ok');
    expect(result.summary).toContain('last window 0x000');
    expect(result.detail).toContain('startup/reset mask 0x000');
    expect(result.detail).toContain('DDC0 0');
    expect(result.detail).toContain('DDC1 n/a');
    expect(result.detail).toContain('DDC6 0');
    expect(result.detail).toContain('threshold 0 words');
    expect(result.detail).toContain('Bridge-session polled HWM 0 words');
  });

  it('does not turn missing or failed reads into zero activity', () => {
    expect(present(undefined, undefined).state).toBe('unavailable');
    const result = present({
      status: 'read_error',
      last_mask: null,
      successful_reads: 3,
      read_failures: 1,
      observations: { ddc6: 2 },
      startup: { status: 'read_error', mask: null },
    }, { threshold_words: null });
    expect(result.summary).toContain('latest read failed; last mask unknown');
    expect(result.detail).toContain('startup/reset mask unavailable');
    expect(result.detail).toContain('DDC0 n/a');
    expect(result.detail).toContain('DDC6 2');
    expect(result.detail).toContain('threshold n/a');
  });

  it('separates startup evidence and session HWM from steady read windows', () => {
    const result = present({
      status: 'ok', last_mask: 0, successful_reads: 4, read_failures: 0,
      observations: { ddc0: 0, ddc6: 1 },
      startup: { status: 'ok', mask: 0x3ff, after_enable_us: 1200 },
    }, {
      threshold_words: 16384, seen_empty_observations: 3,
      startup_seen_empty_observations: 1, poll_high_water_words: 754,
    });
    expect(result.summary).toContain('last window 0x000');
    expect(result.detail).toContain('startup/reset mask 0x3FF');
    expect(result.detail).toContain('ambiguous, excluded from steady counts');
    expect(result.detail).toContain('Bridge-session polled HWM 754 words');
    expect(result.detail).toContain('V29 extrema are FPGA boot-lifetime');
    expect(result.detail).toContain('bit 29 is not a DMA underflow');
  });

  it('includes both provenance objects in exported snapshots', () => {
    expect(template).toContain('per_ddc_almost_full: jsonSafe(');
    expect(template).toContain('direct_xdma_fifo_monitor: jsonSafe(');
    expect(template).toContain('id="p23-per-ddc-almost-full-summary"');
  });
});
