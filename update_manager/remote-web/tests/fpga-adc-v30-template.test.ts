import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/p23test.html'),
  'utf8',
);
const begin = template.indexOf('/* FPGA_ADC_V30_PRESENTATION_BEGIN */');
const end = template.indexOf('/* FPGA_ADC_V30_PRESENTATION_END */');
const context = createContext({});
runInContext(template.slice(begin, end), context);
const present = context.fpgaAdcV30Presentation as (
  value: Record<string, unknown> | undefined,
  firmware: number,
) => { state: string; summary: string; detail: string };
const overview = context.adcTelemetryOverviewPresentation as (
  perf: Record<string, unknown>,
  status: Record<string, unknown>,
) => { source: string; summary: string; controlsEnabled: boolean; error: boolean };

describe('V30 FPGA ADC episode telemetry presentation', () => {
  it('handles absent fields compatibly', () => {
    expect(present(undefined, 29).state).toBe('unsupported');
    expect(present(undefined, 30).state).toBe('unavailable');
  });

  it('labels a marker mismatch before data-register use', () => {
    const result = present({
      available: false,
      status: 'marker_mismatch',
      build_id: 0x56323900,
    }, 30);
    expect(result.state).toBe('marker_mismatch');
    expect(result.detail).toContain('0x56323900');
    expect(result.detail).toContain('were not probed');
  });

  it('renders episode identity, clock-based duration, peak, and active state', () => {
    const result = present({
      available: true,
      status: 'available',
      snapshot_valid: true,
      snapshot_generation: 12,
      snapshot_retry_failure_count: 0,
      clock_hz: 122880000,
      adc1: {
        episode_count: 3,
        total_high_clocks: 123,
        longest_episode_clocks: 61,
        latest_episode_clocks: 20,
        latest_episode_peak: 32768,
        episode_active: false,
      },
      adc2: {
        episode_count: 1,
        total_high_clocks: 10,
        longest_episode_clocks: 10,
        latest_episode_clocks: 10,
        latest_episode_peak: 7000,
        episode_active: true,
      },
    }, 30);
    expect(result.state).toBe('available');
    expect(result.summary).toContain('snapshot generation 12');
    expect(result.detail).toContain('ADC1 episodes 3');
    expect(result.detail).toContain('ADC2 episodes 1');
    expect(result.detail).toContain('active');
    expect(result.detail).toContain('peak 32768');
  });

  it('includes the ADC object in captured telemetry JSON', () => {
    expect(template).toContain('fpga_adc_v30: jsonSafe(');
    expect(template).toContain('id="p23-fpga-adc-v30-summary"');
  });

  it('uses V30 episode telemetry for the Direct-XDMA ADC overview', () => {
    const result = overview({
      workload: { selected_app: 'xdma' },
      app_telemetry: {
        current: {
          app: 'saturn-bridge',
          fpga: { firmware_version: 30 },
          gauges: {
            fpga_adc_v30: {
              available: true,
              status: 'available',
              snapshot_valid: true,
              snapshot_generation: 99,
              snapshot_retry_failure_count: 0,
              clock_hz: 122880000,
              adc1: { episode_count: 3, total_high_clocks: 5, longest_episode_clocks: 2, latest_episode_clocks: 2, latest_episode_peak: 32768 },
              adc2: { episode_count: 0, total_high_clocks: 0, longest_episode_clocks: 0, latest_episode_clocks: 0, latest_episode_peak: 0 },
            },
          },
        },
      },
    }, {});
    expect(result.source).toBe('fpga_v30');
    expect(result.controlsEnabled).toBe(false);
    expect(result.error).toBe(false);
    expect(result.summary).toContain('generation 99');
    expect(result.summary).toContain('ADC1 episodes 3');
  });
});
