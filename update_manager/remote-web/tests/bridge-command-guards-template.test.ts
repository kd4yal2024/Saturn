import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/p23test.html'),
  'utf8',
);
const begin = template.indexOf('/* BRIDGE_COMMAND_GUARDS_PRESENTATION_BEGIN */');
const end = template.indexOf('/* BRIDGE_COMMAND_GUARDS_PRESENTATION_END */');
const context = createContext({});
runInContext(template.slice(begin, end), context);

type Presentation = { state: string; summary: string; detail: string };
const present = context.bridgeCommandGuardsPresentation as (
  deltas: Record<string, number> | undefined,
  session: Record<string, unknown> | undefined,
) => Presentation;

const totals = {
  command_arm_cancelled: 4,
  command_mic_cancelled: 9,
  non_finite_controls_rejected: 2,
};

describe('Bridge command-guard telemetry presentation', () => {
  it('marks an older Bridge unavailable, never zero', () => {
    for (const session of [undefined, {}, { command_arm_cancelled: null }]) {
      const result = present({ arm: 0, mic: 0, nonFinite: 0 }, session);
      expect(result.state).toBe('unavailable');
      expect(result.summary).toBe('Not reported');
      expect(result.detail).toContain('not zero');
      expect(result.detail).not.toMatch(/Δ/);
    }
  });

  it('shows collecting until a delta between two samples of the same Bridge exists', () => {
    const result = present({ arm: NaN, mic: NaN, nonFinite: NaN }, totals);
    expect(result.state).toBe('collecting');
    expect(result.summary).toBe('Collecting…');
    expect(result.detail).toContain('Bridge-session totals 4 / 9 / 2');
    expect(result.detail).toContain('queued arms cancelled n/a');
  });

  it('reports a quiet interval without calling it an error', () => {
    const result = present({ arm: 0, mic: 0, nonFinite: 0 }, totals);
    expect(result.state).toBe('quiet');
    expect(result.summary).toBe('None this interval');
  });

  it('sums the interval and names each counter', () => {
    const result = present({ arm: 1, mic: 3, nonFinite: 2 }, totals);
    expect(result.state).toBe('activity');
    expect(result.summary).toBe('6 guard event(s) this interval');
    expect(result.detail).toContain('queued arms cancelled 1');
    expect(result.detail).toContain('mic frames cancelled 3');
    expect(result.detail).toContain('non-finite controls refused 2');
  });

  it('treats a negative delta as unavailable instead of showing it', () => {
    const result = present({ arm: -5, mic: 2, nonFinite: NaN }, totals);
    expect(result.summary).toBe('2 guard event(s) this interval');
    expect(result.detail).toContain('queued arms cancelled n/a');
    expect(result.detail).not.toContain('-5');
  });

  it('keeps a counter the Bridge does not report as n/a while showing the others', () => {
    const result = present(
      { arm: 0, mic: 0, nonFinite: 1 },
      { command_arm_cancelled: 4, command_mic_cancelled: 9 },
    );
    expect(result.summary).toBe('1 guard event(s) this interval');
    expect(result.detail).toContain('Bridge-session totals 4 / 9 / n/a');
  });
});

describe('Bridge command-guard wiring in the telemetry page', () => {
  it('reads the three counters as optional deltas for the direct-XDMA app only', () => {
    for (const key of [
      'command_arm_cancelled',
      'command_mic_cancelled',
      'non_finite_controls_rejected',
    ]) {
      expect(template).toContain(`optionalDeltaCounter('${key}')`);
    }
    expect(template).toContain("runtimeRow('Command guards'");
    // The row sits inside the direct-XDMA branch with the other integrity rows.
    const rowAt = template.indexOf("runtimeRow('Command guards'");
    const integrityAt = template.indexOf("runtimeRow('Direct-XDMA integrity'");
    expect(integrityAt).toBeGreaterThan(-1);
    expect(rowAt).toBeGreaterThan(integrityAt);
    expect(template.slice(integrityAt, rowAt)).toContain("d.selectedApp === 'xdma'");
  });

  it('keeps the guard counters informational: none feeds a health alert', () => {
    expect(template).not.toMatch(/alerts\.push\([^)]*(directArmCancelled|directMicCancelled|directNonFinite)/);
  });
});
