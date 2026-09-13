import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/p23test.html'),
  'utf8',
);
const begin = template.indexOf('/* SPEAKER_PACING_PRESENTATION_BEGIN */');
const end = template.indexOf('/* SPEAKER_PACING_PRESENTATION_END */');
const context = createContext({});
runInContext(template.slice(begin, end), context);
const present = context.speakerPacingPresentation as (
  value: Record<string, unknown> | undefined,
) => { state: string; summary: string; detail: string };

describe('speaker pacing diagnostics presentation', () => {
  it('handles telemetry from an older P2 build compatibly', () => {
    const result = present(undefined);
    expect(result.state).toBe('unavailable');
    expect(result.detail).toContain('P2 build');
  });

  it('labels lifetime maxima, threshold counters, thread identity, and incident context', () => {
    const result = present({
      available: true,
      lifetime_scope: 'process',
      duration_unit: 'microseconds',
      thread: { tid: 777, scheduling_policy: 'other', priority: 0, cpu: 2 },
      maximum: {
        loop_gap_us: 17500,
        recvmmsg_duration_us: 9500,
        dma_write_duration_us: 9000,
        software_queue_depth_frames: 11,
      },
      loop_gap_counts: { over_2ms: 4, over_4ms: 3, over_8ms: 2, over_16ms: 1 },
      recvmmsg_duration_counts: { over_1ms: 5, over_2ms: 4, over_4ms: 3, over_8ms: 2, over_16ms: 0 },
      dma_write_duration_counts: { over_1ms: 6, over_2ms: 5, over_4ms: 4, over_8ms: 3, over_16ms: 0 },
      last_underrun: {
        valid: true,
        monotonic_timestamp_ns: 9876543210,
        loop_gap_us: 17500,
        recvmmsg_duration_us: 9500,
        dma_write_duration_us: 9000,
        fifo_frames_before_refill: 0,
        queued_frames: 7,
        frames_selected: 6,
        frames_written: 6,
        queue_age_us: 2400,
        thread_cpu: 3,
      },
    });

    expect(result.state).toBe('available');
    expect(result.summary).toContain('process lifetime');
    expect(result.summary).toContain('max loop 17500 µs');
    expect(result.summary).toContain('queue 11 frames');
    expect(result.detail).toContain('TID 777');
    expect(result.detail).toContain('process-lifetime loop counts >2ms 4');
    expect(result.detail).toContain('receive counts >1ms 5');
    expect(result.detail).toContain('DMA counts >1ms 6');
    expect(result.detail).toContain('selected/written 6/6');
  });

  it('includes compatible display and export hooks', () => {
    expect(template).toContain('id="p23-speaker-pacing-summary"');
    expect(template).toContain('speaker_pacing_diagnostics: jsonSafe(');
    expect(template).toContain('latest_speaker_pacing_diagnostics: jsonSafe(');
    expect(template).toContain("runtimeRow('Speaker pacing'");
  });
});
